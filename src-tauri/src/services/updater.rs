mod http;

use crate::app_state::SettingsState;
use crate::database::DbState;
use crate::error::{AppError, AppResult};
use crate::infrastructure::repository::settings_repo::{
    SettingsRepository, SqliteSettingsRepository,
};
use crate::services::recording_pause::wall_clock_ms;
use http::{newer_than, CheckResult, Failure};
use rusqlite::{params, Connection};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::sync::{atomic::Ordering, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::watch;

pub const ENABLED_KEY: &str = "app.update_check_enabled";
pub const ONBOARDING_DONE_KEY: &str = "app.onboarding_done";
const CHECKED_KEY: &str = "app.update_last_checked_at";
const SKIPPED_KEY: &str = "app.update_skipped_version";
const ETAG_KEY: &str = "app.update_etag";
const RELEASE_KEY: &str = "app.update_cached_release";
const STATUS_EVENT: &str = "update-status-changed";
const CHECK_INTERVAL_MS: i64 = 24 * 60 * 60 * 1000;
const FAILURE_BACKOFF: Duration = Duration::from_secs(3600);
const WAKE_DELAY: Duration = Duration::from_secs(10);

pub fn is_internal_setting(key: &str) -> bool {
    matches!(key, CHECKED_KEY | SKIPPED_KEY | ETAG_KEY | RELEASE_KEY)
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    version: Version,
    release_url: String,
    notes_preview: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckError {
    Offline,
    Timeout,
    RateLimited,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Status {
    Idle,
    UpToDate,
    Available,
    Error,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSnapshot {
    current_version: Version,
    checking: bool,
    status: Status,
    latest: Option<Release>,
    last_checked_at: Option<i64>,
    skipped_version: Option<Version>,
    available: bool,
    banner_dismissed: bool,
    error_kind: Option<CheckError>,
    retry_after_secs: Option<u64>,
}

#[derive(Default)]
struct CachedCheck {
    checked_at: Option<i64>,
    release: Option<Release>,
    etag: Option<String>,
}

impl CachedCheck {
    fn load(conn: &Connection) -> rusqlite::Result<Self> {
        let checked_at = SqliteSettingsRepository::get_raw(conn, CHECKED_KEY)?
            .and_then(|value| value.parse().ok());
        let release = SqliteSettingsRepository::get_raw(conn, RELEASE_KEY)?
            .filter(|json| json.len() <= 2048)
            .and_then(|json| serde_json::from_str::<Option<Release>>(&json).ok());
        let etag = if release.is_some() && checked_at.is_some() {
            SqliteSettingsRepository::get_raw(conn, ETAG_KEY)?.filter(|etag| http::valid_etag(etag))
        } else {
            None
        };
        let release = release
            .flatten()
            .filter(|release| release.version.pre.is_empty())
            .map(|mut release| {
                release.release_url = http::release_url(&release.release_url, http::RELEASES);
                release
            });
        Ok(Self {
            checked_at,
            release,
            etag,
        })
    }

    fn save(&self, conn: &mut Connection) -> AppResult<()> {
        let tx = conn.transaction()?;
        let checked_at = self
            .checked_at
            .expect("time of completed update check")
            .to_string();
        let release = serde_json::to_string(&self.release)
            .map_err(|error| AppError::Internal(error.to_string()))?;
        for (key, value) in [
            (CHECKED_KEY, checked_at.as_str()),
            (RELEASE_KEY, release.as_str()),
        ] {
            tx.execute(
                "INSERT OR REPLACE INTO settings (key, value) VALUES (?, ?)",
                params![key, value],
            )?;
        }
        if let Some(etag) = &self.etag {
            tx.execute(
                "INSERT OR REPLACE INTO settings (key, value) VALUES (?, ?)",
                params![ETAG_KEY, etag],
            )?;
        } else {
            tx.execute("DELETE FROM settings WHERE key = ?", [ETAG_KEY])?;
        }
        tx.commit()?;
        Ok(())
    }

    fn due(&self, now: i64) -> bool {
        self.checked_at
            .is_none_or(|last| now < last || now.saturating_sub(last) >= CHECK_INTERVAL_MS)
    }
}

type CheckReceiver = watch::Receiver<Option<UpdateSnapshot>>;

struct Request {
    etag: Option<String>,
    generation: u64,
    result: watch::Sender<Option<UpdateSnapshot>>,
}

#[derive(Default)]
struct UpdateProgress {
    cache: CachedCheck,
    onboarding_done: bool,
    skipped: Option<Version>,
    dismissed: Option<Version>,
    failure: Option<Failure>,
    failed_at_ms: Option<i64>,
    wake_until: Option<Instant>,
    in_flight: Option<CheckReceiver>,
    generation: u64,
}

impl UpdateProgress {
    fn save_onboarding_done(
        &mut self,
        settings: &impl SettingsRepository,
        value: &str,
    ) -> AppResult<()> {
        settings.set(ONBOARDING_DONE_KEY, value)?;
        self.onboarding_done = value == "true";
        Ok(())
    }

    fn latest(&self, current: &Version) -> Option<&Release> {
        self.cache
            .release
            .as_ref()
            .filter(|release| newer_than(&release.version, current))
    }

    fn available(&self, current: &Version) -> Option<&Release> {
        self.latest(current).filter(|release| {
            self.skipped
                .as_ref()
                .is_none_or(|skipped| newer_than(&release.version, skipped))
        })
    }

    fn snapshot(&self, current: &Version) -> UpdateSnapshot {
        let latest = self.latest(current);
        let status = if self.failure.is_some() {
            Status::Error
        } else if latest.is_some() {
            Status::Available
        } else if self.cache.checked_at.is_some() {
            Status::UpToDate
        } else {
            Status::Idle
        };
        UpdateSnapshot {
            current_version: current.clone(),
            checking: self.in_flight.is_some(),
            status,
            latest: latest.cloned(),
            last_checked_at: self.cache.checked_at,
            skipped_version: self.skipped.clone(),
            available: self.available(current).is_some(),
            banner_dismissed: latest
                .is_some_and(|release| Some(&release.version) == self.dismissed.as_ref()),
            error_kind: self.failure.as_ref().map(|failure| failure.kind),
            retry_after_secs: self
                .failure
                .as_ref()
                .and_then(|failure| failure.retry_after)
                .map(|delay| delay.as_secs()),
        }
    }

    fn begin(
        &mut self,
        manual: bool,
        now_ms: i64,
        now: Instant,
    ) -> Option<(CheckReceiver, Option<Request>)> {
        if let Some(receiver) = &self.in_flight {
            return manual.then(|| (receiver.clone(), None));
        }
        if !manual
            && (!self.onboarding_done
                || self.backing_off(now_ms)
                || !self.cache.due(now_ms)
                || self.wake_until.is_some_and(|until| now < until))
        {
            return None;
        }
        let (result, receiver) = watch::channel(None);
        self.in_flight = Some(receiver.clone());
        Some((
            receiver,
            Some(Request {
                etag: self.cache.etag.clone(),
                generation: self.generation,
                result,
            }),
        ))
    }

    fn backing_off(&mut self, now_ms: i64) -> bool {
        let Some(failed_at) = &mut self.failed_at_ms else {
            return false;
        };
        // Start a fresh delay on rollback instead of waiting for the old clock time.
        *failed_at = (*failed_at).min(now_ms);
        let elapsed = Duration::from_millis(now_ms.saturating_sub(*failed_at) as u64);
        let backoff = self
            .failure
            .as_ref()
            .and_then(|failure| failure.retry_after)
            .unwrap_or_default()
            .max(FAILURE_BACKOFF);
        elapsed < backoff
    }

    fn finish(&mut self, result: Result<CheckResult, Failure>, now: i64, conn: &mut Connection) {
        let checked_at = Some(now);
        let next = result.and_then(|checked| {
            let cache = match checked {
                CheckResult::Fresh { release, etag } => CachedCheck {
                    checked_at,
                    release,
                    etag,
                },
                CheckResult::Unchanged if self.cache.etag.is_some() => CachedCheck {
                    checked_at,
                    release: self.cache.release.clone(),
                    etag: self.cache.etag.clone(),
                },
                CheckResult::Unchanged => {
                    return Err(Failure::new(CheckError::Failed, "304 without an ETag"))
                }
            };
            cache.save(conn).map_err(|error| {
                crate::error!("Could not store update status: {error}");
                Failure::new(CheckError::Failed, "could not store update status")
            })?;
            Ok(cache)
        });
        match next {
            Ok(cache) => {
                self.cache = cache;
                self.failure = None;
                self.failed_at_ms = None;
                crate::info!("Update check completed");
            }
            Err(failure) => {
                crate::error!("Update check: {}", failure.detail);
                self.failure = Some(failure);
                self.failed_at_ms = Some(now);
            }
        }
    }

    fn reset(&mut self, conn: &mut Connection) -> AppResult<()> {
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM settings", [])?;
        crate::database::seed_defaults(&tx)?;
        tx.commit()?;
        *self = Self {
            // A request begun before reset may finish, but must not write the old settings back.
            generation: self.generation.wrapping_add(1),
            in_flight: self.in_flight.take(),
            ..Self::default()
        };
        Ok(())
    }
}

pub struct UpdateState {
    current: Version,
    progress: Mutex<UpdateProgress>,
}

impl UpdateState {
    pub fn load(current: Version, conn: &Connection) -> rusqlite::Result<Self> {
        let skipped = SqliteSettingsRepository::get_raw(conn, SKIPPED_KEY)?;
        let version = skipped
            .as_deref()
            .and_then(http::parse_version)
            .filter(|skipped| newer_than(skipped, &current));
        if skipped.is_some() && version.is_none() {
            conn.execute("DELETE FROM settings WHERE key = ?", [SKIPPED_KEY])?;
        }
        Ok(Self {
            current,
            progress: Mutex::new(UpdateProgress {
                cache: CachedCheck::load(conn)?,
                onboarding_done: SqliteSettingsRepository::get_raw(conn, ONBOARDING_DONE_KEY)?
                    .as_deref() == Some("true"),
                skipped: version,
                ..Default::default()
            }),
        })
    }

    pub fn available_version(&self) -> Option<String> {
        self.progress
            .lock()
            .unwrap()
            .available(&self.current)
            .map(|release| release.version.to_string())
    }
}

fn publish(app: &AppHandle, snapshot: &UpdateSnapshot) {
    if let Err(error) = app.emit_to("main", STATUS_EVENT, snapshot) {
        crate::error!("Could not publish update status: {error}");
    }
}

fn begin(app: &AppHandle, manual: bool) -> Option<CheckReceiver> {
    let state = app.state::<UpdateState>();
    let mut progress = state.progress.lock().unwrap();
    if !manual
        && (cfg!(debug_assertions)
            || !app
                .state::<SettingsState>()
                .update_check_enabled
                .load(Ordering::Relaxed))
    {
        return None;
    }
    let (receiver, request) = progress.begin(manual, wall_clock_ms(), Instant::now())?;
    if let Some(request) = request {
        publish(app, &progress.snapshot(&state.current));
        tauri::async_runtime::spawn(run(app.clone(), request));
    }
    Some(receiver)
}

async fn run(app: AppHandle, request: Request) {
    let Request {
        etag,
        generation,
        result,
    } = request;
    let response =
        tauri::async_runtime::spawn_blocking(move || http::fetch(http::ENDPOINT, etag.as_deref()))
            .await;
    let now = wall_clock_ms();
    let checked = match response {
        Ok(response) => response.interpret(http::RELEASES, now),
        Err(_) => Err(Failure::new(
            CheckError::Failed,
            "native request worker failed",
        )),
    };
    let state = app.state::<UpdateState>();
    {
        let mut progress = state.progress.lock().unwrap();
        if generation == progress.generation {
            let db = app.state::<DbState>();
            progress.finish(checked, now, &mut db.conn.lock().unwrap());
        }
        progress.in_flight = None;
        let snapshot = progress.snapshot(&state.current);
        publish(&app, &snapshot);
        result.send_replace(Some(snapshot));
    }
    crate::app::tray::refresh_tray(&app);
}

pub fn start_scheduler(app: AppHandle, first_open: bool) {
    if cfg!(debug_assertions) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(20)).await;
        if !first_open {
            begin(&app, false);
        }
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            begin(&app, false);
        }
    });
}

