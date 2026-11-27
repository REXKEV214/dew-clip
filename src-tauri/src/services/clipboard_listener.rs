#[cfg(target_os = "macos")]
fn has_pending_change(
    last: &mut isize,
    current: isize,
    monitor: &mut crate::global_state::MonitorPause,
) -> bool {
    if let Some(discard_through) = monitor.discard_through.take() {
        *last = discard_through;
    }
    if monitor.is_user_paused() {
        *last = current;
    }
    !monitor.is_paused() && current != *last
}

#[cfg(target_os = "macos")]
pub fn listen_clipboard<T: Send + 'static>(
    mut snapshot: impl FnMut(isize) -> Result<Option<T>, ()> + Send + 'static,
    mut capture: impl FnMut(T) + Send + 'static,
) {
    use crate::global_state::MONITOR_PAUSE;
    use crate::infrastructure::macos_api::clipboard::change_count;
    use std::sync::mpsc;
    use std::time::Duration;

    let (sender, receiver) = mpsc::channel();
    let mut last = change_count();
    std::thread::spawn(move || {
        for payload in receiver {
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| capture(payload))).is_err()
            {
                eprintln!(">>> [CLIPBOARD] capture panicked; continuing");
            }
        }
    });

    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_millis(80));
        loop {
            let observed = {
                let mut monitor = MONITOR_PAUSE.lock().unwrap();
                let current = change_count();
                if !has_pending_change(&mut last, current, &mut monitor) {
                    break;
                }
                current
            };
            // Some sources add formats immediately after declaring the new value.
            std::thread::sleep(Duration::from_millis(8));
            // Recheck after the delay and serialize the pasteboard read with a user pause.
            let mut monitor = MONITOR_PAUSE.lock().unwrap();
            let current = change_count();
            if !has_pending_change(&mut last, current, &mut monitor) {
                break;
            }
            if current != observed {
                continue;
            }
            match snapshot(current) {
                Ok(payload) => {
                    last = current;
                    if let Some(payload) = payload {
                        if sender.send(payload).is_err() {
                            return;
                        }
                    }
                    break;
                }
                // A newer change replaced the data before it could be queued.
                Err(()) => continue,
            }
        }
    });
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::has_pending_change;
    use crate::global_state::{MonitorPause, PAUSE_BATCH_PASTE, PAUSE_HOTKEY_RECORDING, PAUSE_USER};

    #[test]
    fn clipboard_change_remains_pending_until_monitor_resumes() {
        let mut last = 10;
        let mut monitor = MonitorPause::default();
        monitor.pause(PAUSE_BATCH_PASTE);
        assert!(!has_pending_change(&mut last, 12, &mut monitor));
        monitor.resume(PAUSE_BATCH_PASTE);
        assert!(has_pending_change(&mut last, 12, &mut monitor));
        last = 12;
        assert!(!has_pending_change(&mut last, 12, &mut monitor));
    }

    #[test]
    fn resume_discards_even_a_copy_between_polls() {
        let mut last = 10;
        let mut monitor = MonitorPause::default();
        monitor.pause(PAUSE_USER);
        assert!(!has_pending_change(&mut last, 11, &mut monitor));
        monitor.finish_user_pause(12);
        assert!(!has_pending_change(&mut last, 12, &mut monitor));
        assert!(has_pending_change(&mut last, 13, &mut monitor));
    }

    #[test]
    fn user_pause_discards_privately_while_other_owners_keep_their_hold() {
        let mut last = 10;
        let mut monitor = MonitorPause::default();
        monitor.pause(PAUSE_HOTKEY_RECORDING);
        monitor.pause(PAUSE_BATCH_PASTE);
        monitor.pause(PAUSE_USER);
        monitor.finish_user_pause(12);
        assert!(!has_pending_change(&mut last, 13, &mut monitor));
        monitor.resume(PAUSE_HOTKEY_RECORDING);
        assert!(!has_pending_change(&mut last, 13, &mut monitor));
        monitor.resume(PAUSE_BATCH_PASTE);
        assert!(!has_pending_change(&mut last, 12, &mut monitor));
        assert!(has_pending_change(&mut last, 13, &mut monitor));
    }
}

