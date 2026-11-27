/// When true, `ExitRequested` is allowed to tear down the process (tray Quit etc.).
pub static ALLOW_APP_EXIT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn request_app_exit(app: &tauri::AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = crate::services::paste_queue::stop_sequential_session(app.clone()).await {
            crate::error!("Restoring the clipboard before exit: {error}");
        }
        ALLOW_APP_EXIT.store(true, std::sync::atomic::Ordering::SeqCst);
        app.exit(0);
    });
}