pub fn set_enabled(app: &AppHandle, enabled: bool) -> AppResult<()> {
    {
        let state = app.state::<UpdateState>();
        let _progress = state.progress.lock().unwrap();
        app.state::<DbState>()
            .settings_repo
            .set(ENABLED_KEY, &enabled.to_string())?;
        app.state::<SettingsState>()
            .update_check_enabled
            .store(enabled, Ordering::Relaxed);
    }
    if enabled {
        begin(app, false);
    }
    Ok(())
}

pub fn set_onboarding_done(app: &AppHandle, value: &str) -> AppResult<()> {
    let state = app.state::<UpdateState>();
    let mut progress = state.progress.lock().unwrap();
    progress.save_onboarding_done(&app.state::<DbState>().settings_repo, value)
}

pub fn check_after_wake(app: AppHandle) {
    if cfg!(debug_assertions) {
        return;
    }
    let Some(state) = app.try_state::<UpdateState>() else {
        return;
    };
    state.progress.lock().unwrap().wake_until = Some(Instant::now() + WAKE_DELAY);
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(WAKE_DELAY).await;
        begin(&app, false);
    });
}

pub fn reset_settings(app: &AppHandle) -> AppResult<()> {
    let state = app.state::<UpdateState>();
    let mut progress = state.progress.lock().unwrap();
    let db = app.state::<DbState>();
    let mut conn = db.conn.lock().unwrap();
    progress.reset(&mut conn)?;
    publish(app, &progress.snapshot(&state.current));
    Ok(())
}

