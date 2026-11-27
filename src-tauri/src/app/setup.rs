use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use tauri::{App, AppHandle, Manager};

use crate::app::window_manager::{maybe_open_devtools, toggle_window};
use crate::app_state::{AppDataDir, EncryptionQueueState, PasteQueue, SessionHistory, SettingsState};
use crate::database::{self, DbState};
use crate::global_state::*;
use crate::info;
use crate::infrastructure::macos_api::window::{self as panel, WindowPinMode, PIN_MODE_KEY};
use crate::infrastructure::repository::clipboard_repo::SqliteClipboardRepository;
use crate::infrastructure::repository::settings_repo::{
    SettingsRepository, SqliteSettingsRepository,
};
use crate::infrastructure::repository::tag_repo::SqliteTagRepository;
use crate::services::encryption_queue::init_encryption_queue;
use crate::services::sensitive_align::spawn_sensitive_alignment;

#[cfg(target_os = "macos")]
fn register_macos_wake_handler(app_handle: AppHandle) {
    use block2::RcBlock;
    use objc2_app_kit::{NSWorkspace, NSWorkspaceDidWakeNotification};
    use objc2_foundation::NSNotification;
    use std::ptr::NonNull;
    use std::time::Duration;

    let center = NSWorkspace::sharedWorkspace().notificationCenter();
    let wake_handler = RcBlock::new(move |_notification: NonNull<NSNotification>| {
        crate::services::recording_pause::check_expired_pause(&app_handle);
        tauri::async_runtime::spawn(crate::services::paste_queue::check_idle_after_wake(app_handle.clone()));
        let app_handle = app_handle.clone();
        std::thread::spawn(move || {
            // macOS may need a short moment to restore Carbon event hotkeys after wake.
            std::thread::sleep(Duration::from_millis(750));
            let main_thread_handle = app_handle.clone();
            let _ = app_handle.run_on_main_thread(move || {
                match crate::app::commands::sync_registered_hotkeys(&main_thread_handle) {
                    Ok(()) => info!(">>> [HOTKEY] Re-registered shortcuts after macOS wake"),
                    Err(error) => {
                        crate::error!(">>> [HOTKEY] Wake re-registration failed: {error}")
                    }
                }
            });
        });
    });

    // NSNotificationCenter owns the returned observer for the process lifetime.
    unsafe {
        let observer = center.addObserverForName_object_queue_usingBlock(
            Some(NSWorkspaceDidWakeNotification),
            None,
            None,
            &wake_handler,
        );
        let _ = objc2::rc::Retained::into_raw(observer);
    }
}

/// A pinned panel never becomes key on click (so it doesn't steal typing from
/// the app being pasted into), which means the page often can't see Esc. This
/// watches key presses meant for other apps and closes the pinned window on
/// Esc. When the panel is key, the page's own Esc handler does the same.
#[cfg(target_os = "macos")]
fn register_macos_pinned_escape_monitor(app_handle: AppHandle) {
    use block2::RcBlock;
    use objc2_app_kit::{NSEvent, NSEventMask};
    use std::ptr::NonNull;

    const KEY_ESCAPE: u16 = 53;
    let handler = RcBlock::new(move |event: NonNull<NSEvent>| {
        let key = unsafe { event.as_ref().keyCode() };
        if key == KEY_ESCAPE
            && panel::is_clipboard_panel_visible()
            && panel::responds_to_global_escape(&app_handle)
        {
            let _ = crate::app::window_manager::escape_window_cmd(app_handle.clone());
        }
    });
    // Lives as long as the process; the monitor object is deliberately leaked.
    if let Some(monitor) = NSEvent::addGlobalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &handler) {
        let _ = objc2::rc::Retained::into_raw(monitor);
    }
}

