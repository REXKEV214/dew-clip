use crate::app_state::{PasteQueue, SequentialSessionKind};
use crate::database::DbState;
use crate::infrastructure::repository::settings_repo::SettingsRepository;
use crate::services::recording_pause::{self, RecordingPauseStatus};
use crate::services::updater::UpdateState;
use serde::Deserialize;
use std::sync::OnceLock;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{App, AppHandle, Emitter, Manager};

pub const TRAY_ID: &str = "main_tray";
const ICON: &[u8] = include_bytes!("../../icons/tray-icon.png");

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Labels {
    open: String,
    quit: String,
    pause: String,
    five_minutes: String,
    fifteen_minutes: String,
    one_hour: String,
    until_resumed: String,
    resume: String,
    resume_timed: String,
    paused_tooltip: String,
    sequential_start: String,
    sequential_end: String,
    update_available: String,
}

impl Labels {
    fn resume(&self, minutes: Option<i64>) -> String {
        match minutes {
            Some(minutes) => self.resume_timed.replace("{min}", &minutes.to_string()),
            None => self.resume.clone(),
        }
    }

    fn tooltip(&self, paused: bool) -> &str {
        if paused {
            &self.paused_tooltip
        } else {
            "DewClip"
        }
    }

    fn sequential(&self, kind: SequentialSessionKind, count: usize) -> (&'static str, String) {
        if kind == SequentialSessionKind::Inactive {
            ("start_sequential", self.sequential_start.clone())
        } else {
            (
                "end_sequential",
                self.sequential_end.replace("{count}", &count.to_string()),
            )
        }
    }

    fn update(&self, version: Option<&str>) -> Option<String> {
        version.map(|version| self.update_available.replace("{version}", version))
    }
}

fn labels(english: bool) -> &'static Labels {
    #[derive(Deserialize)]
    struct Locales {
        en: Labels,
        zh: Labels,
    }
    static LABELS: OnceLock<Locales> = OnceLock::new();
    let locales = LABELS.get_or_init(|| {
        let source = include_str!("../../../src/locales.ts");
        let (_, labels) = source
            .split_once("/* native tray labels */")
            .expect("tray labels start");
        let (json, _) = labels
            .split_once("/* end native tray labels */")
            .expect("tray labels end");
        serde_json::from_str(json).expect("tray labels must be JSON")
    });
    if english {
        &locales.en
    } else {
        &locales.zh
    }
}

fn resolve_tray_english(app: &AppHandle) -> tauri::Result<bool> {
    let saved = app
        .state::<DbState>()
        .settings_repo
        .get("app.language")
        .map_err(|error| tauri::Error::Anyhow(error.into()))?;
    Ok(match saved.as_deref() {
        Some("en") => true,
        Some("zh") => false,
        _ => {
            #[cfg(target_os = "macos")]
            return Ok(!objc2_foundation::NSLocale::preferredLanguages()
                .firstObject()
                .is_some_and(|lang| lang.to_string().to_ascii_lowercase().starts_with("zh")));
            #[cfg(not(target_os = "macos"))]
            true
        }
    })
}

fn tray_icon(paused: bool) -> tauri::Result<Image<'static>> {
    let icon = Image::from_bytes(ICON)?;
    if !paused {
        return Ok(icon);
    }
    let mut rgba = icon.rgba().to_vec();
    for pixel in rgba.chunks_exact_mut(4) {
        pixel[3] /= 2;
    }
    Ok(Image::new_owned(rgba, icon.width(), icon.height()))
}

fn rebuild_tray(app: &AppHandle, tray: &TrayIcon) -> tauri::Result<()> {
    let labels = labels(resolve_tray_english(app)?);
    let status: RecordingPauseStatus = recording_pause::get_recording_pause_state(app.state());
    let open = MenuItem::with_id(app, "show", &labels.open, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", &labels.quit, true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &PredefinedMenuItem::separator(app)?])?;
    if status.paused {
        let remaining = status.minutes_left(recording_pause::wall_clock_ms());
        menu.append(&MenuItem::with_id(
            app,
            "resume_recording",
            labels.resume(remaining),
            true,
            None::<&str>,
        )?)?;
    } else {
        let pause = Submenu::new(app, &labels.pause, true)?;
        for (id, label) in [
            ("pause_five_minutes", &labels.five_minutes),
            ("pause_fifteen_minutes", &labels.fifteen_minutes),
            ("pause_one_hour", &labels.one_hour),
            ("pause_until_resumed", &labels.until_resumed),
        ] {
            pause.append(&MenuItem::with_id(app, id, label, true, None::<&str>)?)?;
        }
        menu.append(&pause)?;
    }
    let (id, label) = {
        let queue = app.state::<PasteQueue>();
        let queue = queue.0.lock().unwrap();
        labels.sequential(queue.kind, queue.items.len())
    };
    menu.append(&MenuItem::with_id(app, id, label, true, None::<&str>)?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    if let Some(label) = labels.update(app.state::<UpdateState>().available_version().as_deref()) {
        menu.append(&MenuItem::with_id(app, "show_update", label, true, None::<&str>)?)?;
    }
    menu.append(&quit)?;
    tray.set_menu(Some(menu))?;
    #[cfg(target_os = "macos")]
    tray.set_icon_with_as_template(Some(tray_icon(status.paused)?), true)?;
    #[cfg(not(target_os = "macos"))]
    tray.set_icon(Some(tray_icon(status.paused)?))?;
    tray.set_tooltip(Some(labels.tooltip(status.paused)))
}

pub fn setup_tray(app: &App, hide_tray: bool) -> tauri::Result<()> {
    let tray = TrayIconBuilder::with_id(TRAY_ID)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            let result = match event.id.as_ref() {
                "show" => {
                    let _ = crate::app::window_manager::focus_clipboard_window(app.clone());
                    return;
                }
                "quit" => {
                    crate::app::system::request_app_exit(app);
                    return;
                }
                "show_update" => {
                    if crate::app::window_manager::focus_clipboard_window(app.clone()).is_ok() {
                        let _ = app.emit_to("main", "open-update-dialog", ());
                    }
                    return;
                }
                "resume_recording" => {
                    recording_pause::resume_recording(app.clone());
                    return;
                }
                "start_sequential" => {
                    tauri::async_runtime::spawn(
                        crate::services::paste_queue::start_sequential_session(app.clone()),
                    );
                    return;
                }
                "end_sequential" => {
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Err(error) =
                            crate::services::paste_queue::stop_sequential_session(app).await
                        {
                            crate::error!("Could not end Sequential Paste: {error}");
                        }
                    });
                    return;
                }
                "pause_five_minutes" => recording_pause::pause_recording(app.clone(), Some(5)),
                "pause_fifteen_minutes" => recording_pause::pause_recording(app.clone(), Some(15)),
                "pause_one_hour" => recording_pause::pause_recording(app.clone(), Some(60)),
                "pause_until_resumed" => recording_pause::pause_recording(app.clone(), None),
                _ => return,
            };
            if let Err(error) = result {
                crate::error!("Could not pause recording: {error}");
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                ..
            } = event
            {
                let _ =
                    crate::app::window_manager::focus_clipboard_window(tray.app_handle().clone());
            }
        })
        .build(app)?;
    rebuild_tray(app.handle(), &tray)?;
    tray.set_visible(!hide_tray)?;
    app.manage(tray);
    Ok(())
}

