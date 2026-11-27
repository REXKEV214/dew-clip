use crate::app::commands::thumbnail_cmd::{thumbnail_dir, THUMBNAIL_WRITERS};
use crate::app::data_directory::session_storage_root;
use crate::app_state::{AppDataDir, PasteQueue, SessionHistory, SettingsState};
use crate::database::{DbState, ATTACHMENT_WRITERS, BYTES_PER_MB};
use crate::domain::models::{ClipboardEntry, EntryIdentity};
use crate::error::{AppError, AppResult};
use crate::infrastructure::repository::clipboard_repo::SqliteClipboardRepository;
use crate::infrastructure::repository::settings_repo::SettingsRepository;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::io::{self, ErrorKind};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::sync::{Mutex, OnceLock, TryLockError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};

const STARTUP_DELAY: Duration = Duration::from_secs(5);
const SESSION_SWEEP_INTERVAL: Duration = Duration::from_secs(30);
const MAINTENANCE_INTERVAL: Duration = Duration::from_secs(10 * 60);
const CAPTURE_DELAY: Duration = Duration::from_secs(2);
const DAY_MS: u64 = 86_400_000;
static CLEANUP_LOCK: Mutex<()> = Mutex::new(());
static MAINTENANCE_SIGNAL: OnceLock<SyncSender<()>> = OnceLock::new();

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageUsage {
    database_bytes: u64,
    history_attachments_bytes: u64,
    favorites_media_bytes: u64,
    thumbnail_cache_bytes: u64,
    total_bytes: u64,
    protected_over_limit: bool,
}

fn path_bytes(path: &Path) -> io::Result<u64> {
    let metadata = match path.symlink_metadata() {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    if metadata.is_file() {
        return Ok(metadata.len());
    }
    if !metadata.is_dir() {
        return Ok(0);
    }
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error),
    };
    let mut bytes = 0;
    for entry in entries {
        bytes += path_bytes(&entry?.path())?;
    }
    Ok(bytes)
}

fn database_bytes(root: &Path) -> io::Result<u64> {
    let mut bytes = 0;
    for file in ["clipboard.db", "clipboard.db-wal", "clipboard.db-shm"] {
        bytes += path_bytes(&root.join(file))?;
    }
    Ok(bytes)
}

fn storage_usage(root: &Path, thumbnail_cache: &Path) -> io::Result<StorageUsage> {
    let database_bytes = database_bytes(root)?;
    let history_attachments_bytes = path_bytes(&root.join("attachments"))?;
    let favorites_media_bytes = path_bytes(&root.join("favorites"))?;
    let thumbnail_cache_bytes =
        path_bytes(thumbnail_cache)? + path_bytes(&session_storage_root(root).join("thumbnails"))?;
    Ok(StorageUsage {
        database_bytes,
        history_attachments_bytes,
        favorites_media_bytes,
        thumbnail_cache_bytes,
        total_bytes: database_bytes
            + history_attachments_bytes
            + favorites_media_bytes
            + thumbnail_cache_bytes,
        protected_over_limit: false,
    })
}

fn clear_thumbnail_dirs(root: &Path, thumbnail_cache: &Path) -> io::Result<u64> {
    let _thumbnails = THUMBNAIL_WRITERS.write().unwrap();
    let session_cache = session_storage_root(root).join("thumbnails");
    let mut released = 0;
    for dir in [thumbnail_cache, session_cache.as_path()] {
        match dir.symlink_metadata() {
            Ok(metadata) if metadata.is_dir() => {
                released += path_bytes(dir)?;
                fs::remove_dir_all(dir)?;
            }
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        };
    }
    Ok(released)
}

#[tauri::command(async)]
pub fn get_storage_usage(app: AppHandle) -> AppResult<StorageUsage> {
    let root = app.state::<AppDataDir>().0.lock().unwrap().clone();
    let mut usage = storage_usage(&root, &thumbnail_dir(&app)?)?;
    let limit = app
        .state::<SettingsState>()
        .storage_limit_mb
        .load(Ordering::Relaxed);
    if limit > 0 {
        let db = app.state::<DbState>();
        let conn = db.conn.lock().unwrap();
        let max_bytes = limit.saturating_mul(BYTES_PER_MB);
        usage.protected_over_limit = protected_over_limit(&db.repo, &conn, &root, max_bytes)?;
    }
    Ok(usage)
}

