use crate::app::capsule::{self, SequentialNotice};
use crate::app::commands::hotkey_cmd::{
    registered_sequential_hotkey, sync_registered_hotkeys_on_main_thread,
};
use crate::app_state::{
    OneShotClipboard, PasteQueue, PasteQueueState, SequentialItem, SequentialSessionKind,
    SessionHistory, SettingsState,
};
use crate::database::DbState;
use crate::domain::models::{ClipboardEntry, EntryIdentity};
use crate::error::{AppError, AppResult};
use crate::infrastructure::macos_api::{clipboard as native, permissions, window};
use crate::infrastructure::repository::clipboard_repo::{ClipboardRepository, PasteHistoryChange};
use crate::services::clipboard_ops::{self, PasteTarget, PASTE_LOCK};
use crate::services::recording_pause::wall_clock_ms;
use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, State};

const IDLE_TIMEOUT_MS: i64 = 15 * 60 * 1000;
const IDLE_CHECK_INTERVAL_MS: i64 = 30_000;
const MODIFIER_WAIT: Duration = Duration::from_millis(400);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SequentialQueueSnapshot {
    pub kind: SequentialSessionKind,
    pub items: Vec<SequentialItem>,
    pub pasted_in_round: usize,
    pub round_total: usize,
}

impl PasteQueueState {
    pub fn has_items(&self) -> bool {
        self.kind != SequentialSessionKind::Inactive && !self.items.is_empty()
    }

    fn snapshot(&self) -> SequentialQueueSnapshot {
        SequentialQueueSnapshot {
            kind: self.kind,
            items: self.items.iter().cloned().collect(),
            pasted_in_round: self.pasted_in_round,
            round_total: self.round_total,
        }
    }

    fn clear(&mut self) {
        self.items.clear();
        self.round_total = 0;
        self.pasted_in_round = 0;
        self.last_paste_at = None;
    }

    fn end(&mut self) {
        self.clear();
        self.kind = SequentialSessionKind::Inactive;
    }

    fn push(&mut self, entry: &ClipboardEntry) {
        self.next_item_id += 1;
        let kind = entry.content_type.as_str();
        let preview = match kind {
            "file" | "video" => entry
                .content
                .lines()
                .find_map(|path| std::path::Path::new(path.trim()).file_name())
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
            "image" => String::new(),
            _ => capsule::first_line_preview(&entry.preview),
        };
        self.items.push_back(SequentialItem {
            queue_id: self.next_item_id,
            identity: EntryIdentity::from_entry(entry),
            content_type: entry.content_type.clone(),
            preview,
            media_path: (matches!(kind, "image" | "video") && !entry.content.starts_with("data:"))
                .then(|| entry.content.clone()),
            timestamp: entry.timestamp,
        });
        self.round_total += 1;
    }

    fn collect(&mut self, entry: &ClipboardEntry, captured_at: Instant) -> bool {
        let new_round = self
            .last_paste_at
            .is_some_and(|pasted_at| captured_at >= pasted_at);
        if new_round {
            self.clear();
        }
        self.push(entry);
        new_round
    }

    fn consume_head(&mut self, pasted_at: Option<Instant>) {
        self.items.pop_front();
        if let Some(pasted_at) = pasted_at {
            self.last_paste_at = Some(pasted_at);
            self.pasted_in_round += 1;
        } else {
            self.round_total -= 1;
        }
    }

    fn finish_step(
        &mut self,
        source: EntryIdentity,
        pasted_at: Option<Instant>,
        deleted_from_history: bool,
        skip_hotkey: Option<String>,
    ) -> SequentialNotice {
        self.consume_head(pasted_at);
        if deleted_from_history {
            self.remove_where(|item| item.identity == source);
        }
        if pasted_at.is_none() {
            let remaining = self.items.len();
            SequentialNotice::Skipped {
                remaining,
                hotkey: skip_hotkey.filter(|_| remaining > 0),
                round_ended: remaining == 0 && self.kind == SequentialSessionKind::OneShot,
            }
        } else if !self.items.is_empty() {
            SequentialNotice::Pasted {
                current: self.pasted_in_round,
                total: self.round_total,
            }
        } else if self.kind == SequentialSessionKind::OneShot {
            SequentialNotice::Done {
                total: self.pasted_in_round,
            }
        } else {
            SequentialNotice::RoundDone {
                total: self.pasted_in_round,
            }
        }
    }

