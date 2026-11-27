mod cache;
mod download;
mod http;
mod install;

use crate::app_state::SettingsState;
use crate::database::DbState;
use crate::error::{AppError, AppResult};
use crate::infrastructure::repository::settings_repo::{
    SettingsRepository, SqliteSettingsRepository,
};
use crate::services::recording_pause::wall_clock_ms;
use cache::Ready;
pub(crate) use cache::UpdateCache;
use download::{Deferred, Transfer};
use http::{newer_than, CheckResult, Failure};
pub use install::{InstallError, InstallPreview};
use rusqlite::{params, Connection};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::sync::{atomic::Ordering, Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::watch;

pub const ENABLED_KEY: &str = "app.update_check_enabled";
pub const ONBOARDING_DONE_KEY: &str = "app.onboarding_done";
const CHECKED_KEY: &str = "app.update_last_checked_at";
const SKIPPED_KEY: &str = "app.update_skipped_version";
const ETAG_KEY: &str = "app.update_etag";
const RELEASE_KEY: &str = "app.update_cached_release";
const INSTALL_ATTEMPT_KEY: &str = "app.update_install_attempt";
const INSTALL_REMINDED_KEY: &str = "app.update_install_reminded";
const DOWNLOAD_RETRY_KEY: &str = "app.update_download_retry";
const STATUS_EVENT: &str = "update-status-changed";
const CHECK_INTERVAL_MS: i64 = 24 * 60 * 60 * 1000;
const FAILURE_BACKOFF: Duration = Duration::from_secs(3600);
const WAKE_DELAY: Duration = Duration::from_secs(10);

pub fn is_internal_setting(key: &str) -> bool {
    matches!(
        key,
        CHECKED_KEY
            | SKIPPED_KEY
            | ETAG_KEY
            | RELEASE_KEY
            | INSTALL_ATTEMPT_KEY
            | INSTALL_REMINDED_KEY
            | DOWNLOAD_RETRY_KEY
    )
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    version: Version,
    release_url: String,
    notes_preview: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    asset: Option<ReleaseAsset>,
}

impl Release {
    fn validate_cached(&mut self) {
        self.release_url = http::release_url(&self.release_url, http::RELEASES);
        self.asset = self
            .asset
            .take()
            .filter(|asset| http::valid_asset(asset, http::DOWNLOADS));
        if let Some(asset) = &mut self.asset {
            asset.sha256.make_ascii_lowercase();
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct ReleaseAsset {
    size: u64,
    sha256: String,
    download_url: String,
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
    notice_version: Option<Version>,
    ready: Option<Release>,
    download: Option<DownloadStatus>,
    incomplete_install: bool,
    banner_dismissed: bool,
    error_kind: Option<CheckError>,
    retry_after_secs: Option<u64>,
}

#[derive(Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum DownloadStatus {
    Downloading {
        version: Version,
        received: u64,
        total: u64,
    },
    Deferred {
        version: Version,
        reason: Deferred,
    },
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
            .filter(|json| json.len() as u64 <= cache::RELEASE_LIMIT)
            .and_then(|json| serde_json::from_str::<Option<Release>>(&json).ok());
        let etag = if release.is_some() && checked_at.is_some() {
            SqliteSettingsRepository::get_raw(conn, ETAG_KEY)?.filter(|etag| http::valid_etag(etag))
        } else {
            None
        };
        let release = release
            .flatten()
            .filter(|release| {
                release.version.pre.is_empty() && release.version.to_string().len() <= 128
            })
            .map(|mut release| {
                release.validate_cached();
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

struct ActiveDownload {
    release: Release,
    transfer: Arc<Transfer>,
    generation: u64,
    manual: bool,
}

#[derive(Deserialize, Serialize)]
struct DownloadDeferral {
    version: Version,
    reason: Deferred,
    retry_at_ms: i64,
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
    manual_check_requested: bool,
    active_download: Option<ActiveDownload>,
    queued_download: Option<Release>,
    deferred: Option<DownloadDeferral>,
    download_blocked: bool,
    ready: Option<Ready>,
    incomplete_install: bool,
    installing: bool,
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
        let remote = self
            .cache
            .release
            .as_ref()
            .filter(|release| newer_than(&release.version, current));
        let ready = self.ready.as_ref().map(|ready| &ready.release);
        match (remote, ready) {
            (Some(remote), Some(ready)) if newer_than(&ready.version, &remote.version) => {
                Some(ready)
            }
            (remote, ready) => remote.or(ready),
        }
    }

    fn eligible(&self, current: &Version) -> Option<&Release> {
        self.latest(current).filter(|release| {
            self.skipped
                .as_ref()
                .is_none_or(|skipped| newer_than(&release.version, skipped))
        })
    }

    fn available(&self, current: &Version) -> Option<&Release> {
        let latest = self.eligible(current).filter(|release| {
            !self.active_download.as_ref().is_some_and(|active| {
                active.generation == self.generation && active.release.version == release.version
            })
        });
        if let Some(ready) = &self.ready {
            return latest
                .filter(|release| {
                    newer_than(&release.version, &ready.release.version)
                        && (release.asset.is_none()
                            || self
                                .deferred
                                .as_ref()
                                .is_some_and(|deferred| deferred.version == release.version))
                })
                .or(Some(&ready.release));
        }
        latest
    }

    fn download_status(&self) -> Option<DownloadStatus> {
        if let Some(active) = &self.active_download {
            if active.generation == self.generation {
                return Some(DownloadStatus::Downloading {
                    version: active.release.version.clone(),
                    received: active.transfer.received.load(Ordering::Relaxed),
                    total: active
                        .release
                        .asset
                        .as_ref()
                        .expect("active download asset")
                        .size,
                });
            }
        }
        self.deferred
            .as_ref()
            .map(|deferred| DownloadStatus::Deferred {
                version: deferred.version.clone(),
                reason: deferred.reason,
            })
    }

    fn snapshot(&self, current: &Version) -> UpdateSnapshot {
        let latest = self.latest(current);
        let notice = self.available(current);
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
            latest: latest.cloned().map(|mut release| {
                if self.download_blocked {
                    release.asset = None;
                }
                release
            }),
            last_checked_at: self.cache.checked_at,
            skipped_version: self.skipped.clone(),
            available: notice.is_some(),
            notice_version: notice.map(|release| release.version.clone()),
            ready: self.ready.as_ref().map(|ready| ready.release.clone()),
            download: self.download_status(),
            incomplete_install: self.incomplete_install,
            banner_dismissed: notice
                .or(latest)
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
            self.manual_check_requested |= manual;
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
        self.manual_check_requested = manual;
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
                self.deferred = self.deferred.take().filter(|deferred| {
                    self.cache.release.as_ref().is_some_and(|release| {
                        release.asset.is_some() && release.version == deferred.version
                    })
                });
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

    fn cancel(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.queued_download = None;
        self.deferred = None;
        self.in_flight = None;
        self.manual_check_requested = false;
        if let Some(active) = &self.active_download {
            active.transfer.cancel();
        }
    }

    fn clear_downloads(&mut self, cache: &UpdateCache) -> std::io::Result<()> {
        self.cancel();
        self.ready = None;
        self.incomplete_install = false;
        cache.clear()
    }

    fn disable(&mut self, conn: &mut Connection, cache: &UpdateCache) -> AppResult<()> {
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES (?, 'false')",
            [ENABLED_KEY],
        )?;
        tx.execute(
            "DELETE FROM settings WHERE key IN (?, ?, ?, ?)",
            [CHECKED_KEY, ETAG_KEY, RELEASE_KEY, DOWNLOAD_RETRY_KEY],
        )?;
        tx.commit()?;
        self.cache = CachedCheck::default();
        self.failure = None;
        self.failed_at_ms = None;
        self.dismissed = None;
        self.clear_downloads(cache)?;
        Ok(())
    }

    fn reset(&mut self, conn: &mut Connection, cache: &UpdateCache) -> AppResult<()> {
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM settings", [])?;
        crate::database::seed_defaults(&tx)?;
        tx.commit()?;
        let cleared = self.clear_downloads(cache);
        *self = Self {
            generation: self.generation,
            active_download: self.active_download.take(),
            installing: self.installing,
            download_blocked: self.download_blocked,
            ..Self::default()
        };
        cleared?;
        Ok(())
    }

    fn take_incomplete_install(&mut self, conn: &Connection) -> rusqlite::Result<Option<Release>> {
        if !self.incomplete_install {
            return Ok(None);
        }
        let Some(ready) = &self.ready else {
            return Ok(None);
        };
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES (?, ?)",
            params![INSTALL_REMINDED_KEY, ready.release.version.to_string()],
        )?;
        self.incomplete_install = false;
        Ok(Some(ready.release.clone()))
    }

    fn can_download(&self, release: &Release, manual: bool, now_ms: i64) -> bool {
        !self.download_blocked
            && release.asset.is_some()
            && self
                .ready
                .as_ref()
                .is_none_or(|ready| newer_than(&release.version, &ready.release.version))
            && (manual
                || self.deferred.as_ref().is_none_or(|deferred| {
                    deferred.version != release.version || now_ms >= deferred.retry_at_ms
                }))
    }

    fn scheduled_download(&self, current: &Version, now_ms: i64) -> Option<Release> {
        if self.in_flight.is_some() || self.active_download.is_some() || self.installing {
            return None;
        }
        self.eligible(current)
            .filter(|release| self.can_download(release, false, now_ms))
            .cloned()
    }

    fn begin_download(
        &mut self,
        release: Release,
        manual: bool,
        now_ms: i64,
    ) -> Option<Arc<Transfer>> {
        if !self.can_download(&release, manual, now_ms) {
            return None;
        }
        if let Some(active) = &self.active_download {
            if manual
                && (active.generation != self.generation
                    || !active.manual
                    || active.release.version != release.version)
            {
                active.transfer.cancel();
                self.queued_download = Some(release);
            }
            return None;
        }
        if self.installing {
            if manual {
                self.queued_download = Some(release);
            }
            return None;
        }
        let transfer = Arc::new(Transfer::default());
        self.active_download = Some(ActiveDownload {
            release,
            transfer: transfer.clone(),
            generation: self.generation,
            manual,
        });
        Some(transfer)
    }

    fn finish_download(
        &mut self,
        cache: &UpdateCache,
        result: Result<cache::Part, Deferred>,
        conn: &Connection,
        now_ms: i64,
    ) -> Option<Release> {
        let active = self.active_download.take().expect("completed download");
        // 文件和清单只由持有当前 generation 的任务提交。
        if active.generation == self.generation
            && !active.transfer.cancelled.load(Ordering::Relaxed)
        {
            match result.and_then(|part| {
                cache
                    .commit(active.release.clone(), part)
                    .map_err(|_| Deferred::Storage)
            }) {
                Ok(ready) => {
                    self.ready = Some(ready);
                    self.incomplete_install = false;
                    self.deferred = None;
                }
                Err(reason) => {
                    self.deferred = Some(DownloadDeferral {
                        version: active.release.version,
                        reason,
                        retry_at_ms: now_ms.saturating_add(FAILURE_BACKOFF.as_millis() as i64),
                    });
                }
            }
            if self.save_download_retry(conn).is_err() {
                crate::error!("Could not store update download retry");
            }
        }
        self.queued_download.take()
    }

    fn save_download_retry(&self, conn: &Connection) -> AppResult<()> {
        if let Some(deferred) = &self.deferred {
            let value = serde_json::to_string(deferred)
                .map_err(|error| AppError::Internal(error.to_string()))?;
            conn.execute(
                "INSERT OR REPLACE INTO settings (key, value) VALUES (?, ?)",
                params![DOWNLOAD_RETRY_KEY, value],
            )?;
        } else {
            conn.execute("DELETE FROM settings WHERE key = ?", [DOWNLOAD_RETRY_KEY])?;
        }
        Ok(())
    }

    fn invalidate_ready(&mut self, cache: &UpdateCache) -> bool {
        let Some(ready) = self.ready.as_ref() else {
            return false;
        };
        if cache.unchanged(ready) {
            return false;
        }
        self.remove_ready(cache);
        true
    }

    fn remove_ready(&mut self, cache: &UpdateCache) {
        if let Some(ready) = self.ready.take() {
            if cache.forget(&ready.release.version).is_err() {
                crate::error!("Update cache cleanup failed");
            }
            if self.cache.release.is_none() {
                self.cache.release = Some(ready.release);
                self.cache.etag = None;
            }
        }
        self.incomplete_install = false;
    }
}

pub struct UpdateState {
    current: Version,
    progress: Mutex<UpdateProgress>,
    cache: UpdateCache,
}

impl UpdateState {
    pub(crate) fn load(
        current: Version,
        conn: &Connection,
        cache: UpdateCache,
    ) -> rusqlite::Result<Self> {
        let skipped = SqliteSettingsRepository::get_raw(conn, SKIPPED_KEY)?;
        let version = skipped
            .as_deref()
            .and_then(http::parse_version)
            .filter(|skipped| newer_than(skipped, &current));
        if skipped.is_some() && version.is_none() {
            conn.execute("DELETE FROM settings WHERE key = ?", [SKIPPED_KEY])?;
        }
        let raw_attempt = SqliteSettingsRepository::get_raw(conn, INSTALL_ATTEMPT_KEY)?;
        let attempt = raw_attempt
            .as_deref()
            .and_then(http::parse_version)
            .filter(|version| newer_than(version, &current));
        if raw_attempt.is_some() && attempt.is_none() {
            conn.execute(
                "DELETE FROM settings WHERE key IN (?, ?)",
                [INSTALL_ATTEMPT_KEY, INSTALL_REMINDED_KEY],
            )?;
        }
        let ready = match cache.load(&current, version.as_ref()) {
            Ok(ready) => ready,
            Err(_) => {
                crate::error!("Update cache could not be loaded");
                None
            }
        };
        let mut checked = CachedCheck::load(conn)?;
        if SqliteSettingsRepository::get_raw(conn, ENABLED_KEY)?.as_deref() == Some("false") {
            // A manual download can still be ready while checks are off.
            checked = CachedCheck::default();
        }
        let reminded = SqliteSettingsRepository::get_raw(conn, INSTALL_REMINDED_KEY)?
            .as_deref()
            .and_then(http::parse_version);
        let incomplete_install = ready.as_ref().is_some_and(|ready| {
            attempt.as_ref() == Some(&ready.release.version) && reminded != attempt
        });
        let deferred = SqliteSettingsRepository::get_raw(conn, DOWNLOAD_RETRY_KEY)?
            .and_then(|value| serde_json::from_str::<DownloadDeferral>(&value).ok())
            .filter(|deferred| {
                cache.usable
                    && newer_than(&deferred.version, &current)
                    && ready
                        .as_ref()
                        .is_none_or(|ready| newer_than(&deferred.version, &ready.release.version))
                    && checked.release.as_ref().is_some_and(|release| {
                        release.asset.is_some() && release.version == deferred.version
                    })
            });
        let download_blocked = !cache.usable;
        if download_blocked {
            crate::error!("Update cache is unavailable for this run");
        }
        Ok(Self {
            current,
            cache,
            progress: Mutex::new(UpdateProgress {
                cache: checked,
                onboarding_done: SqliteSettingsRepository::get_raw(conn, ONBOARDING_DONE_KEY)?
                    .as_deref()
                    == Some("true"),
                skipped: version,
                ready,
                incomplete_install,
                download_blocked,
                deferred,
                ..Default::default()
            }),
        })
    }

    pub fn available_version(&self) -> Option<(String, bool)> {
        let progress = self.progress.lock().unwrap();
        progress.available(&self.current).map(|release| {
            let ready = progress
                .ready
                .as_ref()
                .is_some_and(|ready| ready.release.version == release.version);
            (release.version.to_string(), ready)
        })
    }
}

fn publish(app: &AppHandle, snapshot: &UpdateSnapshot) {
    if let Err(error) = app.emit_to("main", STATUS_EVENT, snapshot) {
        crate::error!("Could not publish update status: {error}");
    }
}

fn start_download(app: &AppHandle, progress: &mut UpdateProgress, release: Release, manual: bool) {
    let state = app.state::<UpdateState>();
    let Some(asset) = &release.asset else { return };
    let Some(transfer) = progress.begin_download(release.clone(), manual, wall_clock_ms()) else {
        return;
    };
    let cache = state.cache.clone();
    let app = app.clone();
    let version = release.version.clone();
    let policy = download::Policy::production(asset.size);
    tauri::async_runtime::spawn(async move {
        let shared = transfer.clone();
        let task = tauri::async_runtime::spawn_blocking(move || {
            download::download(
                &cache,
                &release,
                manual,
                &shared,
                &policy,
                download::free_space,
            )
        });
        let mut bytes = 0;
        while !task.inner().is_finished() {
            tokio::time::sleep(Duration::from_millis(250)).await;
            let received = transfer.received.load(Ordering::Relaxed);
            if received != bytes && !transfer.cancelled.load(Ordering::Relaxed) {
                bytes = received;
                let state = app.state::<UpdateState>();
                let progress = state.progress.lock().unwrap();
                publish(&app, &progress.snapshot(&state.current));
            }
        }
        let result = task.await.unwrap_or(Err(Deferred::Failed));
        let state = app.state::<UpdateState>();
        {
            let mut progress = state.progress.lock().unwrap();
            let db = app.state::<DbState>();
            let queued = progress.finish_download(
                &state.cache,
                result,
                &db.conn.lock().unwrap(),
                wall_clock_ms(),
            );
            if let Some(next) = queued {
                start_download(&app, &mut progress, next, true);
            } else if let Some(next) = progress
                .eligible(&state.current)
                .filter(|next| newer_than(&next.version, &version))
                .cloned()
                .filter(|_| automatic_allowed(&app, &progress))
            {
                start_download(&app, &mut progress, next, false);
            }
            publish(&app, &progress.snapshot(&state.current));
        }
        crate::app::tray::refresh_tray(&app);
    });
}

fn automatic_allowed(app: &AppHandle, progress: &UpdateProgress) -> bool {
    !cfg!(debug_assertions)
        && progress.onboarding_done
        && progress
            .wake_until
            .is_none_or(|until| Instant::now() >= until)
        && app
            .state::<SettingsState>()
            .update_check_enabled
            .load(Ordering::Relaxed)
}

fn begin(app: &AppHandle, manual: bool) -> Option<CheckReceiver> {
    let state = app.state::<UpdateState>();
    let mut progress = state.progress.lock().unwrap();
    if !manual && !automatic_allowed(app, &progress) {
        return None;
    }
    let (receiver, request) = progress.begin(manual, wall_clock_ms(), Instant::now())?;
    if let Some(request) = request {
        publish(app, &progress.snapshot(&state.current));
        tauri::async_runtime::spawn(run(app.clone(), request));
    }
    Some(receiver)
}

fn allow_manual_release(
    app: &AppHandle,
    progress: &mut UpdateProgress,
    release: &Release,
) -> AppResult<()> {
    if release.asset.is_some()
        && progress
            .skipped
            .as_ref()
            .is_some_and(|skipped| !newer_than(&release.version, skipped))
    {
        app.state::<DbState>()
            .conn
            .lock()
            .unwrap()
            .execute("DELETE FROM settings WHERE key = ?", [SKIPPED_KEY])?;
        progress.skipped = None;
        progress.dismissed = None;
    }
    Ok(())
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
            let successful = checked.is_ok();
            progress.finish(checked, now, &mut db.conn.lock().unwrap());
            progress.in_flight = None;
            let manual = std::mem::take(&mut progress.manual_check_requested);
            progress.invalidate_ready(&state.cache);
            if manual {
                if successful && progress.failure.is_none() && !progress.download_blocked {
                    if let Some(release) =
                        progress.latest(&state.current).cloned().filter(|release| {
                            allow_manual_release(&app, &mut progress, release).is_ok()
                        })
                    {
                        start_download(&app, &mut progress, release, manual);
                    }
                }
            } else if automatic_allowed(&app, &progress) {
                if let Some(release) = progress.scheduled_download(&state.current, now) {
                    start_download(&app, &mut progress, release, false);
                }
            }
        }
        let snapshot = progress.snapshot(&state.current);
        publish(&app, &snapshot);
        result.send_replace(Some(snapshot));
    }
    crate::app::tray::refresh_tray(&app);
}

fn poll_updates(app: &AppHandle) {
    if begin(app, false).is_some() {
        return;
    }
    let state = app.state::<UpdateState>();
    let mut progress = state.progress.lock().unwrap();
    let invalidated = progress.invalidate_ready(&state.cache);
    let next = automatic_allowed(app, &progress)
        .then(|| progress.scheduled_download(&state.current, wall_clock_ms()))
        .flatten();
    let starting = next.is_some();
    if let Some(release) = next {
        start_download(app, &mut progress, release, false);
    }
    if invalidated || starting {
        publish(app, &progress.snapshot(&state.current));
        drop(progress);
        crate::app::tray::refresh_tray(app);
    }
}

pub fn start_scheduler(app: AppHandle, first_open: bool) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(20)).await;
        if !first_open {
            poll_updates(&app);
        }
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            poll_updates(&app);
        }
    });
}