pub fn init(app: &mut App) -> Result<(), Box<dyn std::error::Error>> {
    let app_handle = app.handle().clone();

    #[cfg(target_os = "macos")]
    register_macos_wake_handler(app_handle.clone());
    #[cfg(target_os = "macos")]
    register_macos_pinned_escape_monitor(app_handle.clone());
    #[cfg(target_os = "macos")]
    panel::start_window_tracking();

    // Initialize GLOBAL_APP_HANDLE
    let _ = GLOBAL_APP_HANDLE.set(app_handle.clone());

    // 1. Data Directory
    let mut app_dir = resolve_data_dir(app)?;

    // 2. Logger Initialization
    crate::logger::init(app_dir.join("dewclip.log"));
    info!(">>> [STARTUP] DewClip starting up...");

    // 2b. Local-at-rest encryption (AES-GCM + OS key store)
    crate::infrastructure::encryption::init(&app_dir);

    if portable_data_dir().is_none() {
        let default = default_app_data_dir(app.path())?;
        match crate::app::data_directory::apply_requested_move(&default, &app_dir) {
            Ok(Some(path)) => {
                app_dir = path;
                crate::infrastructure::encryption::init(&app_dir);
                crate::logger::init(app_dir.join("dewclip.log"));
            }
            Ok(None) => {}
            Err(error) => {
                use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
                crate::error!(">>> [STARTUP] {error}");
                app.dialog().message(error).kind(MessageDialogKind::Error).show(|_| {});
            }
        }
    }

    // 3. Database Initialization
    let db_path = app_dir.join("clipboard.db");
    let db_path_str = db_path.to_string_lossy();
    let conn = database::init_db(&db_path_str).map_err(|e| {
        let err_msg = format!("无法打开记录存储。\n详情：{}", e);
        eprintln!("DewClip Startup Error: {}", err_msg);
        e
    })?;

    #[cfg(not(feature = "portable"))]
    {
        match crate::infrastructure::encryption::migrate_legacy_ciphertexts(&conn) {
            Ok(n) if n > 0 => info!(">>> [STARTUP] Migrated {} legacy cipher fields", n),
            Ok(_) => {}
            Err(e) => eprintln!(">>> [STARTUP] Legacy encryption migration warning: {}", e),
        }
    }

    let conn_arc = std::sync::Arc::new(std::sync::Mutex::new(conn));
    let settings_repo = SqliteSettingsRepository::new(conn_arc.clone());

    // 4. Avoid stale persisted pin state causing unexpected always-on-top behavior.
    let _ = settings_repo.set("app.window_pinned", "false");

    let settings = load_settings(&settings_repo)?;

    // 5. App State Management
    crate::app::commands::thumbnail_cmd::init_thumbnail_cache(app.handle());
    crate::app::data_directory::prepare_session(&app_dir)?;
    setup_state(app, conn_arc.clone(), &settings, app_dir.clone());
    app.manage(crate::services::recording_pause::RecordingPause::default());
    crate::services::clipboard::start_session_file_sweeper(app.handle().clone());
    app.manage(EncryptionQueueState(init_encryption_queue(app_handle.clone())));
    spawn_sensitive_alignment(app_handle.clone());

    // 6. App Visibility
    apply_initial_dock_visibility(app, &settings);

    // 7. Window Initialization (Pinned/Focus)
    setup_main_window(app, &settings);
    crate::app::capsule::create_window(&app_handle);

    // 8. Background Services & Monitors
    start_services(app, &settings, app_handle.clone());

    // 9. Tray Setup
    crate::app::tray::setup_tray(app, settings.hide_tray_icon)?;

    Ok(())
}

pub(crate) fn default_app_data_dir<R: tauri::Runtime>(
    paths: &tauri::path::PathResolver<R>,
) -> tauri::Result<std::path::PathBuf> {
    Ok(isolate_debug_data_dir(
        paths.app_data_dir()?,
        cfg!(all(target_os = "macos", debug_assertions)),
    ))
}

// macOS 调试构建以本地文件保存主密钥，必须与正式版使用不同的数据目录。
fn isolate_debug_data_dir(dir: std::path::PathBuf, macos_debug: bool) -> std::path::PathBuf {
    if !macos_debug {
        return dir;
    }
    match dir.file_name().and_then(|name| name.to_str()) {
        Some(name) if name.ends_with(".dev") => dir,
        name => dir.with_file_name(format!("{}.dev", name.unwrap_or("io.github.rexkev214.dewclip"))),
    }
}

fn resolve_data_dir(app: &App) -> Result<std::path::PathBuf, Box<dyn std::error::Error>> {
    if let Some(portable_data) = portable_data_dir() {
        return Ok(portable_data);
    }
    let app_dir = crate::app::data_directory::startup_directory(&default_app_data_dir(app.path())?)
        .map_err(std::io::Error::other)?;

    // Cleanup temp files
    std::thread::spawn(|| {
        let temp_dir = std::env::temp_dir();
        if let Ok(entries) = std::fs::read_dir(&temp_dir) {
            for entry in entries.flatten() {
                if let Ok(name) = entry.file_name().into_string() {
                    // Remove clipboard temp files left by earlier runs.
                    if ["DewClip_Clip_", "DewClip_IMG_", "DewClip_GIF_"]
                        .iter()
                        .any(|prefix| name.starts_with(prefix))
                    {
                        let _ = std::fs::remove_file(entry.path());
                    }
                }
            }
        }
    });

    Ok(app_dir)
}

