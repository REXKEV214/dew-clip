use std::sync::Mutex;
use std::time::Duration;

use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSAppearanceNameAqua, NSAppearanceNameDarkAqua,
    NSApplication, NSWindow,
};
use objc2_foundation::{
    NSArray, NSDistributedNotificationCenter, NSNotification, NSNotificationSuspensionBehavior,
    NSObject, NSString,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::app::{capsule, window_manager};
use crate::app_state::SettingsState;
use crate::database::DbState;
use crate::error::AppResult;
use crate::infrastructure::repository::settings_repo::SettingsRepository;

pub const SETTING_KEY: &str = "app.appearance";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AppearancePreference {
    System,
    Light,
    Dark,
}

impl AppearancePreference {
    pub fn from_setting(value: &str) -> Self {
        match value {
            "light" => Self::Light,
            "dark" => Self::Dark,
            _ => Self::System,
        }
    }

    pub fn as_setting(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EffectiveTheme {
    #[default]
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct AppearanceState {
    preference: AppearancePreference,
    pub effective: EffectiveTheme,
}

fn resolve_effective_theme(
    preference: AppearancePreference,
    system_is_dark: bool,
) -> EffectiveTheme {
    match preference {
        AppearancePreference::Dark => EffectiveTheme::Dark,
        AppearancePreference::System if system_is_dark => EffectiveTheme::Dark,
        _ => EffectiveTheme::Light,
    }
}

fn native_application() -> objc2::rc::Retained<NSApplication> {
    NSApplication::sharedApplication(
        MainThreadMarker::new().expect("Appearance requires the main thread"),
    )
}

fn read_state(native: &NSApplication, preference: AppearancePreference) -> AppearanceState {
    let system_is_dark = preference == AppearancePreference::System
        && unsafe {
            let appearances =
                NSArray::from_slice(&[NSAppearanceNameAqua, NSAppearanceNameDarkAqua]);
            native
                .effectiveAppearance()
                .bestMatchFromAppearancesWithNames(&appearances)
                .as_deref()
                == Some(NSAppearanceNameDarkAqua)
        };
    AppearanceState {
        preference,
        effective: resolve_effective_theme(preference, system_is_dark),
    }
}

fn apply_preference(preference: AppearancePreference) -> AppearanceState {
    let native = native_application();
    let name = unsafe {
        match preference {
            AppearancePreference::System => None,
            AppearancePreference::Light => Some(NSAppearanceNameAqua),
            AppearancePreference::Dark => Some(NSAppearanceNameDarkAqua),
        }
    };
    let appearance =
        name.map(|name| NSAppearance::appearanceNamed(name).expect("Missing AppKit appearance"));
    native.setAppearance(appearance.as_deref());
    read_state(&native, preference)
}

pub fn init(app: &AppHandle, preference: AppearancePreference) {
    app.manage(Mutex::new(apply_preference(preference)));
    register_observer(app);
}

#[tauri::command]
pub fn get_appearance_state(app: AppHandle) -> AppearanceState {
    *app.state::<Mutex<AppearanceState>>().lock().unwrap()
}

pub fn effective_theme(app: &AppHandle) -> EffectiveTheme {
    app.state::<Mutex<AppearanceState>>()
        .lock()
        .unwrap()
        .effective
}

pub fn set_appearance(app: AppHandle, preference: AppearancePreference) -> AppResult<()> {
    {
        let settings = app.state::<SettingsState>();
        let mut current = settings.appearance.lock().unwrap();
        app.state::<DbState>()
            .settings_repo
            .set(SETTING_KEY, preference.as_setting())?;
        *current = preference;
    }
    reapply(&app)
}

pub fn reapply(app: &AppHandle) -> AppResult<()> {
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let preference = *handle.state::<SettingsState>().appearance.lock().unwrap();
        publish(&handle, apply_preference(preference), true);
    })?;
    Ok(())
}

/// Refreshes the inherited theme on the main thread.
pub fn refresh_system_theme(app: &AppHandle) {
    let preference = *app.state::<SettingsState>().appearance.lock().unwrap();
    if preference == AppearancePreference::System {
        publish(app, read_state(&native_application(), preference), false);
    }
}

fn publish(app: &AppHandle, next: AppearanceState, reapply: bool) {
    {
        let state = app.state::<Mutex<AppearanceState>>();
        let mut current = state.lock().unwrap();
        if !reapply && *current == next {
            return;
        }
        *current = next;
    }
    refresh_panel_appearances(app);
    if let Some(window) = app.get_webview_window("main") {
        window_manager::apply_window_vibrancy(app, &window);
    }
    capsule::refresh_appearance(app, next.effective);
    if let Err(error) = app.emit("appearance-changed", next) {
        crate::error!("[appearance] Could not publish appearance: {error}");
    }
}

fn refresh_panel_appearances(app: &AppHandle) {
    let appearance = native_application().effectiveAppearance();
    for label in ["main", capsule::LABEL] {
        let Some(window) = app.get_webview_window(label) else {
            continue;
        };
        let Ok(pointer) = window.ns_window() else {
            continue;
        };
        let window = unsafe { &*pointer.cast::<NSWindow>() };
        if window.effectiveAppearance().name() != appearance.name() {
            // AppKit can keep the panel's previous inherited appearance.
            // Refresh the view tree, then restore inheritance from NSApplication.
            window.setAppearance(Some(&appearance));
            window.setAppearance(None);
        }
    }
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = AppHandle]
    struct SystemAppearanceObserver;

    impl SystemAppearanceObserver {
        #[unsafe(method(systemAppearanceChanged:))]
        fn system_appearance_changed(&self, _notification: &NSNotification) {
            let app = self.ivars();
            if *app.state::<SettingsState>().appearance.lock().unwrap() != AppearancePreference::System {
                return;
            }
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                // AppKit can update effectiveAppearance after delivering the notification.
                tokio::time::sleep(Duration::from_millis(40)).await;
                let handle = app.clone();
                if let Err(error) = app.run_on_main_thread(move || refresh_system_theme(&handle)) {
                    crate::error!("[appearance] Could not schedule appearance refresh: {error}");
                }
            });
        }
    }
);