pub fn refresh_tray(app: &AppHandle) {
    let handle = app.clone();
    if let Err(error) = app.run_on_main_thread(move || {
        if let Some(tray) = handle.tray_by_id(TRAY_ID) {
            if let Err(error) = rebuild_tray(&handle, &tray) {
                crate::error!("Could not refresh menu bar: {error}");
            }
        }
    }) {
        crate::error!("Could not schedule menu bar refresh: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_labels_cover_both_languages_and_pause_states() {
        let en = labels(true);
        assert_eq!(
            [
                &*en.open,
                &*en.pause,
                &*en.five_minutes,
                &*en.fifteen_minutes,
                &*en.one_hour,
                &*en.until_resumed,
                &*en.quit
            ],
            [
                "Open DewClip",
                "Pause Recording",
                "For 5 Minutes",
                "For 15 Minutes",
                "For 1 Hour",
                "Until Resumed",
                "Quit"
            ]
        );
        assert_eq!(en.resume(None), "Resume Recording");
        assert_eq!(en.resume(Some(5)), "Resume Recording (~5 min left)");
        assert_eq!(en.tooltip(false), "DewClip");
        assert_eq!(en.tooltip(true), "DewClip (Recording Paused)");
        let zh = labels(false);
        assert_eq!(
            [
                &*zh.open,
                &*zh.pause,
                &*zh.five_minutes,
                &*zh.fifteen_minutes,
                &*zh.one_hour,
                &*zh.until_resumed,
                &*zh.quit
            ],
            [
                "打开 DewClip",
                "暂停记录",
                "暂停 5 分钟",
                "暂停 15 分钟",
                "暂停 1 小时",
                "直到手动恢复",
                "退出"
            ]
        );
        assert_eq!(zh.resume(None), "恢复记录");
        assert_eq!(zh.resume(Some(5)), "恢复记录（约剩 5 分钟）");
        assert_eq!(zh.tooltip(false), "DewClip");
        assert_eq!(zh.tooltip(true), "DewClip（已暂停记录）");
    }

    #[test]
    fn paused_icon_uses_the_same_pixels_at_half_alpha() {
        let original = tray_icon(false).unwrap();
        let faded = tray_icon(true).unwrap();
        assert_eq!(
            (original.width(), original.height()),
            (faded.width(), faded.height())
        );
        for (normal, faded) in original
            .rgba()
            .chunks_exact(4)
            .zip(faded.rgba().chunks_exact(4))
        {
            assert_eq!(normal[..3], faded[..3]);
            assert_eq!(normal[3] / 2, faded[3]);
        }
    }

    #[test]
    fn tray_labels_show_the_remaining_items_in_both_session_kinds() {
        let en = labels(true);
        let zh = labels(false);
        assert_eq!(
            en.sequential(SequentialSessionKind::Inactive, 0).1,
            "Start Sequential Paste"
        );
        assert_eq!(
            zh.sequential(SequentialSessionKind::Inactive, 0).1,
            "开始顺序粘贴"
        );
        for kind in [
            SequentialSessionKind::Recording,
            SequentialSessionKind::OneShot,
        ] {
            assert_eq!(
                en.sequential(kind, 2).1,
                "End Sequential Paste (2 remaining)"
            );
            assert_eq!(zh.sequential(kind, 2).1, "结束顺序粘贴（剩 2 项）");
        }
    }

    #[test]
    fn update_menu_item_uses_the_language_and_requires_an_available_version() {
        assert_eq!(labels(true).update(Some("1.2.0")).as_deref(), Some("Update Available (v1.2.0)…"));
        assert_eq!(labels(false).update(Some("1.2.0")).as_deref(), Some("发现新版本 v1.2.0…"));
        assert_eq!(labels(true).update(None), None);
        assert_eq!(labels(false).update(None), None);
    }
}