#[tauri::command]
pub fn get_update_state(state: State<'_, UpdateState>) -> UpdateSnapshot {
    state.progress.lock().unwrap().snapshot(&state.current)
}

#[tauri::command]
pub async fn check_for_updates_now(app: AppHandle) -> AppResult<UpdateSnapshot> {
    let mut receiver = begin(&app, true).expect("a manual check starts or joins a request");
    let completed = receiver
        .wait_for(Option::is_some)
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?;
    Ok(completed.as_ref().expect("completed request").clone())
}

#[tauri::command]
pub fn skip_update_version(app: AppHandle, version: Version) -> AppResult<UpdateSnapshot> {
    let state = app.state::<UpdateState>();
    let snapshot = {
        let mut progress = state.progress.lock().unwrap();
        if !progress
            .latest(&state.current)
            .is_some_and(|release| release.version == version)
        {
            return Err(AppError::Validation("update_failed".into()));
        }
        if progress
            .skipped
            .as_ref()
            .is_none_or(|skipped| newer_than(&version, skipped))
        {
            app.state::<DbState>()
                .settings_repo
                .set(SKIPPED_KEY, &version.to_string())?;
            progress.skipped = Some(version);
        }
        let snapshot = progress.snapshot(&state.current);
        publish(&app, &snapshot);
        snapshot
    };
    crate::app::tray::refresh_tray(&app);
    Ok(snapshot)
}

