use crate::app_state::{PasteQueue, SettingsState};
use crate::database::DbState;
use crate::error::{AppError, AppResult};
use crate::global_state::{HOTKEY_STRING, IS_RECORDING};
use crate::infrastructure::repository::settings_repo::SettingsRepository;
use std::collections::HashSet;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

fn is_win_v_hotkey(hotkey: &str) -> bool {
    let parts: Vec<String> = hotkey
        .split('+')
        .map(|p| p.trim().to_uppercase())
        .filter(|p| !p.is_empty())
        .collect();

    if parts.is_empty() {
        return false;
    }

    let mut has_win = false;
    let mut has_v = false;

    for part in &parts {
        match part.as_str() {
            "WIN" | "SUPER" | "COMMAND" | "META" => has_win = true,
            "V" => has_v = true,
            _ => return false,
        }
    }

    has_win && has_v
}

fn should_register_main_hotkey(hotkey: &str) -> bool {
    if hotkey.is_empty() {
        return false;
    }

    // Never turn a Win+V setting into Command+V and intercept system paste.
    !is_win_v_hotkey(hotkey) && !is_close_window_shortcut(hotkey)
}

fn is_close_window_shortcut(hotkey: &str) -> bool {
    cfg!(target_os = "macos") &&
        parse_shortcut(hotkey).is_some_and(|shortcut| Some(shortcut) == parse_shortcut("Command+W"))
}

fn validate_close_window_shortcut(hotkey: &str) -> AppResult<()> {
    if is_close_window_shortcut(hotkey) {
        return Err(AppError::Validation("hotkey_reserved_close".into()));
    }
    Ok(())
}

pub(crate) fn parse_shortcut(hotkey: &str) -> Option<Shortcut> {
    if hotkey.is_empty()
        || hotkey.eq_ignore_ascii_case("MouseMiddle")
        || hotkey.eq_ignore_ascii_case("MButton")
    {
        return None;
    }

    hotkey.replace("Win", "Super").parse::<Shortcut>().ok()
}

fn validate_distinct_hotkeys(main: &str, sequential: &str) -> AppResult<()> {
    if parse_shortcut(main).is_some_and(|shortcut| Some(shortcut) == parse_shortcut(sequential)) {
        return Err(AppError::Validation("hotkey_conflict".into()));
    }
    Ok(())
}




fn register_unique_shortcut(
    app_handle: &AppHandle,
    hotkey: &str,
    registered: &mut HashSet<String>,
) -> AppResult<()> {
    let shortcut = parse_shortcut(hotkey).ok_or_else(|| {
        AppError::Validation(format!("快捷键格式不正确，请重新设置。\n当前：{hotkey}"))
    })?;
    let key = format!("{shortcut:?}");
    if registered.insert(key) {
        app_handle
            .global_shortcut()
            .register(shortcut)
            .map_err(|e| AppError::Internal(format!("快捷键设置失败，请换一组按键。\n当前：{hotkey}\n详情：{e}")))?;
    }
    Ok(())
}

pub fn sync_registered_hotkeys(app_handle: &AppHandle) -> AppResult<()> {
    if IS_RECORDING.load(Ordering::SeqCst) {
        return Ok(());
    }
    let settings = app_handle.state::<SettingsState>();
    let main_hotkey = settings.main_hotkey.lock().unwrap().clone();
    let sequential_hotkey = settings.sequential_paste_hotkey.lock().unwrap().clone();
    apply_registered_hotkeys(app_handle, &main_hotkey, &sequential_hotkey)
}

pub(crate) fn registered_sequential_hotkey(app: &AppHandle) -> Option<String> {
    if IS_RECORDING.load(Ordering::SeqCst) {
        return None;
    }
    let settings = app.state::<SettingsState>();
    let main = settings.main_hotkey.lock().unwrap().clone();
    let hotkey = settings.sequential_paste_hotkey.lock().unwrap().clone();
    if !should_register_main_hotkey(&hotkey) || validate_distinct_hotkeys(&main, &hotkey).is_err() {
        return None;
    }
    parse_shortcut(&hotkey)
        .filter(|shortcut| app.global_shortcut().is_registered(*shortcut))
        .map(|_| hotkey)
}

