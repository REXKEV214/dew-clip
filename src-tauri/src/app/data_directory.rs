//! Session storage and user-requested moves of the active data directory.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior};

const MOVE_REQUEST: &str = ".dewclip-data-directory-move.json";
const OWNER: &str = ".dewclip-data-directory";
pub(crate) const SESSION_OWNER: &str = ".dewclip-session";
const OWNER_NAME: &str = "DewClip\n";

pub(crate) fn owns_session(path: &Path) -> bool {
    let marker = path.join(SESSION_OWNER);
    path.symlink_metadata().is_ok_and(|meta| meta.is_dir())
        && marker.symlink_metadata().is_ok_and(|meta| meta.is_file())
        && std::fs::read_to_string(marker).is_ok_and(|value| value == OWNER_NAME)
}

pub(crate) fn session_storage_root(data_dir: &Path) -> PathBuf {
    let root = data_dir.join("session");
    // An unmarked session directory can belong to the user. Leave it in place.
    for number in 0.. {
        let path = if number == 0 {
            root.clone()
        } else {
            data_dir.join(format!("session.{number}"))
        };
        if owns_session(&path) {
            return path;
        }
        if path.symlink_metadata().is_err() {
            return path;
        }
    }
    unreachable!()
}

pub(crate) fn prepare_session(data_dir: &Path) -> std::io::Result<()> {
    let root = session_storage_root(data_dir);
    if owns_session(&root) {
        return Ok(());
    }
    std::fs::create_dir(&root)?;
    std::fs::write(root.join(SESSION_OWNER), OWNER_NAME)
}

#[derive(serde::Serialize, serde::Deserialize)]
struct RequestedMove {
    source: PathBuf,
    destination: PathBuf,
    id: uuid::Uuid,
}

fn error(context: &str, err: impl std::fmt::Display) -> String {
    format!("数据目录迁移失败（{context}）：{err}。原有数据已保留，请解决问题后重新启动。")
}

fn validate_move_target(source: &Path, target: &Path) -> Result<PathBuf, String> {
    let source = source.canonicalize().map_err(|e| error("检查原目录", e))?;
    let target = target
        .canonicalize()
        .map_err(|e| error("检查目标目录", e))?;
    if source == target {
        return Ok(target);
    }
    if target.starts_with(&source)
        || source.starts_with(&target)
        || target.ancestors().any(|path| {
            path.extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
        })
    {
        return Err(error("目标目录", "请选择独立的普通文件夹"));
    }
    for entry in std::fs::read_dir(&target).map_err(|e| error("检查目标目录", e))? {
        let entry = entry.map_err(|e| error("检查目标目录", e))?;
        if entry.file_name() != OWNER
            || !entry
                .file_type()
                .map_err(|e| error("检查目录标记", e))?
                .is_file()
            || std::fs::read_to_string(entry.path())
                .ok()
                .and_then(|value| uuid::Uuid::parse_str(value.strip_prefix(OWNER_NAME)?).ok())
                .is_none()
        {
            return Err(error("目标目录", "请选择空目录，已有数据不会被覆盖"));
        }
    }
    Ok(target)
}

fn atomic_write(path: &Path, value: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(value)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

/// The next process makes the final copy, after the old process releases both
/// the database and its attachment writers.
pub(crate) fn request_move(config: &Path, source: &Path, target: &Path) -> Result<(), String> {
    let target = validate_move_target(source, target)?;
    let source = source.canonicalize().map_err(|e| error("检查原目录", e))?;
    let request_path = config.join(MOVE_REQUEST);
    if source == target {
        return match std::fs::remove_file(request_path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(error("取消目录迁移", e)),
        };
    }
    let request = RequestedMove {
        source,
        destination: target,
        id: uuid::Uuid::new_v4(),
    };
    let bytes = serde_json::to_vec(&request).map_err(|e| error("生成迁移请求", e))?;
    atomic_write(&request_path, &bytes).map_err(|e| error("保存迁移请求", e))
}

pub(crate) fn apply_requested_move(
    config: &Path,
    source: &Path,
) -> Result<Option<PathBuf>, String> {
    let path = config.join(MOVE_REQUEST);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(error("读取迁移请求", e)),
    };
    let request: RequestedMove =
        serde_json::from_slice(&bytes).map_err(|e| error("读取迁移请求", e))?;
    let current = source.canonicalize().map_err(|e| error("检查原目录", e))?;
    if current == request.destination {
        let _ = std::fs::remove_file(path);
        return Ok(None);
    }
    if current != request.source {
        return Err(error("原目录", "活动数据位置已改变，请重新选择目标目录"));
    }
    publish_move(config, source, &request)?;
    // The redirect is the commit point. A stale request is harmless next time.
    let _ = std::fs::remove_file(path);
    Ok(Some(request.destination))
}

