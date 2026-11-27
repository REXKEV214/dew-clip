use crate::domain::models::{ClipboardEntry, EntryIdentity};
use crate::infrastructure::macos_api::window::WindowPinMode;
use crate::services::encryption_queue::EncryptionQueue;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

pub struct SettingsState {
    pub persistent: AtomicBool,
    pub history_retention_days: AtomicU64,
    pub storage_limit_mb: AtomicU64,
    pub download_remote_images: AtomicBool,
    pub remote_image_max_bytes: AtomicU64,
    /// Always on in DewClip: the 文件 / 视频 tabs need file copies.
    pub capture_files: AtomicBool,
    /// Always on in DewClip: 原格式 / Markdown paste needs the HTML.
    pub capture_rich_text: AtomicBool,
    pub silent_start: AtomicBool,
    pub delete_after_paste: AtomicBool,
    pub cleanup_rules: Mutex<String>,
    pub app_cleanup_policies: Mutex<String>,
    pub sequential_paste_hotkey: Mutex<String>,
    pub hide_tray_icon: AtomicBool,
    pub hide_dock_icon: AtomicBool,
    pub main_hotkey: Mutex<String>,
    /// `follow_mouse` | `last` | `fixed`
    pub window_position_mode: Mutex<String>,
    pub window_pin_mode: Mutex<WindowPinMode>,
    /// 0 (clearest) – 100 (most tinted).
    pub glass_level: AtomicU64,
    /// 复制提示弹窗: the pill that pops up at the top of the screen on each copy.
    pub capsule_enabled: AtomicBool,
    /// `glass` | `island` | `black`
    pub capsule_style: Mutex<String>,
    /// How long it stays out, in ms.
    pub capsule_duration_ms: AtomicU64,
}

impl SettingsState {
    /// Takes every value from `fresh` (「恢复初始设置」). No `..` below: a new
    /// field won't compile until it's handled here.
    pub fn take_values_from(&self, fresh: SettingsState) {
        let SettingsState {
            persistent,
            history_retention_days,
            storage_limit_mb,
            download_remote_images,
            remote_image_max_bytes,
            capture_files,
            capture_rich_text,
            silent_start,
            delete_after_paste,
            cleanup_rules,
            app_cleanup_policies,
            sequential_paste_hotkey,
            hide_tray_icon,
            hide_dock_icon,
            main_hotkey,
            window_position_mode,
            window_pin_mode,
            glass_level,
            capsule_enabled,
            capsule_style,
            capsule_duration_ms,
        } = fresh;
        self.persistent.store(persistent.into_inner(), Ordering::Relaxed);
        self.history_retention_days.store(history_retention_days.into_inner(), Ordering::Relaxed);
        self.storage_limit_mb.store(storage_limit_mb.into_inner(), Ordering::Relaxed);
        self.download_remote_images.store(download_remote_images.into_inner(), Ordering::Relaxed);
        self.remote_image_max_bytes.store(remote_image_max_bytes.into_inner(), Ordering::Relaxed);
        self.capture_files.store(capture_files.into_inner(), Ordering::Relaxed);
        self.capture_rich_text.store(capture_rich_text.into_inner(), Ordering::Relaxed);
        self.silent_start.store(silent_start.into_inner(), Ordering::Relaxed);
        self.delete_after_paste.store(delete_after_paste.into_inner(), Ordering::Relaxed);
        *self.cleanup_rules.lock().unwrap() = cleanup_rules.into_inner().unwrap();
        *self.app_cleanup_policies.lock().unwrap() = app_cleanup_policies.into_inner().unwrap();
        *self.sequential_paste_hotkey.lock().unwrap() = sequential_paste_hotkey.into_inner().unwrap();
        self.hide_tray_icon.store(hide_tray_icon.into_inner(), Ordering::Relaxed);
        self.hide_dock_icon.store(hide_dock_icon.into_inner(), Ordering::Relaxed);
        *self.main_hotkey.lock().unwrap() = main_hotkey.into_inner().unwrap();
        *self.window_position_mode.lock().unwrap() = window_position_mode.into_inner().unwrap();
        *self.window_pin_mode.lock().unwrap() = window_pin_mode.into_inner().unwrap();
        self.glass_level.store(glass_level.into_inner(), Ordering::Relaxed);
        self.capsule_enabled.store(capsule_enabled.into_inner(), Ordering::Relaxed);
        *self.capsule_style.lock().unwrap() = capsule_style.into_inner().unwrap();
        self.capsule_duration_ms.store(capsule_duration_ms.into_inner(), Ordering::Relaxed);
    }
}