pub(crate) fn cleanup_update_mounts() {
    install::cleanup_mounts();
}

pub fn set_enabled(app: &AppHandle, enabled: bool) -> AppResult<()> {
    let cleanup = {
        let state = app.state::<UpdateState>();
        let mut progress = state.progress.lock().unwrap();
        let db = app.state::<DbState>();
        let setting = &app.state::<SettingsState>().update_check_enabled;
        if enabled {
            db.settings_repo.set(ENABLED_KEY, "true")?;
            setting.store(true, Ordering::Relaxed);
            Ok(())
        } else {
            let disabled = progress.disable(&mut db.conn.lock().unwrap(), &state.cache);
            if disabled.is_ok() || matches!(disabled, Err(AppError::IO(_))) {
                setting.store(false, Ordering::Relaxed);
            }
            publish(app, &progress.snapshot(&state.current));
            disabled
        }
    };
    if enabled {
        poll_updates(app);
    } else {
        crate::app::tray::refresh_tray(app);
    }
    cleanup?;
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
        poll_updates(&app);
    });
}

pub fn reset_settings(app: &AppHandle) -> AppResult<()> {
    let state = app.state::<UpdateState>();
    let mut progress = state.progress.lock().unwrap();
    let db = app.state::<DbState>();
    let mut conn = db.conn.lock().unwrap();
    let reset = progress.reset(&mut conn, &state.cache);
    publish(app, &progress.snapshot(&state.current));
    reset
}