#[cfg(test)]
mod data_dir_tests {
    use super::isolate_debug_data_dir;

    #[test]
    fn debug_and_release_default_data_directories_are_separate() {
        let base = std::env::temp_dir();
        let release = base.join("io.github.rexkev214.dewclip");
        let dev = base.join("io.github.rexkev214.dewclip.dev");
        assert_eq!(isolate_debug_data_dir(release.clone(), false), release);
        assert_eq!(isolate_debug_data_dir(release, true), dev);
        assert_eq!(isolate_debug_data_dir(dev.clone(), true), dev);
        assert_eq!(
            isolate_debug_data_dir(std::path::PathBuf::new(), true),
            std::path::PathBuf::from("io.github.rexkev214.dewclip.dev")
        );
    }
}

/// Portable mode: a `data` folder next to the executable holds the data, and
/// always wins over a data folder chosen in settings.
pub(crate) fn portable_data_dir() -> Option<std::path::PathBuf> {
    let exe_path = std::env::current_exe().ok()?;
    let portable_data = exe_path.parent()?.join("data");
    portable_data.is_dir().then_some(portable_data)
}




pub struct StartupSettings {
    pub persistent: bool,
    pub history_retention_days: u64,
    pub storage_limit_mb: u64,
    pub download_remote_images: bool,
    pub remote_image_max_bytes: u64,
    pub silent_start: bool,
    pub delete_after_paste: bool,
    pub cleanup_rules: String,
    pub app_cleanup_policies: String,
    pub sequential_hotkey: String,
    pub hide_tray_icon: bool,
    pub hide_dock_icon: bool,
    pub window_pinned: bool,
    pub window_width: Option<f64>,
    pub window_height: Option<f64>,
    pub main_hotkey: String,
    pub window_position_mode: String,
    pub window_pin_mode: WindowPinMode,
    /// No window has been shown yet: a new user, or settings just restored.
    pub first_open: bool,
    pub glass_level: u64,
    pub capsule_enabled: bool,
    pub capsule_style: String,
    pub capsule_duration_ms: u64,
}

pub(crate) fn load_settings(repo: &SqliteSettingsRepository) -> rusqlite::Result<StartupSettings> {
    let values = repo.app_settings()?;
    let get = |key: &str, default: &str| -> String {
        values.get(key).cloned()
            .unwrap_or_else(|| default.to_string())
    };
    let flag = |key: &str, default: bool| -> bool {
        match values.get(key) {
            Some(v) => v == "true",
            None => default,
        }
    };
    let size = |key: &str| -> Option<f64> {
        values.get(key)
            .and_then(|v| v.parse::<f64>().ok())
            .filter(|v| v.is_finite() && *v > 0.0)
    };
    Ok(StartupSettings {
        persistent: get("app.persistent", "true") != "false",
        history_retention_days: get("app.history_retention_days", "0").parse().unwrap_or(0),
        storage_limit_mb: get("app.storage_limit_mb", "0").parse().unwrap_or(0),
        download_remote_images: flag("app.download_remote_images", true),
        remote_image_max_bytes: crate::database::remote_image_max_bytes(&get("app.remote_image_max_mb", "8")),
        silent_start: flag("app.silent_start", false),
        delete_after_paste: flag("app.delete_after_paste", false),
        cleanup_rules: get("app.cleanup_rules", ""),
        app_cleanup_policies: get("app.app_cleanup_policies", "[]"),
        sequential_hotkey: get("app.sequential_hotkey", "Alt+V"),
        hide_tray_icon: flag("app.hide_tray_icon", false),
        hide_dock_icon: flag("app.hide_dock_icon", false),
        window_pinned: flag("app.window_pinned", false),
        window_width: size("app.window_logical_width"),
        window_height: size("app.window_logical_height"),
        main_hotkey: get("app.hotkey", "Alt+Z"),
        first_open: !flag("app.onboarding_done", false),
        window_position_mode: crate::app::window_manager::normalize_position_mode(&get(
            "app.window_position_mode",
            "follow_mouse",
        ))
        .to_string(),
        window_pin_mode: WindowPinMode::from_setting(&get(PIN_MODE_KEY, "always_on_top")),
        glass_level: get("app.glass_level", "35")
            .parse::<u64>()
            .map(|v| v.min(100))
            .unwrap_or(35),
        capsule_enabled: flag(crate::app::capsule::ENABLED_KEY, true),
        capsule_style: crate::app::capsule::normalize_style(&get(
            crate::app::capsule::STYLE_KEY,
            crate::app::capsule::STYLE_GLASS,
        ))
        .to_string(),
        capsule_duration_ms: crate::app::capsule::normalize_duration(&get(
            crate::app::capsule::DURATION_KEY,
            "",
        )),
    })
}