#[tauri::command(async)]
pub fn clear_thumbnail_cache(app: AppHandle) -> AppResult<u64> {
    let root = app.state::<AppDataDir>().0.lock().unwrap().clone();
    Ok(clear_thumbnail_dirs(&root, &thumbnail_dir(&app)?)?)
}

#[tauri::command(async)]
pub fn run_storage_cleanup_now(
    app: AppHandle,
    change: Option<StorageSettingChange>,
) -> AppResult<usize> {
    let _cleanup = CLEANUP_LOCK.lock().unwrap();
    // Saving under this lock lets the confirmed cleanup report its own deletions.
    if let Some(change) = change {
        let settings = app.state::<SettingsState>();
        let field = match change.key.as_str() {
            "app.history_retention_days" => &settings.history_retention_days,
            "app.storage_limit_mb" => &settings.storage_limit_mb,
            _ => return Err(AppError::Validation("unknown storage setting".into())),
        };
        let value: u64 = change
            .value
            .parse()
            .map_err(|error| AppError::Validation(format!("{error}")))?;
        app.state::<DbState>()
            .settings_repo
            .set(&change.key, &change.value)?;
        field.store(value, Ordering::Relaxed);
    }
    run_maintenance(&app, true).map(|(count, _)| count)
}

#[derive(Deserialize)]
pub struct StorageSettingChange {
    key: String,
    value: String,
}

#[derive(Clone, Copy)]
enum CleanupRule {
    Age(i64),
    Size(u64),
}

fn checkpoint(conn: &Connection) -> rusqlite::Result<bool> {
    conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
        Ok(row.get::<_, i64>(0)? == 0)
    })
}

fn database_used_bytes(conn: &Connection) -> rusqlite::Result<u64> {
    conn.query_row(
        "SELECT (page_count - freelist_count) * page_size
         FROM pragma_page_count(), pragma_freelist_count(), pragma_page_size()",
        [],
        |row| row.get(0),
    )
}

fn history_used_bytes(conn: &Connection, root: &Path) -> AppResult<u64> {
    Ok(database_used_bytes(conn)?.saturating_add(path_bytes(&root.join("attachments"))?))
}

fn protected_over_limit(
    repo: &SqliteClipboardRepository,
    conn: &Connection,
    root: &Path,
    max_bytes: u64,
) -> AppResult<bool> {
    Ok(history_used_bytes(conn, root)? > max_bytes
        && repo.protected_history_bytes(conn, &root.join("attachments"), max_bytes)? > max_bytes)
}

fn apply_batch<'a>(
    rule: CleanupRule,
    repo: &SqliteClipboardRepository,
    conn: &Connection,
    root: &Path,
    session: impl Iterator<Item = &'a ClipboardEntry> + Clone,
    deleted: &mut HashSet<EntryIdentity>,
) -> AppResult<bool> {
    let (cutoff, limit, before) = match rule {
        CleanupRule::Age(cutoff) => (Some(cutoff), 500, None),
        CleanupRule::Size(max_bytes) => {
            let attachments = path_bytes(&root.join("attachments"))?;
            // Files on disk are an upper bound. The deletion decision uses live pages.
            if database_bytes(root)?.saturating_add(attachments) <= max_bytes {
                return Ok(false);
            }
            // Reclaim pending files and transient WAL growth before deleting history.
            repo.sweep_attachment_deletions(conn, root, Vec::new(), session.clone())?;
            if !checkpoint(conn)? {
                return Err(AppError::Database("WAL checkpoint busy".into()));
            }
            let before = history_used_bytes(conn, root)?;
            if before <= max_bytes
                || repo.protected_history_bytes(conn, &root.join("attachments"), max_bytes)?
                    > max_bytes
            {
                return Ok(false);
            }
            (None, 50, Some(before))
        }
    };
    let batch = repo.prune_history_batch(conn, root, cutoff, limit)?;
    if batch.is_empty() {
        return Ok(false);
    }
    let full_batch = batch.len() == limit as usize;
    deleted.extend(batch);
    let sweep = repo.sweep_attachment_deletions(conn, root, Vec::new(), session);
    let checkpointed = checkpoint(conn)?;
    sweep?;
    if let (CleanupRule::Size(max_bytes), Some(before)) = (rule, before) {
        if !checkpointed {
            return Err(AppError::Database("WAL checkpoint busy".into()));
        }
        let after = history_used_bytes(conn, root)?;
        return Ok(after > max_bytes && after < before);
    }
    Ok(full_batch)
}

