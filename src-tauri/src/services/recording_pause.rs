use crate::app::tray;
use crate::error::{AppError, AppResult};
use crate::global_state::{MONITOR_PAUSE, PAUSE_USER};
use serde::Serialize;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

const MINUTE_MS: i64 = 60_000;

#[derive(Clone, Copy, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordingPauseStatus {
    pub paused: bool,
    pub resume_at: Option<i64>,
}

impl RecordingPauseStatus {
    pub fn minutes_left(self, now: i64) -> Option<i64> {
        self.resume_at
            .map(|end| ((end - now).max(1) + MINUTE_MS - 1) / MINUTE_MS)
    }
}

#[derive(Default)]
struct UserPauseState {
    status: RecordingPauseStatus,
    generation: u64,
}

impl UserPauseState {
    fn pause(&mut self, now: i64, minutes: Option<i64>) -> u64 {
        self.generation += 1;
        self.status = RecordingPauseStatus {
            paused: true,
            resume_at: minutes.map(|m| now + m * MINUTE_MS),
        };
        self.generation
    }

    fn is_expired(&self, now: i64) -> bool {
        self.status.resume_at.is_some_and(|end| end <= now)
    }

    fn resume(&mut self) {
        self.status = RecordingPauseStatus::default();
        self.generation += 1;
    }

    fn next_check(&self, generation: u64, now: i64) -> Option<Duration> {
        if self.generation != generation {
            return None;
        }
        self.status
            .resume_at
            .map(|end| Duration::from_millis((end - now).clamp(1, 30_000) as u64))
    }
}

#[derive(Default)]
pub struct RecordingPause(Mutex<UserPauseState>);

#[tauri::command]
pub fn get_recording_pause_state(state: State<'_, RecordingPause>) -> RecordingPauseStatus {
    state.0.lock().unwrap().status
}

pub fn wall_clock_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn emit(app: &AppHandle, status: RecordingPauseStatus) {
    if let Err(error) = app.emit("recording-pause-changed", status) {
        crate::error!("Could not publish recording pause state: {error}");
    }
}

#[tauri::command]
pub fn pause_recording(app: AppHandle, duration_minutes: Option<i64>) -> AppResult<()> {
    if !matches!(duration_minutes, None | Some(5 | 15 | 60)) {
        return Err(AppError::Validation(
            "invalid recording pause duration".into(),
        ));
    }
    let state = app.state::<RecordingPause>();
    let generation = {
        let mut pause = state.0.lock().unwrap();
        MONITOR_PAUSE.lock().unwrap().pause(PAUSE_USER);
        let generation = pause.pause(wall_clock_ms(), duration_minutes);
        emit(&app, pause.status);
        generation
    };
    tray::refresh_tray(&app);

    if duration_minutes.is_some() {
        tauri::async_runtime::spawn(async move {
            loop {
                let delay = app
                    .state::<RecordingPause>()
                    .0
                    .lock()
                    .unwrap()
                    .next_check(generation, wall_clock_ms());
                let Some(delay) = delay else { break };
                tokio::time::sleep(delay).await;
                if !check_pause(&app, Some(generation), false) {
                    break;
                }
            }
        });
    }
    Ok(())
}

#[tauri::command]
pub fn resume_recording(app: AppHandle) {
    check_pause(&app, None, true);
}

pub fn check_expired_pause(app: &AppHandle) {
    check_pause(app, None, false);
}

fn check_pause(app: &AppHandle, generation: Option<u64>, force_resume: bool) -> bool {
    let Some(state) = app.try_state::<RecordingPause>() else {
        return false;
    };
    let mut pause = state.0.lock().unwrap();
    if !pause.status.paused || generation.is_some_and(|generation| generation != pause.generation) {
        return false;
    }
    if force_resume || pause.is_expired(wall_clock_ms()) {
        #[cfg(target_os = "macos")]
        {
            let mut monitor = MONITOR_PAUSE.lock().unwrap();
            monitor.finish_user_pause(crate::infrastructure::macos_api::clipboard::change_count());
        }
        #[cfg(not(target_os = "macos"))]
        MONITOR_PAUSE.lock().unwrap().resume(PAUSE_USER);
        pause.resume();
        emit(app, pause.status);
    }
    let still_paused = pause.status.paused;
    drop(pause);
    tray::refresh_tray(app);
    still_paused
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indefinite_pause_has_no_deadline_and_can_resume() {
        let mut pause = UserPauseState::default();
        let generation = pause.pause(1_000, None);
        assert!(pause.status.paused);
        assert!(!pause.is_expired(i64::MAX));
        assert_eq!(pause.next_check(generation, 1_000), None);
        pause.resume();
        assert!(!pause.status.paused);
    }

    #[test]
    fn wall_clock_expiry_and_replacement_ignore_the_old_timer() {
        let mut pause = UserPauseState::default();
        let old = pause.pause(1_000, Some(5));
        assert!(!pause.is_expired(300_999));
        assert!(pause.is_expired(301_000));
        assert_eq!(
            pause.next_check(old, 300_999),
            Some(Duration::from_millis(1))
        );
        let current = pause.pause(5_000, Some(15));
        assert_eq!(pause.next_check(old, 301_000), None);
        assert!(!pause.is_expired(301_000));
        assert!(pause.is_expired(905_000));
        pause.resume();
        assert_eq!(pause.next_check(current, 905_000), None);
    }
}