#[derive(Default)]
pub struct PasteQueueState {
    pub kind: SequentialSessionKind,
    pub items: VecDeque<SequentialItem>,
    pub recording_since: Option<Instant>,
    pub last_paste_at: Option<Instant>,
    pub round_total: usize,
    pub pasted_in_round: usize,
    pub idle_generation: u64,
    pub idle_deadline_ms: Option<i64>,
    pub next_item_id: u64,
    pub one_shot_clipboard: Option<OneShotClipboard>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SequentialSessionKind {
    #[default]
    Inactive,
    Recording,
    OneShot,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SequentialItem {
    pub queue_id: u64,
    pub identity: EntryIdentity,
    pub content_type: String,
    pub preview: String,
    pub media_path: Option<String>,
    pub timestamp: i64,
}

pub struct OneShotClipboard {
    pub before: crate::infrastructure::macos_api::clipboard::PasteboardSnapshot,
    pub last_write: Option<isize>,
}

#[derive(Default)]
pub struct PasteQueue(pub Mutex<PasteQueueState>);

pub struct SessionHistory(pub Mutex<VecDeque<ClipboardEntry>>);

pub struct AppDataDir(pub Mutex<std::path::PathBuf>);

pub struct EncryptionQueueState(pub EncryptionQueue);

#[cfg(test)]
mod tests {
    use crate::app::setup::{load_settings, settings_state_from};
    use crate::infrastructure::macos_api::window::{WindowPinMode, PIN_MODE_KEY};
    use crate::infrastructure::repository::settings_repo::{SettingsRepository, SqliteSettingsRepository};
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex};

    #[test]
    fn a_settings_reset_reaches_the_running_state() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);")
            .unwrap();
        let repo = SqliteSettingsRepository::new(Arc::new(Mutex::new(conn)));
        let defaults = settings_state_from(&load_settings(&repo).unwrap());
        repo.set("app.delete_after_paste", "true").unwrap();
        repo.set("app.cleanup_rules", "secret => ").unwrap();
        repo.set("app.hotkey", "Ctrl+Shift+K").unwrap();
        repo.set(PIN_MODE_KEY, "normal").unwrap();
        repo.set("app.persistent_limit_enabled", "true").unwrap();
        repo.set("app.persistent_limit", "50").unwrap();
        let upgraded = load_settings(&repo).unwrap();
        assert_eq!(upgraded.history_retention_days, 0);
        assert_eq!(upgraded.storage_limit_mb, 0);
        assert_eq!(repo.get("app.persistent_limit").unwrap().as_deref(), Some("50"));
        repo.set("app.history_retention_days", "30").unwrap();
        repo.set("app.storage_limit_mb", "1024").unwrap();
        repo.set("app.download_remote_images", "false").unwrap();
        repo.set("app.remote_image_max_mb", "16").unwrap();
        let running = settings_state_from(&load_settings(&repo).unwrap());
        assert!(running.delete_after_paste.load(Ordering::Relaxed));
        assert_eq!(*running.window_pin_mode.lock().unwrap(), WindowPinMode::Normal);
        assert_eq!(running.history_retention_days.load(Ordering::Relaxed), 30);
        assert_eq!(running.storage_limit_mb.load(Ordering::Relaxed), 1024);
        assert!(!running.download_remote_images.load(Ordering::Relaxed));
        assert_eq!(running.remote_image_max_bytes.load(Ordering::Relaxed), 16 * crate::database::BYTES_PER_MB);

        repo.clear().unwrap();
        running.take_values_from(settings_state_from(&load_settings(&repo).unwrap()));

        assert!(!running.delete_after_paste.load(Ordering::Relaxed));
        assert_eq!(*running.cleanup_rules.lock().unwrap(), *defaults.cleanup_rules.lock().unwrap());
        assert_eq!(*running.main_hotkey.lock().unwrap(), *defaults.main_hotkey.lock().unwrap());
        assert_eq!(*running.window_pin_mode.lock().unwrap(), WindowPinMode::AlwaysOnTop);
        assert_eq!(running.history_retention_days.load(Ordering::Relaxed), 0);
        assert_eq!(running.storage_limit_mb.load(Ordering::Relaxed), 0);
        assert!(running.download_remote_images.load(Ordering::Relaxed));
        assert_eq!(running.remote_image_max_bytes.load(Ordering::Relaxed), 8 * crate::database::BYTES_PER_MB);
    }
}