fn run_maintenance(app: &AppHandle, wait: bool) -> AppResult<(usize, bool)> {
    let settings = app.state::<SettingsState>();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    let days = settings.history_retention_days.load(Ordering::Relaxed);
    let mb = settings.storage_limit_mb.load(Ordering::Relaxed);
    let age = (days > 0).then(|| {
        CleanupRule::Age(
            now.saturating_sub(days.saturating_mul(DAY_MS).min(i64::MAX as u64) as i64),
        )
    });
    let size = (mb > 0).then(|| CleanupRule::Size(mb.saturating_mul(BYTES_PER_MB)));
    let mut deleted = HashSet::new();
    let run = (|| -> AppResult<bool> {
        for rule in [age, size].into_iter().flatten() {
            loop {
                // Limit contention with capture by releasing every lock between batches.
                let _attachments = if wait {
                    ATTACHMENT_WRITERS.write().unwrap()
                } else {
                    match ATTACHMENT_WRITERS.try_write() {
                        Ok(lock) => lock,
                        Err(TryLockError::WouldBlock) => return Ok(false),
                        Err(error) => panic!("{error}"),
                    }
                };
                let root = app.state::<AppDataDir>().0.lock().unwrap().clone();
                let session = app.state::<SessionHistory>();
                let session = session.0.lock().unwrap();
                let db = app.state::<DbState>();
                let conn = db.conn.lock().unwrap();
                // A settings reset or a looser limit can arrive between batches.
                if settings.history_retention_days.load(Ordering::Relaxed) != days
                    || settings.storage_limit_mb.load(Ordering::Relaxed) != mb
                {
                    return Ok(false);
                }
                if !apply_batch(rule, &db.repo, &conn, &root, session.iter(), &mut deleted)? {
                    break;
                }
            }
        }
        Ok(true)
    })();
    if !deleted.is_empty() {
        app.state::<PasteQueue>()
            .0
            .lock()
            .unwrap()
            .items
            .retain(|item| !deleted.contains(item));
        let _ = app.emit("clipboard-changed", ());
    }
    run.map(|completed| (deleted.len(), completed))
}

pub(crate) fn after_capture(app: &AppHandle, entry: &ClipboardEntry) {
    static CAPTURES: AtomicUsize = AtomicUsize::new(0);
    let captures = CAPTURES.fetch_add(1, Ordering::Relaxed);
    if captures % 20 == 19 || matches!(entry.content_type.as_str(), "image" | "rich_text") {
        request_maintenance(app);
    }
}

pub(crate) fn after_attachments(app: &AppHandle) {
    if app
        .state::<SettingsState>()
        .persistent
        .load(Ordering::Relaxed)
    {
        request_maintenance(app);
    }
}

fn request_maintenance(app: &AppHandle) {
    let settings = app.state::<SettingsState>();
    if settings.history_retention_days.load(Ordering::Relaxed) == 0
        && settings.storage_limit_mb.load(Ordering::Relaxed) == 0
    {
        return;
    }
    if let Some(sender) = MAINTENANCE_SIGNAL.get() {
        let _ = sender.try_send(());
    }
}