#[tauri::command]
pub fn get_update_state(app: AppHandle, state: State<'_, UpdateState>) -> UpdateSnapshot {
    let mut progress = state.progress.lock().unwrap();
    let invalidated = progress.invalidate_ready(&state.cache);
    let snapshot = progress.snapshot(&state.current);
    if invalidated {
        publish(&app, &snapshot);
        drop(progress);
        crate::app::tray::refresh_tray(&app);
    }
    snapshot
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
    let (snapshot, cleanup) = {
        let mut progress = state.progress.lock().unwrap();
        if !progress
            .latest(&state.current)
            .is_some_and(|release| release.version == version)
            && !progress
                .ready
                .as_ref()
                .is_some_and(|ready| ready.release.version == version)
        {
            return Err(AppError::Validation("update_failed".into()));
        }
        let db = app.state::<DbState>();
        let mut conn = db.conn.lock().unwrap();
        let cleanup = skip(&mut progress, &mut conn, &state.cache, version);
        let snapshot = progress.snapshot(&state.current);
        publish(&app, &snapshot);
        (snapshot, cleanup)
    };
    crate::app::tray::refresh_tray(&app);
    cleanup?;
    Ok(snapshot)
}

fn skip(
    progress: &mut UpdateProgress,
    conn: &mut Connection,
    cache: &UpdateCache,
    version: Version,
) -> AppResult<()> {
    let tx = conn.transaction()?;
    let skipped = progress
        .skipped
        .as_ref()
        .filter(|previous| newer_than(previous, &version))
        .unwrap_or(&version)
        .clone();
    tx.execute(
        "INSERT OR REPLACE INTO settings (key, value) VALUES (?, ?)",
        params![SKIPPED_KEY, skipped.to_string()],
    )?;
    tx.execute(
        "DELETE FROM settings WHERE key IN (?, ?, ?)",
        [
            INSTALL_ATTEMPT_KEY,
            INSTALL_REMINDED_KEY,
            DOWNLOAD_RETRY_KEY,
        ],
    )?;
    tx.commit()?;
    progress.skipped = Some(skipped);
    progress.clear_downloads(cache)?;
    Ok(())
}