fn apply_registered_hotkeys(app_handle: &AppHandle, main_hotkey: &str, sequential_hotkey: &str) -> AppResult<()> {
    app_handle.global_shortcut().unregister_all()
        .map_err(|error| AppError::Internal(error.to_string()))?;
    let has_queued_items = app_handle.state::<PasteQueue>().0.lock().unwrap().has_items();
    let mut registered = HashSet::new();
    // Register every shortcut even if one fails (e.g. taken by another app):
    // an early return used to leave the later ones unregistered, so pressing
    // them typed a character instead (Option+F typing "ƒ"). Failures are
    // logged to dewclip.log; the first one is returned.
    let mut first_error: Option<AppError> = None;
    let mut register = |hotkey: &str| {
        if let Err(err) = register_unique_shortcut(app_handle, hotkey, &mut registered) {
            crate::error!(">>> [HOTKEY] {err}");
            first_error.get_or_insert(err);
        }
    };

    if should_register_main_hotkey(main_hotkey) {
        register(main_hotkey);
    }

    if has_queued_items && should_register_main_hotkey(sequential_hotkey)
        && validate_distinct_hotkeys(main_hotkey, sequential_hotkey).is_ok()
    {
        register(sequential_hotkey);
    }

    first_error.map_or(Ok(()), Err)
}

/// `sync_registered_hotkeys` for background threads. The shortcut plugin
/// holds its lock while it waits for the main thread, and the main thread
/// takes that lock to deliver a hotkey press, so a press at the wrong moment
/// froze both. On the main thread the two can't overlap.
pub fn sync_registered_hotkeys_on_main_thread(app_handle: &AppHandle) {
    let handle = app_handle.clone();
    let _ = app_handle.run_on_main_thread(move || {
        let _ = sync_registered_hotkeys(&handle);
    });
}

#[tauri::command]
pub fn register_hotkey(app_handle: AppHandle, hotkey: String) -> AppResult<()> {
    validate_close_window_shortcut(&hotkey)?;
    let settings = app_handle.state::<SettingsState>();
    let previous = settings.main_hotkey.lock().unwrap().clone();
    let sequential = settings.sequential_paste_hotkey.lock().unwrap().clone();
    validate_distinct_hotkeys(&hotkey, &sequential)?;
    let result = apply_registered_hotkeys(&app_handle, &hotkey, &sequential).and_then(|()| {
        app_handle
            .state::<DbState>()
            .settings_repo
            .set("app.hotkey", &hotkey)
            .map_err(AppError::from)
    });
    if let Err(error) = result {
        if let Err(restore_error) = apply_registered_hotkeys(&app_handle, &previous, &sequential) {
            return Err(AppError::Internal(format!("{error}\n{restore_error}")));
        }
        crate::app::hooks::update_recording_flags(false);
        return Err(error);
    }

    *settings.main_hotkey.lock().unwrap() = hotkey.clone();
    *HOTKEY_STRING.lock().unwrap() = hotkey;
    crate::app::hooks::update_recording_flags(false);
    Ok(())
}