    fn remove_where(&mut self, matches: impl Fn(&SequentialItem) -> bool) -> bool {
        let before = self.items.len();
        self.items.retain(|item| !matches(item));
        self.round_total -= before - self.items.len();
        before != self.items.len()
    }

    fn reorder(&mut self, ids: &[u64]) -> bool {
        if ids.len() != self.items.len() {
            return false;
        }
        let mut by_id: HashMap<_, _> = self
            .items
            .iter()
            .map(|item| (item.queue_id, item))
            .collect();
        let ordered: Option<VecDeque<_>> = ids.iter().map(|id| by_id.remove(id).cloned()).collect();
        if let Some(ordered) = ordered {
            self.items = ordered;
            true
        } else {
            false
        }
    }

    fn end_if_finished(&mut self) -> Option<OneShotClipboard> {
        if self.kind == SequentialSessionKind::OneShot && self.items.is_empty() {
            self.end();
        }
        if self.kind == SequentialSessionKind::Inactive {
            self.recording_since = None;
            self.one_shot_clipboard.take()
        } else {
            None
        }
    }

    fn expire_if_idle(&mut self, generation: Option<u64>, now: i64) -> Option<()> {
        if generation.is_some_and(|expected| expected != self.idle_generation)
            || self.idle_deadline_ms.is_none_or(|deadline| now < deadline)
        {
            return None;
        }
        self.end();
        Some(())
    }

    fn idle_check_delay(&self, generation: u64, now: i64) -> Option<Duration> {
        if self.idle_generation != generation {
            return None;
        }
        self.idle_deadline_ms.map(|deadline| {
            Duration::from_millis((deadline - now).clamp(0, IDLE_CHECK_INTERVAL_MS) as u64)
        })
    }
}

#[tauri::command]
pub fn get_sequential_queue(state: State<'_, PasteQueue>) -> SequentialQueueSnapshot {
    state.0.lock().unwrap().snapshot()
}

/// Queue mutations are serialized by PASTE_LOCK, including capture and history deletion.
fn update<R>(
    app: &AppHandle,
    change: impl FnOnce(&mut PasteQueueState) -> Option<R>,
) -> Option<(R, Option<OneShotClipboard>)> {
    let state = app.state::<PasteQueue>();
    let mut queue = state.0.lock().unwrap();
    let previous_kind = queue.kind;
    let had_items = queue.has_items();
    let result = change(&mut queue)?;
    let clipboard = queue.end_if_finished();
    let snapshot = queue.snapshot();
    let has_items = queue.has_items();
    let idle = snapshot.kind == SequentialSessionKind::Recording && !has_items;
    queue.idle_deadline_ms = idle.then(|| wall_clock_ms() + IDLE_TIMEOUT_MS);
    queue.idle_generation += 1;
    let generation = queue.idle_generation;
    drop(queue);

    if had_items != has_items {
        sync_registered_hotkeys_on_main_thread(app);
    }
    crate::app::tray::refresh_tray(app);
    let kind = snapshot.kind;
    let _ = app.emit("sequential-queue-changed", snapshot);
    if kind == SequentialSessionKind::Inactive && previous_kind != kind {
        let _ = app.emit("queue-finished", ());
    }
    if idle {
        start_idle_timer(app.clone(), generation);
    }
    Some((result, clipboard))
}