fn register_observer(app: &AppHandle) {
    let mtm = MainThreadMarker::new().expect("Appearance requires the main thread");
    let allocated = SystemAppearanceObserver::alloc(mtm).set_ivars(app.clone());
    let observer: objc2::rc::Retained<SystemAppearanceObserver> =
        unsafe { msg_send![super(allocated), init] };
    unsafe {
        NSDistributedNotificationCenter::defaultCenter()
            .addObserver_selector_name_object_suspensionBehavior(
                &observer,
                sel!(systemAppearanceChanged:),
                Some(&NSString::from_str(
                    "AppleInterfaceThemeChangedNotification",
                )),
                None,
                NSNotificationSuspensionBehavior::DeliverImmediately,
            );
    }
    // The notification observer lives as long as the application.
    let _ = objc2::rc::Retained::into_raw(observer);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferences_round_trip_and_unknown_settings_follow_system() {
        for (setting, preference) in [
            ("system", AppearancePreference::System),
            ("light", AppearancePreference::Light),
            ("dark", AppearancePreference::Dark),
        ] {
            assert_eq!(AppearancePreference::from_setting(setting), preference);
            assert_eq!(preference.as_setting(), setting);
        }
        for setting in ["unknown", "", "  ", " dark ", "Dark"] {
            assert_eq!(
                AppearancePreference::from_setting(setting),
                AppearancePreference::System
            );
        }
    }

    #[test]
    fn only_the_system_preference_follows_system_appearance() {
        for system_is_dark in [false, true] {
            assert_eq!(
                resolve_effective_theme(AppearancePreference::Light, system_is_dark),
                EffectiveTheme::Light
            );
            assert_eq!(
                resolve_effective_theme(AppearancePreference::Dark, system_is_dark),
                EffectiveTheme::Dark
            );
        }
        assert_eq!(
            resolve_effective_theme(AppearancePreference::System, false),
            EffectiveTheme::Light
        );
        assert_eq!(
            resolve_effective_theme(AppearancePreference::System, true),
            EffectiveTheme::Dark
        );
    }
}