#[tauri::command]
pub fn dismiss_update_banner(app: AppHandle, version: Version) -> UpdateSnapshot {
    let state = app.state::<UpdateState>();
    let mut progress = state.progress.lock().unwrap();
    if progress
        .latest(&state.current)
        .is_some_and(|release| release.version == version)
        || progress
            .ready
            .as_ref()
            .is_some_and(|ready| ready.release.version == version)
    {
        progress.dismissed = Some(version);
    }
    let snapshot = progress.snapshot(&state.current);
    publish(&app, &snapshot);
    snapshot
}

#[tauri::command]
pub fn take_incomplete_update(
    app: AppHandle,
    state: State<'_, UpdateState>,
) -> AppResult<Option<Release>> {
    let mut progress = state.progress.lock().unwrap();
    let invalidated = progress.invalidate_ready(&state.cache);
    let db = app.state::<DbState>();
    let release = progress.take_incomplete_install(&db.conn.lock().unwrap())?;
    if invalidated {
        publish(&app, &progress.snapshot(&state.current));
        drop(progress);
        crate::app::tray::refresh_tray(&app);
    }
    Ok(release)
}

#[tauri::command]
pub fn download_update_now(app: AppHandle, version: Version) -> AppResult<UpdateSnapshot> {
    let state = app.state::<UpdateState>();
    let snapshot = {
        let mut progress = state.progress.lock().unwrap();
        if progress.download_blocked {
            return Err(AppError::Validation("update_failed".into()));
        }
        progress.invalidate_ready(&state.cache);
        let release = progress
            .latest(&state.current)
            .filter(|release| release.version == version && release.asset.is_some())
            .cloned()
            .ok_or_else(|| AppError::Validation("update_failed".into()))?;
        allow_manual_release(&app, &mut progress, &release)?;
        start_download(&app, &mut progress, release, true);
        let snapshot = progress.snapshot(&state.current);
        publish(&app, &snapshot);
        snapshot
    };
    crate::app::tray::refresh_tray(&app);
    Ok(snapshot)
}