fn start_idle_timer(app: AppHandle, generation: u64) {
    tauri::async_runtime::spawn(async move {
        loop {
            let delay = {
                let state = app.state::<PasteQueue>();
                let queue = state.0.lock().unwrap();
                queue.idle_check_delay(generation, wall_clock_ms())
            };
            let Some(delay) = delay else { return };
            tokio::time::sleep(delay).await;
            let _guard = PASTE_LOCK.lock().await;
            if update(&app, |queue| {
                queue.expire_if_idle(Some(generation), wall_clock_ms())
            })
            .is_some()
            {
                return;
            }
        }
    });
}

pub async fn check_idle_after_wake(app: AppHandle) {
    let _guard = PASTE_LOCK.lock().await;
    if app.try_state::<PasteQueue>().is_some() {
        update(&app, |queue| queue.expire_if_idle(None, wall_clock_ms()));
    }
}

async fn restore(clipboard: Option<OneShotClipboard>) -> AppResult<()> {
    let Some(OneShotClipboard {
        before,
        last_write: Some(count),
    }) = clipboard
    else {
        return Ok(());
    };
    tokio::time::sleep(Duration::from_millis(clipboard_ops::BATCH_RESTORE_DELAY_MS)).await;
    if native::change_count() == count && native::is_own_change(count) {
        native::restore_pasteboard(&before).map_err(AppError::Internal)?;
    }
    Ok(())
}

fn get_entry(app: &AppHandle, identity: EntryIdentity) -> AppResult<Option<ClipboardEntry>> {
    let entry = if identity.id < 0 {
        app.state::<SessionHistory>()
            .0
            .lock()
            .unwrap()
            .iter()
            .find(|entry| identity.matches(entry))
            .cloned()
    } else {
        app.state::<DbState>()
            .repo
            .get_entry_by_id(identity.id)?
            .filter(|entry| identity.matches(entry))
    };
    Ok(entry.map(|mut entry| {
        entry.content =
            clipboard_ops::with_current_entry_file(app, &entry.content_type, entry.content);
        entry
    }))
}

fn require_entry(app: &AppHandle, identity: EntryIdentity) -> AppResult<ClipboardEntry> {
    get_entry(app, identity)?.ok_or_else(|| AppError::Validation("content_unavailable".into()))
}

#[tauri::command]
pub async fn start_sequential_session(app: AppHandle) {
    let _guard = PASTE_LOCK.lock().await;
    update(&app, |queue| {
        if queue.kind != SequentialSessionKind::Inactive {
            return None;
        }
        queue.kind = SequentialSessionKind::Recording;
        queue.recording_since = Some(Instant::now());
        Some(())
    });
}

#[tauri::command]
pub async fn start_sequential_from_entries(
    app: AppHandle,
    entries: Vec<EntryIdentity>,
) -> AppResult<()> {
    if entries.is_empty() {
        return Ok(());
    }
    let _guard = PASTE_LOCK.lock().await;
    let entries = entries
        .into_iter()
        .map(|identity| require_entry(&app, identity))
        .collect::<AppResult<Vec<_>>>()?;
    if let Some(((), clipboard)) = update(&app, |queue| {
        (queue.kind == SequentialSessionKind::OneShot).then(|| queue.end())
    }) {
        restore(clipboard).await?;
    }
    let before = clipboard_ops::capture_clipboard_snapshot()
        .ok_or_else(|| AppError::Validation("clipboard_snapshot_failed".into()))?;

    if !crate::WINDOW_PINNED.load(Ordering::Relaxed) {
        clipboard_ops::handle_window_focus_for_paste(&app).await?;
        crate::NAVIGATION_ENABLED.store(false, Ordering::Relaxed);
    }
    update(&app, |queue| {
        queue.clear();
        queue.kind = SequentialSessionKind::OneShot;
        queue.recording_since = None;
        queue.one_shot_clipboard = Some(OneShotClipboard {
            before,
            last_write: None,
        });
        for entry in &entries {
            queue.push(entry);
        }
        Some(())
    });
    Ok(())
}