fn publish_move(config: &Path, source: &Path, request: &RequestedMove) -> Result<(), String> {
    let target = &request.destination;
    let id = request.id.to_string();
    let stage = target.with_file_name(format!(".dewclip-move-{id}"));
    let marker = target.join(OWNER);
    let marker_value = format!("{OWNER_NAME}{id}");
    let published = target.canonicalize().is_ok_and(|path| path == *target)
        && marker.symlink_metadata().is_ok_and(|meta| meta.is_file())
        && std::fs::read_to_string(&marker).is_ok_and(|value| value == marker_value);
    let old_marker = if published {
        None
    } else {
        if validate_move_target(source, target)? != *target {
            return Err(error("目标目录", "目录位置已改变"));
        }
        match std::fs::read(&marker) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(error("读取目录标记", e)),
        }
    };
    let old_permissions = std::fs::metadata(target)
        .map_err(|e| error("检查目标目录", e))?
        .permissions();
    if !published {
        if stage.exists() {
            // The random name is recorded before staging, so an interrupted copy
            // can discard this specific staging directory and start over.
            std::fs::remove_dir_all(&stage).map_err(|e| error("清理上次暂存目录", e))?;
        }
        let mut builder = std::fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder
            .create(&stage)
            .map_err(|e| error("创建暂存目录", e))?;
        let result = (|| {
            copy_data_snapshot(source, target, &stage)?;
            std::fs::write(stage.join(OWNER), &marker_value).map_err(|e| error("标记数据目录", e))?;
            if validate_move_target(source, target)? != *target {
                return Err(error("目标目录", "目录位置已改变"));
            }
            // An empty destination directory is replaced atomically. A
            // marker-only destination has no application data to replace.
            if old_marker.is_some() {
                std::fs::remove_file(&marker).map_err(|e| error("更新目录标记", e))?;
            }
            if let Err(e) = std::fs::rename(&stage, target) {
                if let Some(bytes) = &old_marker {
                    std::fs::write(&marker, bytes).map_err(|e| error("恢复目录标记", e))?;
                }
                return Err(error("提交暂存目录", e));
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_dir_all(&stage);
            return result;
        }
    }
    let result = (|| {
        check_database(&target.join("clipboard.db"))?;
        atomic_write(
            &config.join("datapath.txt"),
            target.to_string_lossy().as_bytes(),
        )
        .map_err(|e| error("提交数据目录配置", e))
    })();
    if result.is_err() {
        // Restore an empty destination while keeping the original database in use.
        std::fs::rename(target, &stage).map_err(|e| error("撤销目录切换", e))?;
        std::fs::create_dir(target).map_err(|e| error("恢复目标空目录", e))?;
        std::fs::set_permissions(target, old_permissions)
            .map_err(|e| error("恢复目标目录权限", e))?;
        if let Some(bytes) = &old_marker {
            std::fs::write(&marker, bytes).map_err(|e| error("恢复目录标记", e))?;
        }
        std::fs::remove_dir_all(&stage).map_err(|e| error("清理暂存目录", e))?;
    }
    result
}

fn copy_data_snapshot(source: &Path, target: &Path, stage: &Path) -> Result<(), String> {
    let db_path = stage.join("clipboard.db");
    let database = Connection::open_with_flags(
        source.join("clipboard.db"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(|e| error("打开原数据库", e))?;
    database
        .backup(rusqlite::DatabaseName::Main, &db_path, None)
        .map_err(|e| error("复制数据库快照", e))?;
    for folder in ["attachments", "favorites"] {
        copy_owned_path(&source.join(folder), &stage.join(folder))
            .map_err(|e| error("复制附件", e))?;
    }
    let background: Option<String> = database
        .query_row(
            "SELECT value FROM settings WHERE key = 'app.custom_background'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| error("读取背景路径", e))?;
    if let Some(relative) = background
        .as_deref()
        .and_then(|path| Path::new(path).strip_prefix(source).ok())
    {
        if !relative
            .components()
            .all(|part| matches!(part, std::path::Component::Normal(_)))
        {
            return Err(error("复制背景图片", "路径无效"));
        }
        if !stage.join(relative).exists() {
            copy_owned_path(&source.join(relative), &stage.join(relative))
                .map_err(|e| error("复制背景图片", e))?;
        }
    }
    drop(database);
    rewrite_database(
        &db_path,
        source,
        target,
        &crate::infrastructure::encryption::decrypt_stored_value,
        &|value| {
            crate::infrastructure::encryption::encrypt_value(value)
                .ok_or_else(|| "path encryption unavailable".into())
        },
    )?;
    for name in ["master.key", "master.key.dpapi"] {
        copy_owned_path(&source.join(name), &stage.join(name))
            .map_err(|e| error("复制加密密钥", e))?;
    }
    check_database(&db_path)
}

fn copy_owned_path(source: &Path, destination: &Path) -> std::io::Result<()> {
    let metadata = match std::fs::symlink_metadata(source) {
        Ok(metadata) => metadata,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    if metadata.is_dir() {
        std::fs::create_dir_all(destination)?;
        for entry in std::fs::read_dir(source)? {
            let entry = entry?;
            copy_owned_path(&entry.path(), &destination.join(entry.file_name()))?;
        }
    } else if metadata.is_file() {
        std::fs::create_dir_all(destination.parent().expect("snapshot path has a parent"))?;
        std::fs::copy(source, destination)?;
        std::fs::File::open(destination)?.sync_all()?;
    } else {
        return Err(std::io::Error::other(
            "unsupported file in application data directory",
        ));
    }
    Ok(())
}

fn check_database(path: &Path) -> Result<(), String> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| error("读取原数据库", e))?;
    let check: String = conn
        .query_row("PRAGMA quick_check", [], |r| r.get(0))
        .map_err(|e| error("检查数据库", e))?;
    if check != "ok" {
        return Err(error("检查数据库", check));
    }
    Ok(())
}

pub(crate) fn startup_directory(default: &Path) -> Result<PathBuf, String> {
    let active = redirected_directory(default)?;
    std::fs::create_dir_all(&active).map_err(|e| error("创建数据目录", e))?;
    Ok(active)
}

fn redirected_directory(default: &Path) -> Result<PathBuf, String> {
    let redirect = default.join("datapath.txt");
    let value = match std::fs::read_to_string(&redirect) {
        Ok(value) => value,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(default.into()),
        Err(e) => return Err(error("读取自定义目录", e)),
    };
    let path = PathBuf::from(value.trim());
    if value.trim().is_empty() {
        return Ok(default.into());
    }
    if !path.is_dir() || value.trim().to_ascii_lowercase().ends_with(".app") {
        return Err(error(
            "自定义目录",
            "保存的位置当前不可用，未创建空白记录库",
        ));
    }
    Ok(path)
}

fn rewrite_text(value: &str, source: &Path, destination: &Path) -> String {
    let old = source.to_string_lossy();
    let new = destination.to_string_lossy();
    let encoded = |s: &str| {
        s.split('/')
            .map(|part| urlencoding::encode(part).into_owned())
            .collect::<Vec<_>>()
            .join("/")
    };
    let mut result = value.to_owned();
    for (from, to) in [
        (old.to_string(), new.to_string()),
        (encoded(&old), encoded(&new)),
        (
            urlencoding::encode(&old).into_owned(),
            urlencoding::encode(&new).into_owned(),
        ),
    ] {
        if result == from {
            result = to;
        } else {
            result = result.replace(&format!("{from}/"), &format!("{to}/"));
            result = result.replace(&format!("{from}%2F"), &format!("{to}%2F"));
        }
    }
    result
}

fn rewrite_value<D, E>(
    value: &str,
    has_envelope: bool,
    rewrite: impl FnOnce(&str) -> String,
    decrypt: &D,
    encrypt: &E,
) -> Result<String, String>
where
    D: Fn(&str) -> Result<String, String>,
    E: Fn(&str) -> Result<String, String>,
{
    if has_envelope && crate::infrastructure::encryption::is_encrypted_value(value) {
        let plain = decrypt(value).map_err(|e| error("解密路径", e))?;
        let rewritten = rewrite(&plain);
        if plain == rewritten {
            return Ok(value.into());
        }
        return encrypt(&rewritten).map_err(|e| error("加密路径", e));
    }
    Ok(rewrite(value))
}

fn rewrite_database<D, E>(
    path: &Path,
    source: &Path,
    destination: &Path,
    decrypt: &D,
    encrypt: &E,
) -> Result<(), String>
where
    D: Fn(&str) -> Result<String, String>,
    E: Fn(&str) -> Result<String, String>,
{
    let mut conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .map_err(|e| error("打开数据库", e))?;
    conn.busy_timeout(std::time::Duration::from_secs(2))
        .map_err(|e| error("锁定数据库", e))?;
    let transaction = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| error("开始路径迁移", e))?;
    let media_path = |value: &str| {
        ["attachments", "favorites"].iter().fold(value.to_owned(), |value, folder| {
            rewrite_text(&value, &source.join(folder), &destination.join(folder))
        })
    };
    for table in ["clipboard_history", "favorites"] {
        let mut statement = transaction
            .prepare(&format!("PRAGMA table_info({table})"))
            .map_err(|e| error("检查数据字段", e))?;
        let columns = statement
            .query_map([], |r| r.get::<_, String>(1))
            .map_err(|e| error("检查数据字段", e))?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|e| error("读取数据字段", e))?;
        drop(statement);
        if columns.is_empty() {
            continue;
        }
        for column in ["content", "html_content"] {
            if !columns.iter().any(|v| v == column) {
                continue;
            }
            let condition = if column == "content" {
                if columns.iter().any(|v| v == "is_external") {
                    "(is_external = 1 OR content_type IN ('image','video','file','audio'))"
                } else {
                    "content_type IN ('image','video','file','audio')"
                }
            } else {
                "1 = 1"
            };
            let has_envelope = if table == "favorites" && columns.iter().any(|v| v == "is_encrypted") {
                "is_encrypted"
            } else {
                "1"
            };
            let mut statement = transaction
                .prepare(&format!(
                    "SELECT id, {column}, {has_envelope} FROM {table} WHERE {column} IS NOT NULL AND {condition}"
                ))
                .map_err(|e| error("读取媒体路径", e))?;
            let rows = statement
                .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, bool>(2)?)))
                .map_err(|e| error("读取媒体路径", e))?
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(|e| error("读取媒体路径", e))?;
            drop(statement);
            for (id, value, has_envelope) in rows {
                let updated = rewrite_value(&value, has_envelope, media_path, decrypt, encrypt)?;
                if updated != value {
                    transaction
                        .execute(
                            &format!("UPDATE {table} SET {column} = ?1 WHERE id = ?2"),
                            rusqlite::params![updated, id],
                        )
                        .map_err(|e| error("更新媒体路径", e))?;
                }
            }
        }
    }
    let has_queue: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='pending_attachment_deletions')",
        [], |row| row.get(0),
    ).map_err(|e| error("检查待清理附件", e))?;
    if has_queue {
        let mut stmt = transaction.prepare("SELECT path FROM pending_attachment_deletions")
            .map_err(|e| error("读取待清理附件", e))?;
        let paths = stmt.query_map([], |row| row.get::<_, String>(0))
            .and_then(|rows| rows.collect::<rusqlite::Result<Vec<_>>>())
            .map_err(|e| error("读取待清理附件", e))?;
        for path in paths {
            let updated = media_path(&path);
            if updated != path {
                transaction.execute("UPDATE pending_attachment_deletions SET path = ?1 WHERE path = ?2", [&updated, &path])
                    .map_err(|e| error("更新待清理附件", e))?;
            }
        }
    }
    let settings_exist: bool = transaction
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='settings')",
            [],
            |r| r.get(0),
        )
        .map_err(|e| error("检查设置", e))?;
    if settings_exist {
        let value: Option<String> = transaction
            .query_row(
                "SELECT value FROM settings WHERE key='app.custom_background'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| error("读取背景路径", e))?;
        if let Some(value) = value {
            let updated = rewrite_value(&value, true, |value| rewrite_text(value, source, destination), decrypt, encrypt)?;
            if updated != value {
                transaction
                    .execute(
                        "UPDATE settings SET value=?1 WHERE key='app.custom_background'",
                        [updated],
                    )
                    .map_err(|e| error("更新背景路径", e))?;
            }
        }
    }
    transaction.commit().map_err(|e| error("提交路径迁移", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Folder(PathBuf);
    impl Folder {
        fn new() -> Self {
            let path = std::env::temp_dir()
                .join(format!("dewclip-directory-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn default_path(&self) -> PathBuf {
            self.0.join("io.github.rexkev214.dewclip")
        }
        fn custom_path(&self) -> PathBuf {
            self.0.join("custom")
        }
    }
    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn requested_move_snapshots_latest_wal_and_rewrites_owned_paths() {
        let folder = Folder::new();
        let source = folder.default_path();
        let target = folder.custom_path();
        let config = folder.0.join("config");
        for path in [&source, &target, &config] {
            std::fs::create_dir(path).unwrap();
        }
        let conn = Connection::open(source.join("clipboard.db")).unwrap();
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA wal_autocheckpoint = 0;").unwrap();
        crate::infrastructure::repository::migrations::run_migrations(&conn).unwrap();
        request_move(&config, &source, &target).unwrap();
        let attachments = source.join("attachments");
        std::fs::create_dir(&attachments).unwrap();
        let image = attachments.join("img_1.png");
        std::fs::write(&image, b"attachment").unwrap();
        let favorite_path = "saved/image.png";
        let favorite_file = source.join("favorites").join(favorite_path);
        std::fs::create_dir_all(favorite_file.parent().unwrap()).unwrap();
        std::fs::write(&favorite_file, b"favorite attachment").unwrap();
        let background = source.join("background.png");
        std::fs::write(&background, b"background").unwrap();
        let master_key = [42u8; 32];
        std::fs::write(source.join("master.key"), master_key).unwrap();
        let image_path = image.to_str().unwrap();
        let html = |path: &Path| format!("<img src=\"file://{}\">", path.display());
        conn.execute(
            "INSERT INTO clipboard_history (content_type, content, html_content, source_app, timestamp, preview, is_external)
             VALUES ('image', ?1, ?2, 'Preview', 1, 'Image', 1),
                    ('text', ?1, NULL, 'Notes', 2, 'Path as text', 0)",
            rusqlite::params![image_path, html(&image)],
        ).unwrap();
        conn.execute(
            "INSERT INTO favorites (content_type, content, preview, source_app, sort_order, created_at, updated_at)
             VALUES ('image', ?1, 'Saved image', 'Preview', 1, 1, 1)",
            [favorite_path],
        ).unwrap();
        conn.execute(
            "INSERT INTO settings (key, value) VALUES ('app.custom_background', ?1)",
            [background.to_str().unwrap()],
        ).unwrap();
        conn.execute("INSERT INTO pending_attachment_deletions VALUES (?1, 120000)", [image_path]).unwrap();
        assert!(!config.join("datapath.txt").exists());
        let moved = apply_requested_move(&config, &source).unwrap().unwrap();
        assert_eq!(moved, target.canonicalize().unwrap());
        assert_eq!(redirected_directory(&config).unwrap(), moved);
        let new_db = Connection::open_with_flags(
            moved.join("clipboard.db"),
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        ).unwrap();
        let expected_path = moved.join("attachments/img_1.png");
        let read_text = |query| new_db.query_row(query, [], |row| row.get::<_, String>(0)).unwrap();
        assert_eq!(
            read_text("SELECT content FROM clipboard_history WHERE content_type = 'image'"),
            expected_path.to_str().unwrap()
        );
        assert_eq!(
            read_text("SELECT html_content FROM clipboard_history WHERE content_type = 'image'"),
            html(&expected_path)
        );
        assert_eq!(
            read_text("SELECT content FROM clipboard_history WHERE content_type = 'text'"),
            image_path
        );
        assert_eq!(
            read_text("SELECT path FROM pending_attachment_deletions"),
            expected_path.to_str().unwrap()
        );
        assert_eq!(read_text("SELECT content FROM favorites"), favorite_path);
        assert_eq!(
            std::fs::read(moved.join("favorites").join(favorite_path)).unwrap(),
            b"favorite attachment"
        );
        let moved_background = moved.join("background.png");
        assert_eq!(
            read_text("SELECT value FROM settings WHERE key = 'app.custom_background'"),
            moved_background.to_str().unwrap()
        );
        assert_eq!(std::fs::read(moved_background).unwrap(), b"background");
        assert_eq!(std::fs::read(moved.join("master.key")).unwrap(), master_key);
        assert_eq!(std::fs::read(expected_path).unwrap(), b"attachment");
        assert_eq!(std::fs::read(image).unwrap(), b"attachment");
        assert_eq!(conn.query_row("SELECT count(*) FROM clipboard_history", [], |row| row.get::<_, i64>(0)).unwrap(), 2);
    }

    #[test]
    fn requested_move_rejects_a_corrupt_source_without_creating_an_empty_database() {
        let folder = Folder::new();
        let source = startup_directory(&folder.default_path()).unwrap();
        let target = folder.custom_path();
        std::fs::create_dir(&target).unwrap();
        let source_db = source.join("clipboard.db");
        drop(crate::database::init_db(source_db.to_str().unwrap()).unwrap());
        let mut corrupt = std::fs::read(&source_db).unwrap();
        corrupt[0] = 0;
        std::fs::write(&source_db, &corrupt).unwrap();

        request_move(&source, &source, &target).unwrap();
        assert!(apply_requested_move(&source, &source).is_err());
        assert!(!source.join("datapath.txt").exists());
        assert_eq!(std::fs::read_dir(&target).unwrap().count(), 0);
        assert_eq!(std::fs::read(source_db).unwrap(), corrupt);
    }

    #[test]
    fn requested_move_refuses_existing_db_and_rolls_back_on_unreadable_paths() {
        let folder = Folder::new();
        let source = folder.default_path();
        let target = folder.custom_path();
        let config = folder.0.join("config");
        for path in [&source, &target, &config] {
            std::fs::create_dir(path).unwrap();
        }
        let target_db = target.join("clipboard.db");
        std::fs::write(&target_db, b"owned by something else").unwrap();
        assert!(request_move(&config, &source, &target).is_err());
        assert_eq!(std::fs::read(&target_db).unwrap(), b"owned by something else");
        std::fs::remove_file(target_db).unwrap();
        let conn = Connection::open(source.join("clipboard.db")).unwrap();
        crate::infrastructure::repository::migrations::run_migrations(&conn).unwrap();
        conn.execute_batch(
            "INSERT INTO clipboard_history (content_type, content, source_app, timestamp, preview, is_external)
             VALUES ('image', 'dpapi:unreadable', 'Preview', 1, 'Image', 1);",
        ).unwrap();
        request_move(&config, &source, &target).unwrap();
        assert!(apply_requested_move(&config, &source).is_err());
        assert_eq!(std::fs::read_dir(&target).unwrap().count(), 0);
        assert!(!config.join("datapath.txt").exists());
        assert_eq!(conn.query_row("SELECT content FROM clipboard_history", [], |row| row.get::<_, String>(0)).unwrap(), "dpapi:unreadable");
    }

    #[test]
    fn starts_with_an_unmarked_default_database_and_can_move_it_from_settings() {
        let folder = Folder::new();
        let default = folder.default_path();
        let target = folder.custom_path();
        let attachments = default.join("attachments");
        std::fs::create_dir_all(&attachments).unwrap();
        std::fs::create_dir(&target).unwrap();
        let image = attachments.join("existing.png");
        std::fs::write(&image, b"existing attachment").unwrap();
        let path = default.join("clipboard.db");
        let writer = crate::database::init_db(path.to_str().unwrap()).unwrap();
        writer.execute_batch("PRAGMA journal_mode = WAL; PRAGMA wal_autocheckpoint = 0;").unwrap();
        writer.execute(
            "INSERT INTO clipboard_history (content_type, content, source_app, timestamp, preview, is_external)
             VALUES ('image', ?1, 'Preview', 1, 'Existing image', 1)",
            [image.to_str().unwrap()],
        ).unwrap();
        assert!(default.join("clipboard.db-wal").is_file());
        assert!(!default.join(OWNER).exists());

        let active = startup_directory(&default).unwrap();
        assert_eq!(active, default);
        let db = crate::database::init_db(active.join("clipboard.db").to_str().unwrap()).unwrap();
        let saved_image = |conn: &Connection| {
            conn.query_row("SELECT content FROM clipboard_history", [], |row| row.get::<_, String>(0))
                .unwrap()
        };
        assert_eq!(saved_image(&db), image.to_str().unwrap());
        assert_eq!(std::fs::read(&image).unwrap(), b"existing attachment");
        assert!(!default.join(OWNER).exists());
        drop(db);
        drop(writer);

        request_move(&default, &active, &target).unwrap();
        let moved = apply_requested_move(&default, &active).unwrap().unwrap();
        assert_eq!(startup_directory(&default).unwrap(), moved);
        let db = crate::database::init_db(moved.join("clipboard.db").to_str().unwrap()).unwrap();
        let moved_image = moved.join("attachments/existing.png");
        assert_eq!(saved_image(&db), moved_image.to_str().unwrap());
        assert_eq!(std::fs::read(&moved_image).unwrap(), b"existing attachment");
        assert!(default.join("clipboard.db").is_file());
        assert_eq!(std::fs::read(image).unwrap(), b"existing attachment");
    }

    #[test]
    fn unavailable_saved_directory_is_rejected() {
        let folder = Folder::new();
        let default = folder.default_path();
        std::fs::create_dir(&default).unwrap();
        std::fs::write(default.join("datapath.txt"), folder.custom_path().to_str().unwrap()).unwrap();
        assert!(startup_directory(&default).is_err());
        assert!(!default.join("clipboard.db").exists());
    }

    #[test]
    fn session_startup_preserves_an_unmarked_directory() {
        let folder = Folder::new();
        let original = folder.0.join("session");
        std::fs::create_dir(&original).unwrap();
        std::fs::write(original.join("user-file"), b"keep").unwrap();
        prepare_session(&folder.0).unwrap();
        let session = session_storage_root(&folder.0);
        assert_ne!(session, original);
        assert!(owns_session(&session));
        prepare_session(&folder.0).unwrap();
        assert_eq!(session_storage_root(&folder.0), session);
        assert_eq!(std::fs::read(original.join("user-file")).unwrap(), b"keep");
    }

    #[test]
    fn rewrites_encoded_paths_without_rewriting_neighbor_directories() {
        let old = Path::new("/Users/A B/io.github.rexkev214.dewclip");
        let new = Path::new("/Users/A B/custom");
        assert_eq!(
            rewrite_text(
                "file:///Users/A%20B/io.github.rexkev214.dewclip/attachments/a.png",
                old,
                new
            ),
            "file:///Users/A%20B/custom/attachments/a.png"
        );
        assert_eq!(
            rewrite_text("/Users/A B/io.github.rexkev214.dewclip-other/a", old, new),
            "/Users/A B/io.github.rexkev214.dewclip-other/a"
        );
    }
    #[cfg(not(feature = "portable"))]
    #[test]
    fn requested_move_rewrites_encrypted_media_paths() {
        use crate::infrastructure::encryption;
        let f = Folder::new();
        let old = f.default_path();
        let new = f.custom_path();
        std::fs::create_dir_all(old.join("attachments")).unwrap();
        std::fs::create_dir(&new).unwrap();
        std::fs::write(old.join("attachments/secret.png"), b"attachment").unwrap();
        let conn = Connection::open(old.join("clipboard.db")).unwrap();
        crate::infrastructure::repository::migrations::run_migrations(&conn).unwrap();
        let encrypted =
            encryption::encrypt_for_storage(&old.join("attachments/secret.png").to_string_lossy()).unwrap();
        conn.execute(
            "INSERT INTO clipboard_history (content_type, content, source_app, timestamp, preview, is_external)
             VALUES ('image', ?1, 'Preview', 1, 'Image', 1)",
            [encrypted],
        )
        .unwrap();
        drop(conn);
        request_move(&old, &old, &new).unwrap();
        let moved = apply_requested_move(&old, &old).unwrap().unwrap();
        let conn = Connection::open(new.join("clipboard.db")).unwrap();
        let value: String = conn
            .query_row("SELECT content FROM clipboard_history", [], |r| r.get(0))
            .unwrap();
        assert!(value.starts_with("enc1:"));
        assert_eq!(
            encryption::decrypt_stored_value(&value).unwrap(),
            moved.join("attachments/secret.png").to_string_lossy()
        );
    }
}