async fn prepare_install(
    app: &AppHandle,
    version: &Version,
    installing: bool,
) -> Result<(install::PreparedInstall, u64), InstallError> {
    let state = app.state::<UpdateState>();
    let (release, generation) = {
        let mut progress = state.progress.lock().unwrap();
        let Some(release) = progress
            .ready
            .as_ref()
            .filter(|ready| &ready.release.version == version)
            .map(|ready| ready.release.clone())
        else {
            if installing {
                if let Some(release) = progress
                    .eligible(&state.current)
                    .filter(|release| &release.version == version)
                    .cloned()
                {
                    start_download(app, &mut progress, release, true);
                    publish(app, &progress.snapshot(&state.current));
                }
            }
            return Err(InstallError::Missing);
        };
        if progress.installing {
            return Err(InstallError::Failed);
        }
        if installing {
            progress.installing = true;
            if let Some(active) = &progress.active_download {
                active.transfer.cancel();
            }
        }
        (release, progress.generation)
    };
    let cache = state.cache.clone();
    let candidate = release.clone();
    let result = tauri::async_runtime::spawn_blocking(move || install::prepare(&cache, &candidate))
        .await
        .unwrap_or(Err(InstallError::Failed));
    let result = {
        let mut progress = state.progress.lock().unwrap();
        let current = progress.generation == generation
            && progress
                .ready
                .as_ref()
                .is_some_and(|ready| &ready.release.version == version);
        let result = if current {
            result
        } else {
            Err(InstallError::Missing)
        };
        if result.is_err() && installing {
            progress.installing = false;
        }
        if current && matches!(result, Err(InstallError::Missing | InstallError::Invalid)) {
            progress.remove_ready(&state.cache);
            if installing {
                start_download(app, &mut progress, release, true);
            }
            publish(app, &progress.snapshot(&state.current));
        } else if result.is_err() && installing {
            if let Some(next) = progress.queued_download.take() {
                start_download(app, &mut progress, next, true);
                publish(app, &progress.snapshot(&state.current));
            }
        }
        result
    };
    if result.is_err() {
        crate::app::tray::refresh_tray(app);
    }
    result.map(|prepared| (prepared, generation))
}

#[tauri::command]
pub async fn inspect_update(
    app: AppHandle,
    version: Version,
) -> Result<InstallPreview, InstallError> {
    let (prepared, _) = prepare_install(&app, &version, false).await?;
    tauri::async_runtime::spawn_blocking(move || prepared.preview())
        .await
        .map_err(|_| InstallError::Failed)
}

fn record_install_attempt(conn: &mut Connection, version: &Version) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    tx.execute(
        "INSERT OR REPLACE INTO settings (key, value) VALUES (?, ?)",
        params![INSTALL_ATTEMPT_KEY, version.to_string()],
    )?;
    tx.execute("DELETE FROM settings WHERE key = ?", [INSTALL_REMINDED_KEY])?;
    tx.commit()
}

#[tauri::command]
pub async fn install_update(
    app: AppHandle,
    version: Version,
    confirmed_automatic: Option<bool>,
) -> Result<Option<InstallPreview>, InstallError> {
    let (prepared, generation) = prepare_install(&app, &version, true).await?;
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<UpdateState>();
        let mut progress = state.progress.lock().unwrap();
        let preview = prepared.preview();
        let result = if progress.generation != generation
            || !progress
                .ready
                .as_ref()
                .is_some_and(|ready| ready.release.version == version)
        {
            Err(InstallError::Missing)
        } else if confirmed_automatic != Some(preview.automatic) {
            Ok(Some(preview))
        } else {
            let db = app.state::<DbState>();
            let marked = record_install_attempt(&mut db.conn.lock().unwrap(), &version);
            marked
                .map_err(|_| InstallError::Failed)
                .and_then(|()| prepared.launch())
                .map(|()| None)
        };
        if matches!(result, Ok(None)) {
            crate::app::system::request_app_exit(&app);
        } else {
            progress.installing = false;
            if let Some(next) = progress.queued_download.take() {
                start_download(&app, &mut progress, next, true);
                publish(&app, &progress.snapshot(&state.current));
            }
        }
        result
    })
    .await
    .map_err(|_| InstallError::Failed)?
}