pub(crate) fn start_sweeper(app: AppHandle) {
    let (sender, receiver) = mpsc::sync_channel(1);
    MAINTENANCE_SIGNAL
        .set(sender)
        .expect("storage sweeper already started");
    std::thread::spawn(move || {
        let mut session_sweep = Instant::now();
        let startup = session_sweep + STARTUP_DELAY;
        let mut maintenance = startup;
        let mut after_capture: Option<Instant> = None;
        loop {
            let now = Instant::now();
            if now >= session_sweep {
                crate::services::clipboard::sweep_session_files(&app);
                session_sweep = now + SESSION_SWEEP_INTERVAL;
            }
            if now >= maintenance || after_capture.is_some_and(|due| now >= due) {
                if now >= maintenance {
                    maintenance = now + MAINTENANCE_INTERVAL;
                }
                after_capture = None;
                let outcome = match CLEANUP_LOCK.try_lock() {
                    Ok(_cleanup) => run_maintenance(&app, false),
                    Err(TryLockError::WouldBlock) => Ok((0, false)),
                    Err(error) => panic!("{error}"),
                };
                match outcome {
                    Ok((_, false)) => after_capture = Some(Instant::now() + CAPTURE_DELAY),
                    Err(error) => crate::error!(">>> [STORAGE] {error}"),
                    _ => {}
                }
            }
            let next = session_sweep
                .min(maintenance)
                .min(after_capture.unwrap_or(maintenance));
            match receiver.recv_timeout(next.saturating_duration_since(Instant::now())) {
                Ok(()) => {
                    after_capture.get_or_insert(
                        (Instant::now() + CAPTURE_DELAY)
                            .max(startup)
                            .min(maintenance),
                    );
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use std::os::unix::fs::symlink;
    use std::path::PathBuf;
    use std::sync::Arc;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("dewclip-storage-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&root).unwrap();
            Self(root)
        }

        fn database(&self) -> Arc<Mutex<Connection>> {
            let path = self.0.join("clipboard.db");
            Arc::new(Mutex::new(
                crate::database::init_db(path.to_str().unwrap()).unwrap(),
            ))
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn retention_checks_all_activity_times_and_protects_pins_tags_and_favorites() {
        let dir = TestDirectory::new();
        let database = dir.database();
        let repo = SqliteClipboardRepository::new(database.clone());
        let conn = database.lock().unwrap();
        conn.execute_batch(
            "INSERT INTO clipboard_history
                (id, content_type, content, source_app, timestamp, preview, created_at, last_used_at, sort_at, is_pinned, tags)
             VALUES
                (1, 'text', 'expired', 'Notes', 1, 'expired', 1, 1, 1, 0, '[]'),
                (2, 'text', 'recent', 'Notes', 2, 'recent', 100, 2, 2, 0, '[]'),
                (3, 'text', 'used', 'Notes', 3, 'used', 3, 100, 3, 0, '[]'),
                (4, 'text', 'sorted', 'Notes', 4, 'sorted', 4, 4, 100, 0, '[]'),
                (5, 'text', 'pinned', 'Notes', 5, 'pinned', 5, 5, 5, 1, '[]'),
                (6, 'text', 'old tag', 'Notes', 6, 'old tag', 6, 6, 6, 0, '[\"label\"]'),
                (7, 'text', 'tag', 'Notes', 7, 'tag', 7, 7, 7, 0, '[]');
             INSERT INTO entry_tags (entry_id, tag) VALUES (7, 'label');
             INSERT INTO favorites (content_type, content, sort_order, created_at, updated_at)
             VALUES ('text', 'favorite copy', 1, 1, 1);",
        ).unwrap();
        let mut deleted = HashSet::new();
        assert!(!apply_batch(
            CleanupRule::Age(100),
            &repo,
            &conn,
            &dir.0,
            std::iter::empty(),
            &mut deleted
        )
        .unwrap());
        assert_eq!(
            deleted,
            HashSet::from([EntryIdentity {
                id: 1,
                created_at: 1
            }])
        );
        assert_eq!(
            conn.query_row("SELECT count(*) FROM clipboard_history", [], |row| row
                .get::<_, u64>(0))
                .unwrap(),
            6
        );
        assert_eq!(
            conn.query_row("SELECT content FROM favorites", [], |row| row
                .get::<_, String>(0))
                .unwrap(),
            "favorite copy"
        );
        assert_eq!(path_bytes(&dir.0.join("clipboard.db-wal")).unwrap(), 0);
    }

    #[test]
    fn size_cleanup_removes_oldest_batch_and_sweeps_only_unreferenced_attachments() {
        let dir = TestDirectory::new();
        let database = dir.database();
        let repo = SqliteClipboardRepository::new(database.clone());
        let conn = database.lock().unwrap();
        let attachments = dir.0.join("attachments");
        fs::create_dir(&attachments).unwrap();
        let shared = attachments.join("img_1.png");
        let unused = attachments.join("img_2.png");
        fs::File::create(&shared)
            .unwrap()
            .set_len(BYTES_PER_MB)
            .unwrap();
        fs::File::create(&unused)
            .unwrap()
            .set_len(2 * BYTES_PER_MB)
            .unwrap();
        let shared_html = format!("<img src=\"file://{}\">", shared.display());
        let unused_html = format!("<img src=\"file://{}\">", unused.display());
        conn.execute(
            "INSERT INTO clipboard_history
                (id, content_type, content, html_content, source_app, timestamp, preview, created_at, sort_at)
             VALUES (1, 'rich_text', 'body', ?1, 'Notes', 1, 'body', 1, 1)",
            [format!("{shared_html}{unused_html}")],
        ).unwrap();
        for id in 2..=51 {
            conn.execute(
                "INSERT INTO clipboard_history
                    (id, content_type, content, source_app, timestamp, preview, created_at, sort_at)
                 VALUES (?1, 'text', 'copied text', 'Notes', ?1, 'text', ?1, ?1)",
                [id],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO favorites (content_type, content, html_content, sort_order, created_at, updated_at)
             VALUES ('rich_text', 'favorite copy', ?1, 1, 1, 1)",
            [shared_html],
        ).unwrap();
        let rule = CleanupRule::Size(2 * BYTES_PER_MB);
        let mut deleted = HashSet::new();
        assert!(
            !apply_batch(rule, &repo, &conn, &dir.0, std::iter::empty(), &mut deleted).unwrap()
        );
        assert!(
            !apply_batch(rule, &repo, &conn, &dir.0, std::iter::empty(), &mut deleted).unwrap()
        );
        assert_eq!(deleted.len(), 50);
        assert_eq!(
            conn.query_row("SELECT id FROM clipboard_history", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            51
        );
        assert!(!unused.exists());
        assert!(shared.exists());
        assert_eq!(
            conn.query_row(
                "SELECT count(*) FROM pending_attachment_deletions",
                [],
                |row| row.get::<_, u64>(0)
            )
            .unwrap(),
            0
        );
        conn.execute("UPDATE clipboard_history SET is_pinned = 1", [])
            .unwrap();
        deleted.clear();
        assert!(!apply_batch(
            CleanupRule::Size(1),
            &repo,
            &conn,
            &dir.0,
            std::iter::empty(),
            &mut deleted
        )
        .unwrap());
        assert!(deleted.is_empty());
    }

    #[test]
    fn size_cleanup_reclaims_wal_and_excludes_favorites_and_thumbnail_cache() {
        let dir = TestDirectory::new();
        let database = dir.database();
        let repo = SqliteClipboardRepository::new(database.clone());
        let conn = database.lock().unwrap();
        fs::create_dir(dir.0.join("favorites")).unwrap();
        fs::create_dir(dir.0.join("thumbnails")).unwrap();
        for file in ["favorites/media.mp4", "thumbnails/cover.jpg"] {
            fs::File::create(dir.0.join(file))
                .unwrap()
                .set_len(4 * BYTES_PER_MB)
                .unwrap();
        }
        conn.execute(
            "INSERT INTO clipboard_history (content_type, content, source_app, timestamp, preview)
             VALUES ('text', 'copied text', 'Notes', 1, 'text')",
            [],
        )
        .unwrap();
        let committed_bytes = path_bytes(&dir.0.join("clipboard.db")).unwrap()
            + path_bytes(&dir.0.join("clipboard.db-shm")).unwrap();
        assert!(database_bytes(&dir.0).unwrap() > committed_bytes);
        let mut deleted = HashSet::new();
        assert!(!apply_batch(
            CleanupRule::Size(BYTES_PER_MB),
            &repo,
            &conn,
            &dir.0,
            std::iter::empty(),
            &mut deleted
        )
        .unwrap());
        assert!(deleted.is_empty());
        let mut estimated =
            database_bytes(&dir.0).unwrap() - path_bytes(&dir.0.join("clipboard.db-wal")).unwrap();
        assert!(checkpoint(&conn).unwrap());
        estimated = estimated.max(database_bytes(&dir.0).unwrap());
        for revision in 0..4 {
            conn.execute(
                "UPDATE clipboard_history SET use_count = ?1",
                params![revision + 1],
            )
            .unwrap();
        }
        assert!(database_bytes(&dir.0).unwrap() > estimated);
        assert!(!apply_batch(
            CleanupRule::Size(estimated),
            &repo,
            &conn,
            &dir.0,
            std::iter::empty(),
            &mut deleted
        )
        .unwrap());
        assert!(deleted.is_empty());
        assert_eq!(path_bytes(&dir.0.join("clipboard.db-wal")).unwrap(), 0);
    }

    #[test]
    fn size_cleanup_stops_at_the_limit_on_a_database_without_auto_vacuum() {
        let dir = TestDirectory::new();
        let old_db = Connection::open(dir.0.join("clipboard.db")).unwrap();
        old_db
            .execute_batch("PRAGMA auto_vacuum = NONE; CREATE TABLE old_database (value TEXT);")
            .unwrap();
        drop(old_db);

        let database = dir.database();
        let repo = SqliteClipboardRepository::new(database.clone());
        let conn = database.lock().unwrap();
        assert_eq!(
            conn.query_row("PRAGMA auto_vacuum", [], |row| row.get::<_, u64>(0))
                .unwrap(),
            0
        );
        conn.execute_batch(
            "WITH RECURSIVE entries(id) AS (VALUES(1) UNION ALL SELECT id + 1 FROM entries WHERE id < 125)
             INSERT INTO clipboard_history (id, content_type, content, source_app, timestamp, preview, sort_at)
             SELECT id, 'text', CAST(zeroblob(32768) AS TEXT), 'Notes', id, 'copied text', id FROM entries;",
        ).unwrap();
        let max_bytes = 3 * BYTES_PER_MB;
        assert!(checkpoint(&conn).unwrap());
        let file_bytes = path_bytes(&dir.0.join("clipboard.db")).unwrap();
        assert!(database_used_bytes(&conn).unwrap() > max_bytes);

        let mut deleted = HashSet::new();
        while apply_batch(
            CleanupRule::Size(max_bytes),
            &repo,
            &conn,
            &dir.0,
            std::iter::empty(),
            &mut deleted,
        )
        .unwrap()
        {}
        assert_eq!(deleted.len(), 50);
        assert_eq!(path_bytes(&dir.0.join("clipboard.db")).unwrap(), file_bytes);
        assert!(file_bytes > max_bytes);
        assert!(database_used_bytes(&conn).unwrap() <= max_bytes);
        assert_eq!(
            conn.query_row("SELECT count(*) FROM clipboard_history", [], |row| row
                .get::<_, u64>(0))
                .unwrap(),
            75
        );
        assert!(!apply_batch(
            CleanupRule::Size(max_bytes),
            &repo,
            &conn,
            &dir.0,
            std::iter::empty(),
            &mut deleted
        )
        .unwrap());
        assert_eq!(deleted.len(), 50);
    }

    #[test]
    fn a_size_batch_with_no_space_reduction_stops_cleanup() {
        let dir = TestDirectory::new();
        let database = dir.database();
        let repo = SqliteClipboardRepository::new(database.clone());
        let conn = database.lock().unwrap();
        let attachments = dir.0.join("attachments");
        fs::create_dir(&attachments).unwrap();
        fs::File::create(attachments.join("unreferenced.bin"))
            .unwrap()
            .set_len(2 * BYTES_PER_MB)
            .unwrap();
        conn.execute_batch(
            "WITH RECURSIVE entries(id) AS (VALUES(1) UNION ALL SELECT id + 1 FROM entries WHERE id < 60)
             INSERT INTO clipboard_history (id, content_type, content, source_app, timestamp, preview, sort_at)
             SELECT id, 'text', 'x', 'Notes', id, 'x', id FROM entries;",
        ).unwrap();
        let before = history_used_bytes(&conn, &dir.0).unwrap();
        let mut deleted = HashSet::new();
        assert!(!apply_batch(
            CleanupRule::Size(BYTES_PER_MB),
            &repo,
            &conn,
            &dir.0,
            std::iter::empty(),
            &mut deleted
        )
        .unwrap());
        assert_eq!(history_used_bytes(&conn, &dir.0).unwrap(), before);
        assert_eq!(deleted.len(), 50);
    }

    #[test]
    fn an_over_limit_protected_history_pauses_pruning_and_sets_the_usage_note() {
        let dir = TestDirectory::new();
        let database = dir.database();
        let repo = SqliteClipboardRepository::new(database.clone());
        let conn = database.lock().unwrap();
        let attachments = dir.0.join("attachments");
        fs::create_dir(&attachments).unwrap();
        let image = attachments.join("protected.png");
        fs::File::create(&image)
            .unwrap()
            .set_len(BYTES_PER_MB / 2)
            .unwrap();
        conn.execute_batch(
            "INSERT INTO clipboard_history (id, content_type, content, source_app, timestamp, preview, is_pinned)
             VALUES (1, 'text', CAST(zeroblob(655360) AS TEXT), 'Notes', 1, 'pinned', 1),
                    (2, 'text', 'ordinary', 'Notes', 2, 'ordinary', 0);",
        ).unwrap();
        conn.execute(
            "INSERT INTO clipboard_history (id, content_type, content, source_app, timestamp, preview, is_external)
             VALUES (3, 'image', ?1, 'Notes', 3, 'tagged image', 1)",
            [image.to_str().unwrap()],
        ).unwrap();
        conn.execute(
            "INSERT INTO entry_tags (entry_id, tag) VALUES (3, 'important')",
            [],
        )
        .unwrap();
        assert!(protected_over_limit(&repo, &conn, &dir.0, BYTES_PER_MB).unwrap());
        assert!(!protected_over_limit(&repo, &conn, &dir.0, 2 * BYTES_PER_MB).unwrap());
        let mut deleted = HashSet::new();
        assert!(!apply_batch(
            CleanupRule::Size(BYTES_PER_MB),
            &repo,
            &conn,
            &dir.0,
            std::iter::empty(),
            &mut deleted
        )
        .unwrap());
        assert!(deleted.is_empty());
        assert_eq!(
            conn.query_row("SELECT count(*) FROM clipboard_history", [], |row| row
                .get::<_, u64>(0))
                .unwrap(),
            3
        );
        assert!(image.exists());
    }

    #[test]
    fn storage_totals_ignore_symlinks_and_clearing_only_removes_thumbnail_caches() {
        let dir = TestDirectory::new();
        let database = dir.database();
        let root = &dir.0;
        let cache = root.join("cache/thumbnails");
        // A directory created by the user can occupy the standard session name.
        fs::create_dir_all(root.join("session/thumbnails")).unwrap();
        fs::write(root.join("session/thumbnails/keep.jpg"), b"user file").unwrap();
        crate::app::data_directory::prepare_session(root).unwrap();
        let session_cache = session_storage_root(root).join("thumbnails");
        for (path, bytes) in [
            (root.join("attachments/photo.png"), 11),
            (root.join("favorites/media.mp4"), 17),
            (cache.join("cover.jpg"), 19),
            (session_cache.join("cover.jpg"), 23),
        ] {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::File::create(path).unwrap().set_len(bytes).unwrap();
        }
        symlink(root, root.join("attachments/loop")).unwrap();
        symlink(root.join("favorites/media.mp4"), cache.join("elsewhere")).unwrap();

        let db_bytes: u64 = ["clipboard.db", "clipboard.db-wal", "clipboard.db-shm"]
            .iter()
            .map(|file| fs::metadata(root.join(file)).unwrap().len())
            .sum();
        let usage = storage_usage(root, &cache).unwrap();
        assert_eq!(usage.database_bytes, db_bytes);
        assert_eq!(usage.history_attachments_bytes, 11);
        assert_eq!(usage.favorites_media_bytes, 17);
        assert_eq!(usage.thumbnail_cache_bytes, 42);
        assert_eq!(usage.total_bytes, db_bytes + 70);

        assert_eq!(clear_thumbnail_dirs(root, &cache).unwrap(), 42);
        assert_eq!(clear_thumbnail_dirs(root, &cache).unwrap(), 0);
        assert!(!cache.exists() && !session_cache.exists());
        assert!(root.join("attachments/photo.png").exists());
        assert!(root.join("favorites/media.mp4").exists());
        assert!(root.join("session/thumbnails/keep.jpg").exists());
        assert_eq!(
            storage_usage(root, &cache).unwrap().thumbnail_cache_bytes,
            0
        );
        drop(database);
    }
}