fn setup_state(
    app: &App,
    conn_arc: std::sync::Arc<std::sync::Mutex<rusqlite::Connection>>,
    s: &StartupSettings,
    app_dir: std::path::PathBuf,
) {
    let repo = SqliteClipboardRepository::new(conn_arc.clone());
    let settings_repo = SqliteSettingsRepository::new(conn_arc.clone());
    let tag_repo = SqliteTagRepository::new(conn_arc.clone());
    app.manage(DbState {
        conn: conn_arc,
        repo,
        settings_repo,
        tag_repo,
    });

    app.manage(settings_state_from(s));

    app.manage(SessionHistory(std::sync::Mutex::new(
        std::collections::VecDeque::new(),
    )));
    app.manage(AppDataDir(std::sync::Mutex::new(app_dir)));
    app.manage(PasteQueue::default());
}

pub(crate) fn settings_state_from(s: &StartupSettings) -> SettingsState {
    SettingsState {
        persistent: AtomicBool::new(s.persistent),
        history_retention_days: AtomicU64::new(s.history_retention_days),
        storage_limit_mb: AtomicU64::new(s.storage_limit_mb),
        download_remote_images: AtomicBool::new(s.download_remote_images),
        remote_image_max_bytes: AtomicU64::new(s.remote_image_max_bytes),
        capture_files: AtomicBool::new(true),
        capture_rich_text: AtomicBool::new(true),
        silent_start: AtomicBool::new(s.silent_start),
        delete_after_paste: AtomicBool::new(s.delete_after_paste),
        cleanup_rules: std::sync::Mutex::new(s.cleanup_rules.clone()),
        app_cleanup_policies: std::sync::Mutex::new(s.app_cleanup_policies.clone()),
        sequential_paste_hotkey: std::sync::Mutex::new(s.sequential_hotkey.clone()),
        hide_tray_icon: AtomicBool::new(s.hide_tray_icon),
        hide_dock_icon: AtomicBool::new(s.hide_dock_icon),
        main_hotkey: std::sync::Mutex::new(s.main_hotkey.clone()),
        window_position_mode: std::sync::Mutex::new(s.window_position_mode.clone()),
        window_pin_mode: std::sync::Mutex::new(s.window_pin_mode),
        glass_level: AtomicU64::new(s.glass_level),
        capsule_enabled: AtomicBool::new(s.capsule_enabled),
        capsule_style: std::sync::Mutex::new(s.capsule_style.clone()),
        capsule_duration_ms: AtomicU64::new(s.capsule_duration_ms),
    }
}

#[cfg(target_os = "macos")]
fn apply_initial_dock_visibility(app: &mut App, s: &StartupSettings) {
    let _ = app.set_dock_visibility(!s.hide_dock_icon);
}


/// Configure macOS Space / fullscreen overlay behavior for the main window.
#[cfg(target_os = "macos")]
fn set_window_all_spaces(window: &tauri::WebviewWindow) {
    crate::infrastructure::macos_api::window::configure_overlay_space_behavior(window);
}











fn setup_main_window(app: &App, s: &StartupSettings) {
    let effective_pinned = s.window_pinned;
    WINDOW_PINNED.store(effective_pinned, Ordering::Relaxed);

    if let Some(window) = app.get_webview_window("main") {
        if let (Some(w), Some(h)) = (s.window_width, s.window_height) {
            if w >= 300.0 && h >= 300.0 {
                let _ = window.set_size(tauri::Size::Logical(tauri::LogicalSize {
                    width: w,
                    height: h,
                }));
            }
        }
        // Space flags before panel conversion.
        set_window_all_spaces(&window);
    }

    // Handle silent start
    let args: Vec<String> = std::env::args().collect();
    let is_autostart =
        args.contains(&"--autostart".to_string()) || args.contains(&"--minimized".to_string());
    let should_show_main = !is_autostart && !s.silent_start;

    let Some(window) = app.get_webview_window("main") else {
        return;
    };

    if !should_show_main {
        // The main window is created visible (tauri.conf.json), so autostart
        // and silent start must hide it before the panel conversion.
        let _ = window.hide();
    }

    // Convert AFTER final show/hide work — to_panel drops tao ivars.
    if let Err(err) = crate::infrastructure::macos_api::window::setup_clipboard_panel(&window) {
        eprintln!("[macos-overlay] setup_clipboard_panel failed: {err}");
    }
    crate::app::window_manager::apply_window_vibrancy(&app.handle().clone(), &window);

    if should_show_main {
        if s.first_open {
            crate::app::window_manager::position_window_centered(&window);
        } else {
            crate::app::window_manager::position_window_for_show(&app.handle().clone(), &window);
        }
        crate::infrastructure::macos_api::window::configure_overlay_space_behavior(&window);
        let _ = crate::infrastructure::macos_api::window::show_clipboard_panel(&app.handle().clone());
        maybe_open_devtools(&window);
    }
}