#[tauri::command]
pub async fn stop_sequential_session(app: AppHandle) -> AppResult<()> {
    let _guard = PASTE_LOCK.lock().await;
    if let Some(((), clipboard)) = update(&app, |queue| {
        if queue.kind == SequentialSessionKind::Inactive {
            return None;
        }
        queue.end();
        Some(())
    }) {
        restore(clipboard).await?;
    }
    Ok(())
}

#[tauri::command]
pub async fn enqueue_sequential_entry(app: AppHandle, entry: EntryIdentity) -> AppResult<()> {
    let _guard = PASTE_LOCK.lock().await;
    let entry = require_entry(&app, entry)?;
    update(&app, |queue| {
        if queue.kind == SequentialSessionKind::Inactive {
            return None;
        }
        queue.push(&entry);
        Some(())
    })
    .ok_or_else(|| AppError::Validation("sequential_inactive".into()))?;
    Ok(())
}

#[tauri::command]
pub async fn remove_sequential_entry(app: AppHandle, queue_id: u64) -> AppResult<()> {
    let _guard = PASTE_LOCK.lock().await;
    if let Some(((), clipboard)) = update(&app, |queue| {
        queue
            .remove_where(|item| item.queue_id == queue_id)
            .then_some(())
    }) {
        restore(clipboard).await?;
    }
    Ok(())
}

#[tauri::command]
pub async fn reorder_sequential_queue(app: AppHandle, queue_ids: Vec<u64>) -> AppResult<()> {
    let _guard = PASTE_LOCK.lock().await;
    update(&app, |queue| {
        (queue.has_items() && queue.reorder(&queue_ids)).then_some(())
    })
    .ok_or_else(|| AppError::Validation("sequential_queue_changed".into()))?;
    Ok(())
}

#[tauri::command]
pub async fn clear_sequential_queue(app: AppHandle) -> AppResult<()> {
    let _guard = PASTE_LOCK.lock().await;
    if let Some(((), clipboard)) = update(&app, |queue| {
        if !queue.has_items() {
            return None;
        }
        queue.clear();
        Some(())
    }) {
        restore(clipboard).await?;
    }
    Ok(())
}

/// Callers hold PASTE_LOCK from the history change through queue reconciliation.
pub async fn remove_deleted_items(app: &AppHandle, matches: impl Fn(EntryIdentity) -> bool) {
    if let Some(((), clipboard)) = update(app, |queue| {
        queue
            .remove_where(|item| matches(item.identity))
            .then_some(())
    }) {
        if let Err(error) = restore(clipboard).await {
            crate::error!("Restoring the clipboard: {error}");
        }
    }
}

pub async fn prune_deleted_items(app: &AppHandle) -> AppResult<()> {
    let identities: Vec<_> = app
        .state::<PasteQueue>()
        .0
        .lock()
        .unwrap()
        .items
        .iter()
        .map(|item| item.identity)
        .collect();
    let mut missing = HashSet::new();
    for identity in identities {
        if get_entry(app, identity)?.is_none() {
            missing.insert(identity);
        }
    }
    remove_deleted_items(app, |identity| missing.contains(&identity)).await;
    Ok(())
}

pub async fn reconcile_history(
    app: &AppHandle,
    source: EntryIdentity,
    replacement: Option<EntryIdentity>,
    deleted_ids: &[i64],
) {
    if let Some(((), clipboard)) = update(app, |queue| {
        let mut changed = false;
        if let Some(saved) = replacement {
            for item in queue
                .items
                .iter_mut()
                .filter(|item| item.identity == source)
            {
                item.identity = saved;
                changed = true;
            }
        }
        let removed = queue.remove_where(|item| deleted_ids.contains(&item.identity.id));
        (changed || removed).then_some(())
    }) {
        if let Err(error) = restore(clipboard).await {
            crate::error!("Restoring the clipboard: {error}");
        }
    }
}

