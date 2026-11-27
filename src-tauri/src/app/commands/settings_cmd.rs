use crate::app::appearance::{self, AppearancePreference};
use crate::app::commands::hotkey_cmd::sync_registered_hotkeys;
use crate::app_state::SettingsState;
use crate::database::DbState;
use crate::error::{AppError, AppResult};
use crate::infrastructure::macos_api::window::{self as panel, WindowPinMode, PIN_MODE_KEY};
use crate::infrastructure::repository::settings_repo::SettingsRepository;
use crate::services::updater;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Manager, State};

#[tauri::command]
pub fn save_setting(
    app_handle: AppHandle,
    db_state: State<'_, DbState>,
    settings_state: State<'_, crate::app_state::SettingsState>,
    key: String,
    value: String,
) -> AppResult<()> {
    if crate::database::is_translation_setting(&key) {
        return Err(AppError::Validation("use save_translation_config".into()));
    }
    if updater::is_internal_setting(&key) {
        return Err(AppError::Validation("update_failed".into()));
    }
    if key == updater::ENABLED_KEY {
        return updater::set_enabled(&app_handle, value == "true");
    }
    if key == updater::ONBOARDING_DONE_KEY {
        return updater::set_onboarding_done(&app_handle, &value);
    }
    let atomic_setting = match key.as_str() {
        "app.history_retention_days" => Some((&settings_state.history_retention_days, value.parse().unwrap_or(0))),
        "app.storage_limit_mb" => Some((&settings_state.storage_limit_mb, value.parse().unwrap_or(0))),
        "app.remote_image_max_mb" => Some((&settings_state.remote_image_max_bytes, crate::database::remote_image_max_bytes(&value))),
        _ => None,
    };
    if let Some((setting, parsed)) = atomic_setting {
        db_state.settings_repo.set(&key, &value)?;
        setting.store(parsed, Ordering::Relaxed);
        return Ok(());
    }
    if key == "app.download_remote_images" {
        db_state.settings_repo.set(&key, &value)?;
        settings_state.download_remote_images.store(value == "true", Ordering::Relaxed);
        return Ok(());
    }
    match key.as_str() {
        appearance::SETTING_KEY => return appearance::set_appearance(app_handle, AppearancePreference::from_setting(&value)),
        "app.hotkey" => return super::hotkey_cmd::register_hotkey(app_handle, value),
        "app.sequential_hotkey" => return super::hotkey_cmd::set_sequential_hotkey(app_handle, value),
        "app.persistent" => {
            settings_state
                .persistent
                .store(value != "false", Ordering::Relaxed);
        }
        "app.silent_start" => {
            settings_state
                .silent_start
                .store(value == "true", Ordering::Relaxed);
        }
        "app.delete_after_paste" => {
            settings_state
                .delete_after_paste
                .store(value == "true", Ordering::Relaxed);
        }
        "app.hide_tray_icon" => {
            settings_state
                .hide_tray_icon
                .store(value == "true", Ordering::Relaxed);
        }
        "app.hide_dock_icon" => {
            settings_state
                .hide_dock_icon
                .store(value == "true", Ordering::Relaxed);
        }
        "app.window_position_mode" => {
            if let Ok(mut guard) = settings_state.window_position_mode.lock() {
                *guard = crate::app::window_manager::normalize_position_mode(&value).to_string();
            }
        }
        PIN_MODE_KEY => return set_window_pin_mode(app_handle, WindowPinMode::from_setting(&value)),
        crate::app::capsule::ENABLED_KEY => {
            let enabled = value != "false";
            settings_state.capsule_enabled.store(enabled, Ordering::Relaxed);
            if !enabled {
                crate::app::capsule::dismiss(&app_handle);
            }
        }
        crate::app::capsule::DURATION_KEY => {
            settings_state
                .capsule_duration_ms
                .store(crate::app::capsule::normalize_duration(&value), Ordering::Relaxed);
        }
        crate::app::capsule::STYLE_KEY => {
            if let Ok(mut guard) = settings_state.capsule_style.lock() {
                *guard = crate::app::capsule::normalize_style(&value).to_string();
            }
        }
        _ => {}
    }

    db_state
        .settings_repo
        .set(&key, &value)
        .map_err(AppError::from)?;
    if key == "app.language" {
        crate::app::tray::refresh_tray(&app_handle);
    }
    Ok(())
}


#[tauri::command]
pub fn acquire_blur_guard(owner: String) -> Result<usize, String> {
    let owner = owner.trim();
    if owner.is_empty() {
        return Err("Blur guard owner cannot be empty".to_string());
    }
    Ok(crate::global_state::acquire_blur_guard(owner))
}

#[tauri::command]
pub fn release_blur_guard(owner: String) -> Result<usize, String> {
    let owner = owner.trim();
    if owner.is_empty() {
        return Err("Blur guard owner cannot be empty".to_string());
    }
    Ok(crate::global_state::release_blur_guard(owner))
}

#[tauri::command]
pub fn set_window_pinned(app_handle: AppHandle, pinned: bool) {
    set_pinned_state(&app_handle, pinned);
}

/// A pinned window stays open after focus moves to another app or after pasting.
pub fn set_pinned_state(app_handle: &AppHandle, pinned: bool) {
    crate::WINDOW_PINNED.store(pinned, Ordering::Relaxed);
    let _ = panel::refresh_clipboard_panel_policy(app_handle);
    if let Some(state) = app_handle.try_state::<DbState>() {
        let _ = state
            .settings_repo
            .set("app.window_pinned", &pinned.to_string());
    }
}