/// New installs launch at login. Runs once, on the first start of a fresh
/// database (see `database::AUTOSTART_PENDING_KEY`); upgrades never touch the
/// user's login items.
fn enable_autostart_on_first_run(db_state: &DbState) {
    if db_state.settings_repo.get(database::AUTOSTART_PENDING_KEY).ok().flatten().is_none() {
        return;
    }
    let done = || {
        if let Ok(conn) = db_state.conn.lock() {
            let _ = conn.execute("DELETE FROM settings WHERE key = ?1", [database::AUTOSTART_PENDING_KEY]);
        }
    };
    // Dev builds and the portable exe move around; a login item would point
    // at a stale path.
    if cfg!(debug_assertions) || cfg!(feature = "portable") {
        return;
    }
    // Run from the mounted DMG or a quarantine copy (App Translocation): the
    // path is temporary, so wait for a start from the installed location.
    #[cfg(target_os = "macos")]
    if let Ok(exe) = std::env::current_exe() {
        let path = exe.to_string_lossy();
        if path.starts_with("/Volumes/") || path.contains("/AppTranslocation/") {
            return;
        }
    }
    use crate::app::commands::system_cmd::{apply_autostart, is_autostart_enabled};
    let _ = apply_autostart(true);
    if is_autostart_enabled().unwrap_or(false) {
        done();
    } else if login_item_switched_off() {
        // Registered, but the user switched the app off in System Settings
        // before (a reinstall): their choice, and nothing left to retry.
        done();
    } else {
        eprintln!(">>> [STARTUP] enabling launch at login failed; retrying next start");
    }
}

#[cfg(target_os = "macos")]
fn login_item_switched_off() -> bool {
    use crate::infrastructure::macos_api::login_item;
    login_item::available() && login_item::requires_approval()
}



fn start_services(app: &App, s: &StartupSettings, app_handle: AppHandle) {
    crate::services::clipboard::start_clipboard_monitor(app_handle.clone());

    let db_state = app.state::<DbState>();
    enable_autostart_on_first_run(&db_state);
    // Register initial hotkey
    let hotkey_str = s.main_hotkey.clone();
    {
        let mut guard = HOTKEY_STRING.lock().unwrap();
        *guard = hotkey_str.clone();
    }
    if let Err(error) = crate::app::commands::sync_registered_hotkeys(&app_handle) {
        crate::error!(">>> [HOTKEY] Initial registration failed: {error}");
    }

}


pub fn handle_global_shortcut(
    app: &AppHandle,
    shortcut: &tauri_plugin_global_shortcut::Shortcut,
    state: tauri_plugin_global_shortcut::ShortcutState,
) {
    use crate::app::commands::hotkey_cmd::parse_shortcut;
    use tauri_plugin_global_shortcut::ShortcutState;
    let settings = app.state::<SettingsState>();

    let main = parse_shortcut(&settings.main_hotkey.lock().unwrap());
    if main.as_ref() == Some(shortcut) {
        if state == ShortcutState::Pressed {
            toggle_window(app);
        }
    } else if state == ShortcutState::Released
        && parse_shortcut(&settings.sequential_paste_hotkey.lock().unwrap()).as_ref() == Some(shortcut)
    {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(error) = crate::services::paste_queue::paste_next_step(app).await {
                crate::error!("Sequential paste: {error}");
            }
        });
    }
}

pub fn handle_window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    if window.label() != "main" {
        return;
    }
    match event {
        tauri::WindowEvent::Focused(false) => {
            panel::remember_frontmost_external_app();
        }
        tauri::WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            let _ = crate::app::window_manager::hide_window_cmd(window.app_handle().clone());
        }
        _ => {}
    }
}