#[cfg(test)]
mod tests {
    use super::*;
    use cache::TestDirectory;
    use sha2::{Digest, Sha256};
    use std::fs;
    use std::io::Write;

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
            asset: None,
        }
    }

    fn downloadable_release(version: &str, bytes: &[u8]) -> Release {
        let mut release = release(version);
        release.asset = Some(ReleaseAsset {
            size: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(bytes)),
            download_url: format!(
                "{}v{version}/{}",
                http::DOWNLOADS,
                http::dmg_name(&release.version)
            ),
        });
        release
    }

    fn stage(cache: &UpdateCache, release: &Release, bytes: &[u8]) -> cache::Part {
        cache.ensure_dir().unwrap();
        let mut part = cache.temporary(&http::dmg_name(&release.version)).unwrap();
        part.file.write_all(bytes).unwrap();
        part.file.sync_all().unwrap();
        part
    }

    fn load(dirs: &TestDirectory, version: &str, conn: &Connection) -> UpdateState {
        UpdateState::load(Version::parse(version).unwrap(), conn, dirs.updates()).unwrap()
    }

    #[test]
    fn cached_release_from_before_asset_support_still_loads() {
        let conn = settings_db();
        conn.execute("INSERT INTO settings VALUES (?, ?)", params![RELEASE_KEY,
            r#"{"version":"1.2.0","releaseUrl":"https://github.com/REXKEV214/dew-clip/releases/tag/v1.2.0","notesPreview":"Release notes"}"#,
        ]).unwrap();
        let cached = CachedCheck::load(&conn).unwrap().release.unwrap();
        assert_eq!(cached.version, Version::new(1, 2, 0));
        assert!(cached.asset.is_none());
    }

    #[test]
    fn verified_download_reappears_after_later_and_an_incomplete_install_prompts_once() {
        let dirs = TestDirectory::new();
        let cache = dirs.updates();
        let mut conn = settings_db();
        let state = load(&dirs, "1.1.0", &conn);
        let bytes = b"downloaded image";
        let release = downloadable_release("1.2.0", bytes);
        let mut progress = state.progress.lock().unwrap();
        progress.cache.release = Some(release.clone());
        assert!(progress.begin_download(release.clone(), false, 0).is_some());
        assert!(!progress.snapshot(&state.current).available);
        progress.finish_download(&cache, Ok(stage(&cache, &release, bytes)), &conn, 0);
        assert!(progress.snapshot(&state.current).available);
        assert!(progress.ready.is_some());
        progress.dismissed = Some(release.version.clone());
        assert!(progress.snapshot(&state.current).banner_dismissed);
        drop(progress);
        let state = load(&dirs, "1.1.0", &conn);
        let snapshot = state.progress.lock().unwrap().snapshot(&state.current);
        assert!(snapshot.ready.is_some());
        assert!(!snapshot.banner_dismissed);
        assert!(!snapshot.incomplete_install);

        record_install_attempt(&mut conn, &release.version).unwrap();
        let state = load(&dirs, "1.1.0", &conn);
        let mut progress = state.progress.lock().unwrap();
        assert!(progress.snapshot(&state.current).incomplete_install);
        assert_eq!(
            progress
                .take_incomplete_install(&conn)
                .unwrap()
                .unwrap()
                .version,
            release.version
        );
        assert!(progress.take_incomplete_install(&conn).unwrap().is_none());
        drop(progress);
        let state = load(&dirs, "1.1.0", &conn);
        let mut progress = state.progress.lock().unwrap();
        let snapshot = progress.snapshot(&state.current);
        assert!(snapshot.available);
        assert!(snapshot.ready.is_some());
        assert!(!snapshot.banner_dismissed);
        assert!(!snapshot.incomplete_install);
        assert!(progress.take_incomplete_install(&conn).unwrap().is_none());
        drop(progress);
        record_install_attempt(&mut conn, &release.version).unwrap();
        let state = load(&dirs, "1.1.0", &conn);
        assert!(state
            .progress
            .lock()
            .unwrap()
            .take_incomplete_install(&conn)
            .unwrap()
            .is_some());

        fs::write(cache.root.join(".DewClip_1.3.0.dmg.abandoned.part"), [0]).unwrap();
        fs::write(cache.root.join("DewClip_1.3.0.dmg"), [0]).unwrap();
        let state = load(&dirs, "1.2.0", &conn);
        assert!(state.progress.lock().unwrap().ready.is_none());
        assert_eq!(fs::read_dir(&cache.root).unwrap().count(), 0);
        for key in [INSTALL_ATTEMPT_KEY, INSTALL_REMINDED_KEY] {
            assert!(SqliteSettingsRepository::get_raw(&conn, key)
                .unwrap()
                .is_none());
        }
    }

    #[test]
    fn a_missing_changed_or_symlinked_dmg_is_no_longer_ready() {
        let dirs = TestDirectory::new();
        let cache = dirs.updates();
        let conn = settings_db();
        let release = downloadable_release("1.2.0", b"image");
        let path = cache.path(&release.version);
        let save = || {
            cache
                .commit(release.clone(), stage(&cache, &release, b"image"))
                .unwrap()
        };

        save();
        fs::remove_file(&path).unwrap();
        let state = load(&dirs, "1.1.0", &conn);
        assert!(state.progress.lock().unwrap().ready.is_none());
        save();
        let state = load(&dirs, "1.1.0", &conn);
        fs::write(&path, b"other").unwrap();
        #[cfg(target_os = "macos")]
        assert!(matches!(
            cache.copy_verified_snapshot(&release, &dirs.0.join("snapshot.dmg")),
            Err(error) if error.kind() == std::io::ErrorKind::InvalidData
        ));
        #[cfg(not(target_os = "macos"))]
        assert!(matches!(
            install::prepare(&cache, &release),
            Err(InstallError::Invalid)
        ));
        let mut progress = state.progress.lock().unwrap();
        assert!(progress.invalidate_ready(&cache));
        assert!(progress.ready.is_none());
        assert!(!path.exists());

        #[cfg(unix)]
        {
            save();
            fs::remove_file(&path).unwrap();
            let outside = dirs.0.join("outside");
            fs::write(&outside, b"image").unwrap();
            std::os::unix::fs::symlink(&outside, &path).unwrap();
            let state = load(&dirs, "1.1.0", &conn);
            assert!(state.progress.lock().unwrap().ready.is_none());
            assert_eq!(fs::read(&outside).unwrap(), b"image");
            assert!(!path.exists());
        }
    }

    #[test]
    fn a_failed_newer_download_preserves_the_ready_dmg_until_a_verified_replacement() {
        let dirs = TestDirectory::new();
        let cache = dirs.updates();
        let conn = settings_db();
        let older = downloadable_release("1.2.0", b"old image");
        let newer = downloadable_release("1.3.0", b"new image");
        cache
            .commit(older.clone(), stage(&cache, &older, b"old image"))
            .unwrap();
        let current = load(&dirs, "1.1.0", &conn);
        let mut progress = current.progress.lock().unwrap();
        progress.cache.release = Some(newer.clone());
        assert!(progress.begin_download(newer.clone(), false, 0).is_some());
        assert!(progress.begin_download(newer.clone(), false, 0).is_none());
        assert_eq!(
            progress.available(&current.current).unwrap().version,
            older.version
        );
        progress.finish_download(&cache, Err(Deferred::Failed), &conn, 0);
        assert_eq!(
            progress.available(&current.current).unwrap().version,
            newer.version
        );
        assert_eq!(
            progress.ready.as_ref().unwrap().release.version,
            older.version
        );
        assert!(cache.path(&older.version).exists());
        assert!(!cache.path(&newer.version).exists());
        assert!(progress.begin_download(newer.clone(), false, 0).is_none());
        assert!(progress.begin_download(newer.clone(), true, 0).is_some());
        progress.finish_download(&cache, Ok(stage(&cache, &newer, b"new image")), &conn, 0);
        assert!(!cache.path(&older.version).exists());
        assert_eq!(
            progress.ready.as_ref().unwrap().release.version,
            newer.version
        );
        assert_eq!(fs::read(cache.path(&newer.version)).unwrap(), b"new image");
    }

    #[test]
    fn skip_disable_and_reset_cancel_transfers_clear_cache_and_reject_stale_results() {
        for action in [
            skip as fn(&mut UpdateProgress, &mut Connection, &UpdateCache, Version) -> _,
            |progress, conn, cache, _| progress.disable(conn, cache),
            |progress, conn, cache, _| progress.reset(conn, cache),
        ] {
            let dirs = TestDirectory::new();
            let cache = dirs.updates();
            let mut conn = settings_db();
            let older = downloadable_release("1.2.0", b"old image");
            let newer = downloadable_release("1.3.0", b"new image");
            cache
                .commit(older.clone(), stage(&cache, &older, b"old image"))
                .unwrap();
            let state = load(&dirs, "1.1.0", &conn);
            let mut progress = state.progress.lock().unwrap();
            let transfer = progress.begin_download(newer.clone(), false, 0).unwrap();
            progress.deferred = Some(DownloadDeferral {
                version: newer.version.clone(),
                reason: Deferred::Slow,
                retry_at_ms: 0,
            });
            progress.save_download_retry(&conn).unwrap();
            let stale = stage(&cache, &newer, b"new image");
            let clipboard = cache.root.join("clipboard.db");
            Connection::open(&clipboard)
                .unwrap()
                .execute_batch("CREATE TABLE clipboard (content TEXT);")
                .unwrap();
            let attachment = cache.root.join("attachments/photo.png");
            fs::create_dir(attachment.parent().unwrap()).unwrap();
            fs::write(&attachment, b"photo").unwrap();
            let unrelated_dmg = cache.root.join("export.dmg");
            fs::write(&unrelated_dmg, b"export").unwrap();
            let unrelated_part = cache.root.join("document.part");
            fs::write(&unrelated_part, b"document").unwrap();
            let directory_named_like_dmg = cache.root.join("DewClip_1.4.0.dmg");
            fs::create_dir(&directory_named_like_dmg).unwrap();
            fs::write(directory_named_like_dmg.join("keep"), b"keep").unwrap();
            let generation = progress.generation;
            action(&mut progress, &mut conn, &cache, older.version).unwrap();
            assert_ne!(generation, progress.generation);
            assert!(transfer.cancelled.load(Ordering::Relaxed));
            assert!(progress.ready.is_none());
            assert_eq!(fs::read_dir(&cache.root).unwrap().count(), 5);
            progress.finish_download(&cache, Ok(stale), &conn, 0);
            assert!(progress.ready.is_none());
            assert_eq!(fs::read_dir(&cache.root).unwrap().count(), 5);
            assert_eq!(fs::read(&attachment).unwrap(), b"photo");
            assert_eq!(
                fs::read(directory_named_like_dmg.join("keep")).unwrap(),
                b"keep"
            );
            assert_eq!(fs::read(unrelated_dmg).unwrap(), b"export");
            assert_eq!(fs::read(unrelated_part).unwrap(), b"document");
            assert_eq!(
                Connection::open(clipboard)
                    .unwrap()
                    .query_row("SELECT count(*) FROM clipboard", [], |row| row
                        .get::<_, i64>(0))
                    .unwrap(),
                0
            );
            assert!(SqliteSettingsRepository::get_raw(&conn, DOWNLOAD_RETRY_KEY)
                .unwrap()
                .is_none());
        }
    }

    #[test]
    fn cache_overlap_blocks_download_and_cleanup_at_startup_and_after_migration() {
        let dirs = TestDirectory::new();
        let source = dirs.0.join("data");
        let mut cache = dirs.updates();
        fs::create_dir(&source).unwrap();
        fs::create_dir(&cache.root).unwrap();
        let source_db = Connection::open(source.join("clipboard.db")).unwrap();
        crate::infrastructure::repository::migrations::run_migrations(&source_db).unwrap();
        fs::create_dir(source.join("attachments")).unwrap();
        fs::write(source.join("attachments/photo.png"), b"photo").unwrap();
        let release = downloadable_release("1.2.0", b"download");
        CachedCheck {
            checked_at: Some(1),
            release: Some(release.clone()),
            etag: None,
        }
        .save(&mut Connection::open(source.join("clipboard.db")).unwrap())
        .unwrap();
        drop(source_db);
        cache.exclude_data_dir(&source);
        assert!(cache.usable);
        crate::app::data_directory::request_move(&source, &source, &cache.root).unwrap();
        let moved = crate::app::data_directory::apply_requested_move(&source, &source)
            .unwrap()
            .unwrap();
        cache.exclude_data_dir(&moved);
        assert!(!cache.usable);

        let mut fresh_cache = dirs.updates();
        fresh_cache.exclude_data_dir(&moved);
        assert!(!fresh_cache.usable);
        let mut containing_cache = dirs.updates();
        containing_cache.exclude_data_dir(&moved.join("attachments"));
        assert!(!containing_cache.usable);
        let mut nested_cache = UpdateCache::new(moved.join("unused-directory/updates"));
        nested_cache.exclude_data_dir(&moved);
        assert!(!nested_cache.usable);

        let conn = Connection::open(moved.join("clipboard.db")).unwrap();
        for cache in [cache, fresh_cache] {
            // 即使数据目录里有同名文件，也不能作为更新文件处理。
            fs::write(cache.path(&release.version), b"user file").unwrap();
            let state = UpdateState::load(Version::new(1, 1, 0), &conn, cache.clone()).unwrap();
            let mut progress = state.progress.lock().unwrap();
            assert!(progress.scheduled_download(&state.current, 1).is_none());
            assert!(progress.begin_download(release.clone(), true, 1).is_none());
            assert!(progress
                .snapshot(&state.current)
                .latest
                .unwrap()
                .asset
                .is_none());
            progress.clear_downloads(&cache).unwrap();
            assert_eq!(
                fs::read(cache.path(&release.version)).unwrap(),
                b"user file"
            );
            assert_eq!(
                fs::read(moved.join("attachments/photo.png")).unwrap(),
                b"photo"
            );
        }
    }

    #[test]
    fn interrupted_download_and_persisted_backoff_resume_independently_of_daily_checks() {
        let dirs = TestDirectory::new();
        let cache = dirs.updates();
        let mut conn = settings_db();
        let release = downloadable_release("1.2.0", b"image");
        let checked_at = 100_000_000;
        CachedCheck {
            checked_at: Some(checked_at),
            release: Some(release.clone()),
            etag: None,
        }
        .save(&mut conn)
        .unwrap();
        cache.ensure_dir().unwrap();
        fs::write(
            cache.root.join(".DewClip_1.2.0.dmg.abandoned.part"),
            b"partial",
        )
        .unwrap();
        let state = load(&dirs, "1.1.0", &conn);
        let mut progress = state.progress.lock().unwrap();
        assert_eq!(fs::read_dir(&cache.root).unwrap().count(), 0);
        assert!(!progress.cache.due(checked_at + 1));
        assert_eq!(
            progress
                .scheduled_download(&state.current, checked_at + 1)
                .unwrap()
                .version,
            release.version
        );
        assert!(progress
            .begin_download(release.clone(), false, checked_at + 1)
            .is_some());
        let failed_at = checked_at + 20;
        progress.finish_download(&cache, Err(Deferred::Slow), &conn, failed_at);
        drop(progress);

        let state = load(&dirs, "1.1.0", &conn);
        let mut progress = state.progress.lock().unwrap();
        let retry_at = failed_at + FAILURE_BACKOFF.as_millis() as i64;
        assert!(matches!(
            progress.download_status(),
            Some(DownloadStatus::Deferred {
                reason: Deferred::Slow,
                ..
            })
        ));
        assert!(progress
            .scheduled_download(&state.current, retry_at - 1)
            .is_none());
        assert_eq!(
            progress
                .scheduled_download(&state.current, retry_at)
                .unwrap()
                .version,
            release.version
        );
        assert!(progress
            .begin_download(release.clone(), true, retry_at - 1)
            .is_some());
        progress.finish_download(
            &cache,
            Ok(stage(&cache, &release, b"image")),
            &conn,
            retry_at,
        );
        assert!(SqliteSettingsRepository::get_raw(&conn, DOWNLOAD_RETRY_KEY)
            .unwrap()
            .is_none());
        let state = load(&dirs, "1.1.0", &conn);
        assert!(state
            .progress
            .lock()
            .unwrap()
            .scheduled_download(&state.current, retry_at)
            .is_none());
    }

    #[test]
    fn low_space_defers_without_opening_a_part_file() {
        let dirs = TestDirectory::new();
        let cache = dirs.updates();
        let bytes = b"image";
        let release = downloadable_release("1.2.0", bytes);
        let policy = download::Policy::production(bytes.len() as u64);
        let result = download::download(
            &cache,
            &release,
            false,
            &Transfer::default(),
            &policy,
            |_| Ok(200 * 1024 * 1024 - 1),
        );
        assert!(matches!(result, Err(Deferred::Space)));
        assert_eq!(fs::read_dir(&cache.root).unwrap().count(), 0);
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
        let dirs = TestDirectory::new();
        let conn = std::sync::Arc::new(Mutex::new(settings_db()));
        let settings = SqliteSettingsRepository::new(conn.clone());
        let current = Version::new(1, 0, 0);
        let state =
            UpdateState::load(current.clone(), &conn.lock().unwrap(), dirs.updates()).unwrap();
        let mut progress = state.progress.into_inner().unwrap();
        let now = Instant::now();
        assert!(progress.begin(false, 1, now).is_none());

        progress.save_onboarding_done(&settings, "true").unwrap();
        let state = UpdateState::load(current, &conn.lock().unwrap(), dirs.updates()).unwrap();
        assert!(state.progress.lock().unwrap().onboarding_done);
        let mut conn = conn.lock().unwrap();
        assert!(progress.begin(false, 1, now).is_some());
        progress.reset(&mut conn, &dirs.updates()).unwrap();
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
        assert!(progress.manual_check_requested);
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
        let dirs = TestDirectory::new();
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
        let _ = UpdateState::load(Version::new(1, 2, 0), &conn, dirs.updates()).unwrap();
        assert!(SqliteSettingsRepository::get_raw(&conn, SKIPPED_KEY)
            .unwrap()
            .is_none());
    }

    #[test]
    fn reset_restores_defaults_and_invalidates_an_in_flight_result() {
        let dirs = TestDirectory::new();
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
        progress.reset(&mut conn, &dirs.updates()).unwrap();
        assert_ne!(request.unwrap().generation, progress.generation);
        assert!(progress.in_flight.is_none());
        assert!(progress.cache.checked_at.is_none());
        for key in [
            CHECKED_KEY,
            RELEASE_KEY,
            ETAG_KEY,
            SKIPPED_KEY,
            DOWNLOAD_RETRY_KEY,
            INSTALL_ATTEMPT_KEY,
            INSTALL_REMINDED_KEY,
        ] {
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