#[tauri::command]
pub fn set_window_pin_mode(app_handle: AppHandle, mode: WindowPinMode) -> AppResult<()> {
    let settings = app_handle.state::<SettingsState>();
    {
        let mut current = settings.window_pin_mode.lock().unwrap();
        app_handle
            .state::<DbState>()
            .settings_repo
            .set(PIN_MODE_KEY, mode.as_setting())
            .map_err(AppError::from)?;
        *current = mode;
    }
    panel::refresh_clipboard_panel_policy(&app_handle)
        .map_err(|error| AppError::Internal(error.to_string()))
}

#[tauri::command]
pub fn get_settings(
    state: State<'_, DbState>,
) -> AppResult<std::collections::HashMap<String, String>> {
    let mut settings = state.settings_repo.get_all()?;
    settings.retain(|key, _| !updater::is_internal_setting(key));
    Ok(settings)
}






#[tauri::command]
pub fn set_persistence(
    state: State<'_, crate::app_state::SettingsState>,
    db_state: State<'_, DbState>,
    enabled: bool,
) -> AppResult<()> {
    state.persistent.store(enabled, Ordering::Relaxed);
    db_state
        .settings_repo
        .set("app.persistent", &enabled.to_string())
        .map_err(AppError::from)
}





#[tauri::command]
pub fn set_silent_start(
    state: State<'_, crate::app_state::SettingsState>,
    db_state: State<'_, DbState>,
    enabled: bool,
) -> AppResult<()> {
    state.silent_start.store(enabled, Ordering::Relaxed);
    db_state
        .settings_repo
        .set("app.silent_start", &enabled.to_string())
        .map_err(AppError::from)
}

#[tauri::command]
pub fn set_delete_after_paste(
    state: State<'_, crate::app_state::SettingsState>,
    db_state: State<'_, DbState>,
    enabled: bool,
) -> AppResult<()> {
    state.delete_after_paste.store(enabled, Ordering::Relaxed);
    db_state
        .settings_repo
        .set("app.delete_after_paste", &enabled.to_string())
        .map_err(AppError::from)
}




#[tauri::command]
pub fn set_cleanup_rules(
    state: State<'_, crate::app_state::SettingsState>,
    db_state: State<'_, DbState>,
    rules: String,
) -> AppResult<()> {
    let mut guard = state.cleanup_rules.lock().unwrap();
    *guard = rules.clone();
    db_state
        .settings_repo
        .set("app.cleanup_rules", &rules)
        .map_err(AppError::from)
}

#[tauri::command]
pub fn set_app_cleanup_policies(
    state: State<'_, crate::app_state::SettingsState>,
    db_state: State<'_, DbState>,
    policies: String,
) -> AppResult<()> {
    let mut guard = state.app_cleanup_policies.lock().unwrap();
    db_state.settings_repo.set("app.app_cleanup_policies", &policies)?;
    *guard = policies;
    Ok(())
}












#[tauri::command]
pub fn reset_settings(
    app: AppHandle,
    state: State<'_, DbState>,
    settings_state: State<'_, crate::app_state::SettingsState>,
) -> AppResult<()> {
    updater::reset_settings(&app)?;
    // Running state follows the defaults now, not after the next start.
    let fresh = crate::app::setup::load_settings(&state.settings_repo)?;
    settings_state.take_values_from(crate::app::setup::settings_state_from(&fresh));
    appearance::reapply(&app)?;
    crate::app::tray::refresh_tray(&app);
    panel::refresh_clipboard_panel_policy(&app)
        .map_err(|error| AppError::Internal(error.to_string()))?;
    *crate::global_state::HOTKEY_STRING.lock().unwrap() = fresh.main_hotkey.clone();
    // The window applies the rest when it reloads, but not these two.
    let _ = set_tray_visible(app.clone(), app.state(), !fresh.hide_tray_icon);
    let _ = set_dock_visible(app.clone(), app.state(), !fresh.hide_dock_icon);

    sync_registered_hotkeys(&app)
}

#[tauri::command]
pub fn set_tray_visible(
    app_handle: AppHandle,
    state: State<'_, crate::app_state::SettingsState>,
    visible: bool,
) -> AppResult<()> {
    state.hide_tray_icon.store(!visible, Ordering::Relaxed);
    if let Some(tray) = app_handle.tray_by_id(crate::app::tray::TRAY_ID) {
        let _ = tray.set_visible(visible);
    }
    let db_state = app_handle.state::<DbState>();
    db_state
        .settings_repo
        .set("app.hide_tray_icon", &(!visible).to_string())
        .map_err(AppError::from)
}

#[tauri::command]
pub fn set_dock_visible(
    app_handle: AppHandle,
    state: State<'_, crate::app_state::SettingsState>,
    visible: bool,
) -> AppResult<()> {
    state.hide_dock_icon.store(!visible, Ordering::Relaxed);
    #[cfg(target_os = "macos")]
    {
        crate::infrastructure::macos_api::window::set_dock_visibility_preserving_panel(
            &app_handle, visible,
        ).map_err(AppError::from)?;
        if !visible {
            // Tao intentionally drops a hide within one second of showing the
            // Dock icon. Retry beyond that interval, but respect a newer toggle.
            let app = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
                let app_on_main = app.clone();
                let _ = app.run_on_main_thread(move || {
                    let settings = app_on_main.state::<crate::app_state::SettingsState>();
                    if settings.hide_dock_icon.load(Ordering::Relaxed) {
                        if let Err(error) = crate::infrastructure::macos_api::window::set_dock_visibility_preserving_panel(&app_on_main, false) {
                            eprintln!("Failed to retry hiding Dock icon: {error}");
                        }
                    }
                });
            });
        }
    }
    let db_state = app_handle.state::<DbState>();
    db_state
        .settings_repo
        .set("app.hide_dock_icon", &(!visible).to_string())
        .map_err(AppError::from)
}