#[tauri::command]
pub fn set_sequential_hotkey(app_handle: AppHandle, hotkey: String) -> AppResult<()> {
    validate_close_window_shortcut(&hotkey)?;
    let settings = app_handle.state::<SettingsState>();
    let main = settings.main_hotkey.lock().unwrap().clone();
    validate_distinct_hotkeys(&main, &hotkey)?;
    if !hotkey.is_empty() && (parse_shortcut(&hotkey).is_none() || is_typing_hotkey(&hotkey) || is_win_v_hotkey(&hotkey)) {
        return Err(AppError::Validation("sequential_invalid_hotkey".into()));
    }

    let previous = settings.sequential_paste_hotkey.lock().unwrap().clone();
    let recording = IS_RECORDING.load(Ordering::SeqCst);
    let result = (if recording { Ok(()) } else { apply_registered_hotkeys(&app_handle, &main, &hotkey) })
        .and_then(|()| app_handle.state::<DbState>().settings_repo.set("app.sequential_hotkey", &hotkey).map_err(AppError::from));
    if let Err(error) = result {
        if !recording {
            apply_registered_hotkeys(&app_handle, &main, &previous)?;
        }
        return Err(error);
    }
    *settings.sequential_paste_hotkey.lock().unwrap() = hotkey;
    Ok(())
}

/// Shift alone (or no modifier) plus a key that types text: registered
/// globally it swallows that character in every app. Mirrors isTypingHotkey
/// in src/shared/lib/hotkeyKey.ts.
pub fn is_typing_hotkey(hotkey: &str) -> bool {
    let parts: Vec<String> = hotkey
        .split('+')
        .map(|p| p.trim().to_uppercase())
        .filter(|p| !p.is_empty())
        .collect();
    let Some(first) = parts.first() else { return false };
    if first == "MOUSEMIDDLE" || first == "MBUTTON" {
        return false;
    }
    const MODIFIERS: [&str; 10] =
        ["CTRL", "CONTROL", "ALT", "OPTION", "MENU", "WIN", "SUPER", "COMMAND", "CMD", "META"];
    if parts.iter().any(|p| MODIFIERS.contains(&p.as_str())) {
        return false;
    }
    let Some(key) = parts.iter().filter(|p| p.as_str() != "SHIFT").last() else { return false };
    let silent = (key.len() >= 2
        && key.starts_with('F')
        && key[1..].parse::<u8>().map_or(false, |n| (1..=24).contains(&n)))
        || matches!(
            key.as_str(),
            "ESC" | "ESCAPE" | "INSERT" | "DELETE" | "HOME" | "END" | "PAGEUP" | "PAGEDOWN"
                | "UP" | "DOWN" | "LEFT" | "RIGHT" | "ARROWUP" | "ARROWDOWN" | "ARROWLEFT"
                | "ARROWRIGHT" | "PRINTSCREEN" | "SCROLLLOCK" | "PAUSE" | "NUMLOCK"
        );
    !silent
}

/// `allow_typing`: the wake hotkey may be Shift + a letter (e.g. ⇧Z) — the
/// user accepts that the character can't be typed while it is registered.
/// Shift plus one letter or digit, e.g. `Shift+Z`.
pub fn is_shift_letter_hotkey(hotkey: &str) -> bool {
    let parts: Vec<String> = hotkey
        .split('+')
        .map(|p| p.trim().to_uppercase())
        .filter(|p| !p.is_empty())
        .collect();
    parts.len() == 2
        && parts[0] == "SHIFT"
        && parts[1].len() == 1
        && parts[1].chars().all(|c| c.is_ascii_alphanumeric())
}