pub fn on_captured(
    app: &AppHandle,
    entry: &ClipboardEntry,
    captured_at: Instant,
    deleted_ids: &[i64],
) -> bool {
    let state = app.state::<PasteQueue>();
    if deleted_ids.is_empty() && state.0.lock().unwrap().kind != SequentialSessionKind::Recording {
        return false;
    }
    let _guard = PASTE_LOCK.blocking_lock();
    let identity = EntryIdentity::from_entry(entry);
    let recording = state
        .0
        .lock()
        .unwrap()
        .recording_since
        .is_some_and(|started| captured_at >= started);
    let still_exists = if recording && !deleted_ids.contains(&entry.id) {
        match get_entry(app, identity) {
            Ok(current) => current.is_some(),
            Err(error) => {
                crate::error!("Checking the captured sequential entry: {error}");
                false
            }
        }
    } else {
        false
    };
    let change = update(app, |queue| {
        let removed = queue.remove_where(|item| deleted_ids.contains(&item.identity.id));
        if still_exists {
            let new_round = queue.collect(entry, captured_at);
            Some(Some((queue.items.len(), new_round)))
        } else {
            removed.then_some(None)
        }
    });
    let Some((queued, clipboard)) = change else {
        return false;
    };
    if let Some(clipboard) = clipboard {
        if let Err(error) = tauri::async_runtime::block_on(restore(Some(clipboard))) {
            crate::error!("Restoring the clipboard: {error}");
        }
    }
    if let Some((position, new_round)) = queued {
        capsule::notify_sequential_queued(app, entry, position, new_round);
        return true;
    }
    false
}

async fn wait_for_modifiers() -> AppResult<()> {
    let deadline = Instant::now() + MODIFIER_WAIT;
    while !permissions::are_paste_modifiers_released() {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(AppError::Validation("sequential_release_modifiers".into()));
        }
        tokio::time::sleep(remaining.min(Duration::from_millis(15))).await;
    }
    Ok(())
}

#[tauri::command]
pub async fn paste_next_step(app: AppHandle) -> AppResult<()> {
    let target = PasteTarget::for_sequential_paste();
    let _guard = PASTE_LOCK.lock().await;
    let result = paste_next(&app, target).await;
    if let Err(error) = &result {
        let reason = match error {
            AppError::Validation(reason) if reason == "accessibility_permission_required" => {
                "permission"
            }
            AppError::Validation(reason) if reason == "sequential_release_modifiers" => "modifiers",
            _ => "paste",
        };
        capsule::notify_sequential(&app, SequentialNotice::Failed { reason }, None);
    }
    result
}