#[tauri::command]
pub fn dismiss_update_banner(app: AppHandle, version: Version) -> UpdateSnapshot {
    let state = app.state::<UpdateState>();
    let mut progress = state.progress.lock().unwrap();
    if progress
        .latest(&state.current)
        .is_some_and(|release| release.version == version)
    {
        progress.dismissed = Some(version);
    }
    let snapshot = progress.snapshot(&state.current);
    publish(&app, &snapshot);
    snapshot
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE settings(key TEXT PRIMARY KEY, value TEXT NOT NULL);")
            .unwrap();
        conn
    }

    fn release(version: &str) -> Release {
        Release {
            version: Version::parse(version).unwrap(),
            release_url: format!("{}tag/v{version}", http::RELEASES),
            notes_preview: "Release notes".into(),
        }
    }

    #[test]
    fn automatic_checks_wait_until_due_and_until_the_network_has_had_time_after_wake() {
        let checked_at = 100_000_000;
        let now = Instant::now();
        let mut progress = UpdateProgress {
            onboarding_done: true,
            cache: CachedCheck {
                checked_at: Some(checked_at),
                ..Default::default()
            },
            ..Default::default()
        };
        let due = checked_at + CHECK_INTERVAL_MS;
        assert!(progress.cache.due(checked_at - 1));
        assert!(progress.begin(false, due - 1, now).is_none());
        progress.wake_until = Some(now + WAKE_DELAY);
        assert!(progress.begin(false, due, now).is_none());
        assert!(progress.begin(false, due, now + WAKE_DELAY).is_some());
    }

    #[test]
    fn failure_backoff_counts_wall_clock_time_even_when_monotonic_time_stands_still() {
        let mut conn = settings_db();
        let mut progress = UpdateProgress {
            onboarding_done: true,
            ..Default::default()
        };
        let now = Instant::now();
        let failed_at = 100_000_000;
        progress.finish(
            Err(Failure::new(CheckError::Offline, "offline")),
            failed_at,
            &mut conn,
        );
        let due = failed_at + FAILURE_BACKOFF.as_millis() as i64;
        assert!(progress.begin(false, due - 1, now).is_none());
        assert!(progress.begin(false, due, now).is_some());
    }

    #[test]
    fn rate_limit_backoff_starts_fresh_when_the_clock_moves_backwards() {
        let mut conn = settings_db();
        let mut progress = UpdateProgress {
            onboarding_done: true,
            ..Default::default()
        };
        let now = Instant::now();
        let failed_at = 100_000_000;
        let retry_after = FAILURE_BACKOFF * 2;
        progress.finish(
            Err(Failure {
                kind: CheckError::RateLimited,
                retry_after: Some(retry_after),
                detail: "GitHub rate limit",
            }),
            failed_at,
            &mut conn,
        );
        let rolled_back = failed_at - CHECK_INTERVAL_MS;
        assert!(progress.begin(false, rolled_back, now).is_none());
        let due = rolled_back + retry_after.as_millis() as i64;
        assert!(progress.begin(false, due - 1, now).is_none());
        assert!(progress.begin(false, due, now).is_some());
    }

    #[test]
    fn automatic_checks_use_in_memory_onboarding_state_until_settings_change() {
        let conn = std::sync::Arc::new(Mutex::new(settings_db()));
        let settings = SqliteSettingsRepository::new(conn.clone());
        let current = Version::new(1, 0, 0);
        let state = UpdateState::load(current.clone(), &conn.lock().unwrap()).unwrap();
        let mut progress = state.progress.into_inner().unwrap();
        let now = Instant::now();
        assert!(progress.begin(false, 1, now).is_none());

        progress.save_onboarding_done(&settings, "true").unwrap();
        let state = UpdateState::load(current, &conn.lock().unwrap()).unwrap();
        assert!(state.progress.lock().unwrap().onboarding_done);
        let mut conn = conn.lock().unwrap();
        assert!(progress.begin(false, 1, now).is_some());
        progress.reset(&mut conn).unwrap();
        progress.in_flight = None;
        assert!(progress.begin(false, 2, now).is_none());
    }

    #[test]
    fn concurrent_manual_checks_join_the_same_in_flight_auto_check() {
        let mut progress = UpdateProgress {
            onboarding_done: true,
            ..Default::default()
        };
        let now = Instant::now();
        let (automatic, request) = progress.begin(false, 1, now).unwrap();
        let request = request.unwrap();
        let (manual, next) = progress.begin(true, 2, now).unwrap();
        assert!(next.is_none());
        assert!(automatic.same_channel(&manual));
        assert!(progress.begin(false, 2, now).is_none());
        let (manual_again, next) = progress.begin(true, 2, now).unwrap();
        assert!(next.is_none());
        assert!(manual_again.same_channel(&request.result.subscribe()));
    }

    #[test]
    fn successful_checks_persist_cache_and_failures_leave_it_intact() {
        let mut conn = settings_db();
        let mut progress = UpdateProgress::default();
        progress.finish(
            Ok(CheckResult::Fresh {
                release: Some(release("1.2.0")),
                etag: Some("\"release\"".into()),
            }),
            10,
            &mut conn,
        );
        let cached = CachedCheck::load(&conn).unwrap();
        assert_eq!(cached.checked_at, Some(10));
        assert_eq!(cached.etag.as_deref(), Some("\"release\""));
        assert_eq!(cached.release.unwrap().version, Version::new(1, 2, 0));

        progress.finish(Ok(CheckResult::Unchanged), 20, &mut conn);
        progress.finish(
            Err(Failure::new(CheckError::Offline, "offline")),
            30,
            &mut conn,
        );
        let current = Version::new(1, 0, 0);
        assert_eq!(CachedCheck::load(&conn).unwrap().checked_at, Some(20));
        assert_eq!(progress.snapshot(&current).status, Status::Error);
        assert!(progress.snapshot(&current).available);

        progress.finish(
            Ok(CheckResult::Fresh {
                release: None,
                etag: None,
            }),
            40,
            &mut conn,
        );
        let snapshot = progress.snapshot(&current);
        assert_eq!(
            serde_json::to_value(&snapshot).unwrap()["currentVersion"],
            "1.0.0"
        );
        assert_eq!(snapshot.status, Status::UpToDate);
        assert_eq!(snapshot.last_checked_at, Some(40));
        assert!(!snapshot.available);
        assert!(CachedCheck::load(&conn).unwrap().etag.is_none());
        progress.finish(Ok(CheckResult::Unchanged), 50, &mut conn);
        assert_eq!(progress.snapshot(&current).last_checked_at, Some(40));
    }

    #[test]
    fn skip_and_dismiss_apply_to_the_version_and_manual_checks_still_show_it() {
        let current = Version::new(1, 0, 0);
        let mut progress = UpdateProgress {
            cache: CachedCheck {
                release: Some(release("1.2.0")),
                ..Default::default()
            },
            skipped: Some(Version::new(1, 2, 0)),
            dismissed: Some(Version::new(1, 2, 0)),
            ..Default::default()
        };
        let snapshot = progress.snapshot(&current);
        assert!(!snapshot.available);
        assert!(snapshot.banner_dismissed);
        assert_eq!(snapshot.status, Status::Available);
        assert_eq!(snapshot.latest.unwrap().version, Version::new(1, 2, 0));
        progress.cache.release = Some(release("1.2.0+rebuilt"));
        assert!(!progress.snapshot(&current).available);
        progress.cache.release = Some(release("1.3.0"));
        assert!(progress.snapshot(&current).available);
        assert!(!progress.snapshot(&current).banner_dismissed);
    }

    #[test]
    fn upgrade_clears_obsolete_skip_and_seeds_enabled_without_overwriting_a_choice() {
        let conn = settings_db();
        conn.execute(
            "INSERT INTO settings VALUES ('app.hotkey', 'Alt+X'), (?1, '1.2.0+old')",
            [SKIPPED_KEY],
        )
        .unwrap();
        crate::database::seed_defaults(&conn).unwrap();
        assert_eq!(
            SqliteSettingsRepository::get_raw(&conn, ENABLED_KEY)
                .unwrap()
                .as_deref(),
            Some("true")
        );
        conn.execute(
            "UPDATE settings SET value = 'false' WHERE key = ?",
            [ENABLED_KEY],
        )
        .unwrap();
        crate::database::seed_defaults(&conn).unwrap();
        assert_eq!(
            SqliteSettingsRepository::get_raw(&conn, ENABLED_KEY)
                .unwrap()
                .as_deref(),
            Some("false")
        );
        let _ = UpdateState::load(Version::new(1, 2, 0), &conn).unwrap();
        assert!(SqliteSettingsRepository::get_raw(&conn, SKIPPED_KEY)
            .unwrap()
            .is_none());
    }

    #[test]
    fn reset_restores_defaults_and_invalidates_an_in_flight_result() {
        let mut conn = settings_db();
        let mut progress = UpdateProgress::default();
        progress.finish(
            Ok(CheckResult::Fresh {
                release: Some(release("1.2.0")),
                etag: Some("\"cached\"".into()),
            }),
            100,
            &mut conn,
        );
        conn.execute(
            "INSERT INTO settings VALUES (?1, 'false'), (?2, '1.2.0')",
            [ENABLED_KEY, SKIPPED_KEY],
        )
        .unwrap();
        let (_, request) = progress.begin(true, 101, Instant::now()).unwrap();
        progress.reset(&mut conn).unwrap();
        assert_ne!(request.unwrap().generation, progress.generation);
        assert!(progress.in_flight.is_some());
        assert!(progress.cache.checked_at.is_none());
        for key in [CHECKED_KEY, RELEASE_KEY, ETAG_KEY, SKIPPED_KEY] {
            assert!(is_internal_setting(key));
            assert!(SqliteSettingsRepository::get_raw(&conn, key)
                .unwrap()
                .is_none());
        }
        assert!(!is_internal_setting(ENABLED_KEY));
        assert_eq!(
            SqliteSettingsRepository::get_raw(&conn, ENABLED_KEY)
                .unwrap()
                .as_deref(),
            Some("true")
        );
    }
}