#[tauri::command]
pub fn test_hotkey_available(
    app_handle: AppHandle,
    hotkey: String,
    allow_typing: Option<bool>,
    for_sequential: Option<bool>,
) -> AppResult<bool> {
    validate_close_window_shortcut(&hotkey)?;
    if hotkey.is_empty()
        || hotkey.eq_ignore_ascii_case("MouseMiddle")
        || hotkey.eq_ignore_ascii_case("MButton")
    {
        return Ok(true);
    }
    if for_sequential == Some(true) && (is_typing_hotkey(&hotkey) || is_win_v_hotkey(&hotkey)) {
        return Err(AppError::Validation("sequential_invalid_hotkey".into()));
    }
    if is_typing_hotkey(&hotkey) && !(allow_typing.unwrap_or(false) && is_shift_letter_hotkey(&hotkey)) {
        return Err(AppError::Validation(
            "快捷键需要包含 Ctrl、Option/Alt 或 Command/Win。".to_string(),
        ));
    }

    let normalized = hotkey.replace("Win", "Super");
    let shortcut = normalized
        .parse::<Shortcut>()
        .map_err(|_| AppError::Validation("请重新设置快捷键组合。".to_string()))?;

    let settings = app_handle.state::<SettingsState>();
    let main = settings.main_hotkey.lock().unwrap().clone();
    let sequential = settings.sequential_paste_hotkey.lock().unwrap().clone();
    let current = if for_sequential == Some(true) {
        validate_distinct_hotkeys(&main, &hotkey)?;
        &sequential
    } else {
        validate_distinct_hotkeys(&hotkey, &sequential)?;
        &main
    };
    if parse_shortcut(current) == Some(shortcut) && app_handle.global_shortcut().is_registered(shortcut) {
        return Ok(true);
    }

    match app_handle.global_shortcut().register(shortcut.clone()) {
        Ok(_) => {
            app_handle.global_shortcut().unregister(shortcut)
                .map_err(|error| AppError::Internal(error.to_string()))?;
            Ok(true)
        }
        Err(e) => {
            let err_str = format!("{:?}", e);
            let user_msg = if err_str.contains("AlreadyRegistered") {
                "该快捷键已被其他程序占用".to_string()
            } else {
                "快捷键不可用".to_string()
            };
            Err(AppError::Internal(user_msg))
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "macos")]
    #[test]
    fn command_w_cannot_be_registered_as_a_global_hotkey() {
        for keyboard in ["Command+W", "Super+KeyW", "Cmd+w"] {
            assert!(!super::should_register_main_hotkey(keyboard));
            assert!(super::validate_close_window_shortcut(keyboard).is_err());
        }
        assert!(super::should_register_main_hotkey("Command+Shift+W"));
        assert!(super::should_register_main_hotkey("Ctrl+W"));
    }

    #[test]
    fn wake_and_sequential_hotkeys_cannot_share_a_normalized_binding() {
        for (wake, sequential) in [("Alt+V", "Option+v"), ("Option+v", "Alt+V"), ("Win+S", "Command+s")] {
            assert_eq!(super::validate_distinct_hotkeys(wake, sequential).unwrap_err().to_string(), "[validation] hotkey_conflict");
        }
        assert!(super::validate_distinct_hotkeys("Alt+Z", "Alt+V").is_ok());
        assert!(super::validate_distinct_hotkeys("", "").is_ok());
    }
    #[test]
    fn shift_letter_wake_hotkey_is_recognized() {
        assert!(super::is_shift_letter_hotkey("Shift+Z"));
        assert!(!super::is_shift_letter_hotkey("Shift+F5"));
        assert!(!super::is_shift_letter_hotkey("Alt+Z"));
    }

    #[test]
    fn typing_hotkeys_are_refused() {
        for h in ["Shift+S", "Shift+1", "C", "Space"] {
            assert!(super::is_typing_hotkey(h), "{h}");
        }
        for h in ["Alt+C", "Command+Shift+V", "Ctrl+1", "F5", "Shift+F5", "MouseMiddle", ""] {
            assert!(!super::is_typing_hotkey(h), "{h}");
        }
    }

    use super::{is_win_v_hotkey, should_register_main_hotkey};

    #[test]
    fn recognizes_win_v_aliases_without_extra_modifiers() {
        assert!(is_win_v_hotkey("Win+V"));
        assert!(is_win_v_hotkey("Super + v"));
        assert!(!is_win_v_hotkey("Ctrl+Win+V"));
        assert!(!is_win_v_hotkey("Win+C"));
    }

    #[test]
    fn registers_win_v_only_on_windows() {
        assert!(should_register_main_hotkey("Alt+C"));
        assert!(!should_register_main_hotkey(""));
        assert_eq!(
            should_register_main_hotkey("Win+V"),
            cfg!(target_os = "windows")
        );
    }
}