async fn paste_next(app: &AppHandle, target: PasteTarget) -> AppResult<()> {
    let state = app.state::<PasteQueue>();
    let Some(source) = state
        .0
        .lock()
        .unwrap()
        .items
        .front()
        .map(|item| item.identity)
    else {
        capsule::notify_sequential(app, SequentialNotice::Empty, None);
        return Ok(());
    };

    if let Err(error) = clipboard_ops::require_paste_permission(app) {
        if !window::is_clipboard_panel_visible() {
            permissions::prompt_accessibility_permission();
        }
        return Err(error);
    }

    let mut pasted_at = None;
    if let Some(entry) = get_entry(app, source)? {
        let kind = &entry.content_type;
        let content = &entry.content;
        if clipboard_ops::check_payload_files(content, kind).is_ok() {
            if matches!(target, PasteTarget::PreviousApp(_)) {
                clipboard_ops::handle_window_focus_for_paste(app).await?;
            }
            clipboard_ops::ensure_paste_target_frontmost(target).await?;
            wait_for_modifiers().await?;
            let before = native::change_count();
            let html = entry.html_content.as_deref();
            let prepared = clipboard_ops::prepare_clipboard_payload(
                content,
                kind,
                html,
                clipboard_ops::resolve_paste_with_format(content, kind, html, None),
            )
            .await;
            let after = native::change_count();
            if after != before && native::is_own_change(after) {
                if let Some(clipboard) = state.0.lock().unwrap().one_shot_clipboard.as_mut() {
                    clipboard.last_write = Some(after);
                }
            }
            match prepared {
                Ok(()) => {
                    clipboard_ops::ensure_paste_target_frontmost(target).await?;
                    if !permissions::are_paste_modifiers_released() {
                        return Err(AppError::Validation("sequential_release_modifiers".into()));
                    }
                    if native::change_count() != after || !native::is_own_change(after) {
                        return Err(AppError::Validation(
                            "clipboard_changed_during_paste".into(),
                        ));
                    }
                    clipboard_ops::send_paste_keystroke()?;
                    pasted_at = Some(Instant::now());
                }
                Err(error) if clipboard_ops::check_payload_files(content, kind).is_ok() => {
                    return Err(error)
                }
                Err(_) => {}
            }
        }
    }

    let pasted = pasted_at.is_some();
    let delete = pasted
        && app
            .state::<SettingsState>()
            .delete_after_paste
            .load(Ordering::Relaxed);
    let resolve_duplicate_before_notice = delete
        && state
            .0
            .lock()
            .unwrap()
            .items
            .iter()
            .skip(1)
            .any(|item| item.identity == source);
    // A database/session deletion can also remove the next queued occurrence.
    // Keep the target's read delay before deleting, then report the resulting queue.
    let history_before_notice = if resolve_duplicate_before_notice {
        tokio::time::sleep(Duration::from_millis(clipboard_ops::BATCH_STEP_DELAY_MS)).await;
        Some(clipboard_ops::record_paste_history(
            app,
            &app.state::<DbState>(),
            source,
            true,
            Some(false),
        ))
    } else {
        None
    };
    let deleted = matches!(
        &history_before_notice,
        Some(Ok(Some(PasteHistoryChange::Deleted)))
    );
    let skip_hotkey = if pasted {
        None
    } else {
        registered_sequential_hotkey(app)
    };
    let ((progress, next), clipboard) = update(app, |queue| {
        let progress = queue.finish_step(source, pasted_at, deleted, skip_hotkey);
        let next = queue
            .items
            .front()
            .map(|item| (item.content_type.clone(), item.preview.clone()));
        Some((progress, next))
    })
    .expect("the step consumes one queue item");
    capsule::notify_sequential(app, progress, next);

    let restoring = clipboard.is_some();
    if let Err(error) = restore(clipboard).await {
        crate::error!("Restoring the clipboard: {error}");
    }
    if pasted {
        if !restoring && history_before_notice.is_none() {
            tokio::time::sleep(Duration::from_millis(clipboard_ops::BATCH_STEP_DELAY_MS)).await;
        }
        let _ = app.emit("queue-item-pasted", source.id);
        let history_changed = match history_before_notice {
            Some(result) => result.map(|change| change == Some(PasteHistoryChange::Deleted)),
            None => {
                clipboard_ops::handle_post_paste_actions(
                    app,
                    &app.state::<DbState>(),
                    Some(source),
                    delete,
                    Some(false),
                )
                .await
            }
        };
        match history_changed {
            Ok(true) => {
                let _ = app.emit("clipboard-changed", ());
            }
            Ok(false) => {}
            Err(error) => crate::error!("Updating history after Sequential Paste: {error}"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: i64, text: &str) -> ClipboardEntry {
        serde_json::from_value(serde_json::json!({
            "id": id, "created_at": id, "content_type": "text", "content": text,
            "source_app": "Notes", "timestamp": id, "preview": text, "is_pinned": false
        }))
        .unwrap()
    }

    #[test]
    fn a_copy_after_a_paste_replaces_the_remaining_round() {
        let mut queue = PasteQueueState {
            kind: SequentialSessionKind::Recording,
            ..Default::default()
        };
        let copied = Instant::now();
        for entry in [item(1, "Name"), item(2, "Address"), item(3, "Email")] {
            assert!(!queue.collect(&entry, copied));
        }
        let pasted = copied + Duration::from_secs(1);
        queue.consume_head(Some(pasted));
        assert_eq!(queue.pasted_in_round, 1);
        assert_eq!(queue.round_total, 3);

        assert!(!queue.collect(&item(4, "City"), copied));
        assert_eq!(queue.items.len(), 3);
        assert!(queue.collect(&item(5, "Next name"), pasted + Duration::from_millis(1)));
        assert_eq!(
            queue
                .items
                .iter()
                .map(|item| item.identity.id)
                .collect::<Vec<_>>(),
            [5]
        );
        assert_eq!(queue.pasted_in_round, 0);
        assert_eq!(queue.round_total, 1);
    }

    #[test]
    fn a_one_shot_ends_with_its_last_item_while_recording_stays_active() {
        for kind in [
            SequentialSessionKind::OneShot,
            SequentialSessionKind::Recording,
        ] {
            let mut queue = PasteQueueState {
                kind,
                ..Default::default()
            };
            assert!(!queue.has_items());
            queue.push(&item(1, "First field"));
            assert!(queue.has_items());
            queue.consume_head(Some(Instant::now()));
            queue.end_if_finished();
            assert!(!queue.has_items());
            assert_eq!(
                queue.kind,
                if kind == SequentialSessionKind::OneShot {
                    SequentialSessionKind::Inactive
                } else {
                    kind
                }
            );
        }
    }

    #[test]
    fn a_skip_consumes_only_the_missing_item_without_counting_a_paste() {
        let mut queue = PasteQueueState {
            kind: SequentialSessionKind::Recording,
            ..Default::default()
        };
        let deleted = item(1, "Deleted");
        queue.push(&deleted);
        queue.push(&item(2, "Still queued"));
        let notice = queue.finish_step(
            EntryIdentity::from_entry(&deleted),
            None,
            false,
            Some("Alt+V".into()),
        );
        assert!(
            matches!(notice, SequentialNotice::Skipped { remaining: 1, hotkey: Some(key), round_ended: false } if key == "Alt+V")
        );
        assert_eq!(queue.items.front().unwrap().identity.id, 2);
        assert_eq!(queue.round_total, 1);
        assert_eq!(queue.pasted_in_round, 0);
        assert!(queue.last_paste_at.is_none());
    }

    #[test]
    fn skipping_the_last_item_drops_the_retry_and_reports_whether_the_session_ends() {
        let deleted = item(1, "Deleted");
        for (kind, ends) in [
            (SequentialSessionKind::OneShot, true),
            (SequentialSessionKind::Recording, false),
        ] {
            let mut queue = PasteQueueState {
                kind,
                ..Default::default()
            };
            queue.push(&deleted);
            let notice = queue.finish_step(
                EntryIdentity::from_entry(&deleted),
                None,
                false,
                Some("Alt+V".into()),
            );
            assert!(
                matches!(notice, SequentialNotice::Skipped { remaining: 0, hotkey: None, round_ended } if round_ended == ends)
            );
            queue.end_if_finished();
            assert!(!queue.has_items());
            assert_eq!(queue.kind == SequentialSessionKind::Inactive, ends);
        }
    }

    #[test]
    fn a_skip_without_a_registered_shortcut_only_reports_the_remaining_count() {
        let deleted = item(1, "Deleted");
        let mut queue = PasteQueueState {
            kind: SequentialSessionKind::Recording,
            ..Default::default()
        };
        queue.push(&deleted);
        queue.push(&item(2, "Still queued"));
        let notice = queue.finish_step(EntryIdentity::from_entry(&deleted), None, false, None);
        assert!(matches!(
            notice,
            SequentialNotice::Skipped {
                remaining: 1,
                hotkey: None,
                round_ended: false
            }
        ));
    }

    #[test]
    fn the_progress_notice_reflects_whether_the_pasted_duplicate_was_deleted() {
        let repeated = item(1, "Repeated");
        let identity = EntryIdentity::from_entry(&repeated);
        for deleted in [true, false] {
            let mut queue = PasteQueueState {
                kind: SequentialSessionKind::OneShot,
                ..Default::default()
            };
            queue.push(&repeated);
            queue.push(&repeated);
            let notice = queue.finish_step(identity, Some(Instant::now()), deleted, None);
            if deleted {
                assert!(matches!(notice, SequentialNotice::Done { total: 1 }));
                assert!(queue.items.is_empty());
                queue.end_if_finished();
                assert_eq!(queue.kind, SequentialSessionKind::Inactive);
            } else {
                assert!(matches!(
                    notice,
                    SequentialNotice::Pasted {
                        current: 1,
                        total: 2
                    }
                ));
                assert_eq!(queue.items.front().unwrap().identity, identity);
            }
        }
    }

    #[test]
    fn deleting_history_removes_all_occurrences_of_that_exact_capture() {
        let mut queue = PasteQueueState {
            kind: SequentialSessionKind::OneShot,
            ..Default::default()
        };
        let deleted = item(8, "Original");
        let mut reused_id = item(8, "New row");
        reused_id.created_at += 10;
        for entry in [&deleted, &reused_id, &deleted] {
            queue.push(entry);
        }
        assert!(queue.remove_where(|item| item.identity.matches(&deleted)));
        assert_eq!(queue.items.len(), 1);
        assert!(queue.items[0].identity.matches(&reused_id));
        assert_eq!(queue.round_total, 1);
        assert!(queue.remove_where(|item| item.identity.matches(&reused_id)));
        queue.end_if_finished();
        assert_eq!(queue.kind, SequentialSessionKind::Inactive);
        assert!(!queue.has_items());
    }

    #[test]
    fn reordering_distinguishes_multiple_occurrences_and_rejects_stale_orders() {
        let mut queue = PasteQueueState {
            kind: SequentialSessionKind::Recording,
            ..Default::default()
        };
        let repeated = item(1, "Repeated field");
        queue.push(&repeated);
        queue.push(&repeated);
        let first = queue.items[0].queue_id;
        let second = queue.items[1].queue_id;
        assert!(queue.reorder(&[second, first]));
        assert!(!queue.reorder(&[first, first]));
        assert_eq!(queue.items[0].queue_id, second);
        assert_eq!(queue.items[1].queue_id, first);
    }

    #[test]
    fn an_empty_recording_ends_at_the_wall_clock_deadline_only_for_the_current_timer() {
        let mut queue = PasteQueueState {
            kind: SequentialSessionKind::Recording,
            idle_generation: 2,
            idle_deadline_ms: Some(1_000 + IDLE_TIMEOUT_MS),
            ..Default::default()
        };
        assert!(queue
            .expire_if_idle(Some(1), 1_000 + IDLE_TIMEOUT_MS)
            .is_none());
        assert!(queue
            .expire_if_idle(Some(2), 999 + IDLE_TIMEOUT_MS)
            .is_none());
        assert!(queue
            .expire_if_idle(Some(2), 1_000 + IDLE_TIMEOUT_MS)
            .is_some());
        assert_eq!(queue.kind, SequentialSessionKind::Inactive);
    }

    #[test]
    fn idle_checks_wait_at_most_thirty_seconds_and_follow_wall_clock_time() {
        let deadline = IDLE_TIMEOUT_MS;
        let queue = PasteQueueState {
            kind: SequentialSessionKind::Recording,
            idle_generation: 2,
            idle_deadline_ms: Some(deadline),
            ..Default::default()
        };
        assert_eq!(queue.idle_check_delay(2, 0), Some(Duration::from_secs(30)));
        assert_eq!(
            queue.idle_check_delay(2, deadline - 4_000),
            Some(Duration::from_secs(4))
        );
        assert_eq!(
            queue.idle_check_delay(2, deadline + 1_000),
            Some(Duration::ZERO)
        );
        assert_eq!(queue.idle_check_delay(1, deadline), None);
    }
}
