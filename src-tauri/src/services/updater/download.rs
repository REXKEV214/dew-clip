use super::{
    cache::{Part, UpdateCache},
    http, Release,
};
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering::Relaxed},
    Mutex,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Deferred {
    Slow,
    Restricted,
    Space,
    Storage,
    Failed,
}

#[derive(Default)]
pub(super) struct Transfer {
    pub cancelled: AtomicBool,
    pub received: AtomicU64,
    preparing: Mutex<()>,
}

impl Transfer {
    pub fn cancel(&self) {
        let _preparing = self.preparing.lock().unwrap();
        self.cancelled.store(true, Relaxed);
    }
}

#[derive(Serialize)]
pub(super) struct Policy<'a> {
    pub base: &'a str,
    pub hosts: &'a [&'a str],
    pub size: u64,
    pub idle: f64,
    pub grace: f64,
    pub sample: f64,
    pub window: f64,
    #[serde(rename = "minimumRate")]
    pub minimum_rate: f64,
    pub total: f64,
}

impl Policy<'_> {
    pub fn production(size: u64) -> Self {
        Self {
            base: http::DOWNLOADS,
            hosts: &[
                "github.com",
                "release-assets.githubusercontent.com",
                "objects.githubusercontent.com",
                "github-releases.githubusercontent.com",
            ],
            size,
            idle: 15.0,
            grace: 5.0,
            sample: 2.0,
            window: 10.0,
            minimum_rate: 100.0 * 1024.0,
            total: (size as f64 / (64.0 * 1024.0)).clamp(120.0, 300.0),
        }
    }
}

#[cfg(target_os = "macos")]
pub(super) fn free_space(path: &std::path::Path) -> std::io::Result<u64> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes())?;
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    if unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let stats = unsafe { stats.assume_init() };
    Ok(u64::from(stats.f_bavail).saturating_mul(stats.f_frsize))
}

#[cfg(not(target_os = "macos"))]
pub(super) fn free_space(_path: &std::path::Path) -> std::io::Result<u64> {
    Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
}

pub(super) fn download(
    cache: &UpdateCache,
    release: &Release,
    manual: bool,
    transfer: &Transfer,
    policy: &Policy<'_>,
    available_space: impl FnOnce(&std::path::Path) -> std::io::Result<u64>,
) -> Result<Part, Deferred> {
    let asset = release.asset.as_ref().ok_or(Deferred::Failed)?;
    if !http::valid_asset(asset, policy.base) || asset.size != policy.size {
        return Err(Deferred::Failed);
    }
    let part = {
        let _preparing = transfer.preparing.lock().unwrap();
        if transfer.cancelled.load(Relaxed) {
            return Err(Deferred::Failed);
        }
        cache.ensure_dir().map_err(|_| Deferred::Storage)?;
        let required = (asset.size * 3 + 100 * 1024 * 1024).max(200 * 1024 * 1024);
        if available_space(&cache.root).map_err(|_| Deferred::Storage)? < required {
            return Err(Deferred::Space);
        }
        cache
            .temporary(&http::dmg_name(&release.version))
            .map_err(|_| Deferred::Storage)?
    };
    receive(
        &asset.download_url,
        &asset.sha256,
        &part,
        manual,
        transfer,
        policy,
    )?;
    part.file.sync_all().map_err(|_| Deferred::Storage)?;
    Ok(part)
}

#[cfg(target_os = "macos")]
fn receive(
    url: &str,
    sha256: &str,
    part: &Part,
    manual: bool,
    transfer: &Transfer,
    policy: &Policy<'_>,
) -> Result<(), Deferred> {
    use sha2::{Digest, Sha256};
    use std::ffi::{c_char, c_void, CString};
    use std::os::fd::AsRawFd;
    use std::sync::atomic::Ordering::Relaxed;

    type Progress = unsafe extern "C" fn(*mut c_void, *const u8, usize) -> bool;
    extern "C" {
        fn dewclip_download_update(
            url: *const c_char,
            policy: *const c_char,
            fd: i32,
            manual: bool,
            context: *mut c_void,
            progress: Progress,
        ) -> i32;
    }
    struct Context<'a> {
        transfer: &'a Transfer,
        digest: Sha256,
    }
    unsafe extern "C" fn progress(context: *mut c_void, bytes: *const u8, len: usize) -> bool {
        let context = &mut *context.cast::<Context<'_>>();
        if context.transfer.cancelled.load(Relaxed) {
            return false;
        }
        if len != 0 {
            context
                .digest
                .update(std::slice::from_raw_parts(bytes, len));
            context.transfer.received.fetch_add(len as u64, Relaxed);
        }
        true
    }

    let url = CString::new(url).map_err(|_| Deferred::Failed)?;
    let options = CString::new(serde_json::to_vec(policy).map_err(|_| Deferred::Failed)?)
        .map_err(|_| Deferred::Failed)?;
    let mut context = Context {
        transfer,
        digest: Sha256::new(),
    };
    // Swift calls progress serially and finishes all callbacks before returning.
    let status = unsafe {
        dewclip_download_update(
            url.as_ptr(),
            options.as_ptr(),
            part.file.as_raw_fd(),
            manual,
            (&mut context as *mut Context<'_>).cast(),
            progress,
        )
    };
    let received = transfer.received.load(Relaxed);
    crate::info!("Update download ended (status {status}, bytes {received})");
    match status {
        200 if received == policy.size && format!("{:x}", context.digest.finalize()) == sha256 => {
            Ok(())
        }
        -2 => Err(Deferred::Slow),
        -3 => Err(Deferred::Restricted),
        -4 => Err(Deferred::Storage),
        _ => Err(Deferred::Failed),
    }
}

#[cfg(not(target_os = "macos"))]
fn receive(
    _url: &str,
    _sha256: &str,
    _part: &Part,
    _manual: bool,
    _transfer: &Transfer,
    _policy: &Policy<'_>,
) -> Result<(), Deferred> {
    Err(Deferred::Failed)
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use crate::services::updater::{cache::TestDirectory, ReleaseAsset};
    use semver::Version;
    use sha2::{Digest, Sha256};
    use std::io::{self, BufRead, BufReader, Write};
    use std::net::{TcpListener, TcpStream};
    use std::os::unix::fs::PermissionsExt;
    use std::thread;
    use std::time::{Duration, Instant};

    const BODY: &[u8] = &[0xa5; 128 * 1024];

    struct Server {
        listener: TcpListener,
        requests: Vec<String>,
    }

    impl Server {
        fn address(&self) -> std::net::SocketAddr {
            self.listener.local_addr().unwrap()
        }

        fn respond(&mut self, reply: impl FnOnce(&mut TcpStream)) {
            let deadline = Instant::now() + Duration::from_secs(3);
            let mut socket = loop {
                match self.listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(error)
                        if error.kind() == io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => panic!("accept local download request: {error}"),
                }
            };
            socket.set_nonblocking(false).unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = String::new();
            let mut reader = BufReader::new(&mut socket);
            loop {
                let mut line = String::new();
                assert_ne!(reader.read_line(&mut line).unwrap(), 0);
                request.push_str(&line);
                if line == "\r\n" {
                    break;
                }
            }
            self.requests.push(request.to_ascii_lowercase());
            reply(&mut socket);
        }

        fn redirect(&mut self, location: &str) {
            self.respond(|socket| {
                write!(socket, "HTTP/1.1 302 Found\r\nLocation: {location}\r\nSet-Cookie: update=value; Path=/\r\nConnection: close\r\nContent-Length: 0\r\n\r\n").unwrap();
            });
        }
    }

    fn exchange(
        digest: &[u8],
        hosts: &[&str],
        configure: impl FnOnce(&mut Policy<'_>),
        respond: impl FnOnce(&mut Server) + Send + 'static,
    ) -> (Result<(), Deferred>, Vec<String>, u64, Duration) {
        let listener =
            TcpListener::bind("127.0.0.1:0").expect("bind local download test HTTP server");
        listener.set_nonblocking(true).unwrap();
        let mut server = Server {
            listener,
            requests: Vec::new(),
        };
        let base = format!("http://{}/download/", server.address());
        let worker = thread::spawn(move || {
            respond(&mut server);
            server.requests
        });
        let dirs = TestDirectory::new();
        let cache = dirs.updates();
        let release = Release {
            version: Version::new(1, 2, 0),
            notes_preview: String::new(),
            release_url: format!("{base}latest"),
            asset: Some(ReleaseAsset {
                size: BODY.len() as u64,
                download_url: format!("{base}start"),
                sha256: format!("{:x}", Sha256::digest(digest)),
            }),
        };
        let mut policy = Policy {
            base: &base,
            hosts,
            idle: 1.0,
            total: 2.0,
            ..Policy::production(BODY.len() as u64)
        };
        configure(&mut policy);
        let transfer = Transfer::default();
        let started = Instant::now();
        let result = download(&cache, &release, true, &transfer, &policy, |_| Ok(u64::MAX));
        let elapsed = started.elapsed();
        let result = result.map(|part| {
            assert_eq!(
                part.file.metadata().unwrap().permissions().mode() & 0o777,
                0o600
            );
            cache.commit(release.clone(), part).unwrap();
            assert_eq!(std::fs::read(cache.path(&release.version)).unwrap(), BODY);
        });
        let remaining = std::fs::read_dir(&cache.root).unwrap().count();
        assert_eq!(remaining, if result.is_ok() { 2 } else { 0 });
        (
            result,
            worker.join().unwrap(),
            transfer.received.load(Relaxed),
            elapsed,
        )
    }

    fn response_headers(socket: &mut TcpStream, length: Option<usize>) {
        write!(
            socket,
            "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: application/octet-stream\r\n"
        )
        .unwrap();
        if let Some(length) = length {
            write!(socket, "Content-Length: {length}\r\n").unwrap();
        }
        socket.write_all(b"\r\n").unwrap();
    }

    fn response(socket: &mut TcpStream, body: &[u8], length: Option<usize>) {
        response_headers(socket, length);
        let _ = socket.write_all(body);
    }

    #[test]
    fn native_download_streams_to_a_verified_file() {
        let (result, requests, ..) = exchange(
            BODY,
            &["127.0.0.1"],
            |_| {},
            |server| {
                server.respond(|socket| response(socket, BODY, Some(BODY.len())));
            },
        );
        assert!(result.is_ok());
        assert!(requests[0].starts_with("get /download/start http/1.1\r\n"));
    }

    #[test]
    fn native_download_follows_an_allowed_redirect_with_only_the_expected_headers() {
        let (result, requests, ..) = exchange(
            BODY,
            &["127.0.0.1", "localhost"],
            |_| {},
            |server| {
                server.redirect(&format!(
                    "http://localhost:{}/asset?signed=1",
                    server.address().port()
                ));
                server.respond(|socket| response(socket, BODY, Some(BODY.len())));
            },
        );
        assert!(result.is_ok());
        assert!(requests[1].starts_with("get /asset?signed=1 http/1.1\r\n"));
        for request in requests {
            assert!(request.lines().any(|line| line == "user-agent: dewclip"));
            for header in request.lines().skip(1).filter(|line| !line.is_empty()) {
                let (field, value) = header.split_once(':').unwrap();
                match field {
                    "user-agent" | "host" | "connection" | "accept-encoding" => {}
                    "accept" => assert_eq!(value.trim(), "*/*"),
                    "accept-language" => assert!(value.trim().is_empty()),
                    _ => panic!("unexpected HTTP header: {field}"),
                }
            }
        }
    }

    #[test]
    fn native_download_rejects_disallowed_destinations_and_a_fourth_redirect() {
        let hosts = &["127.0.0.1"];
        let (result, ..) = exchange(
            BODY,
            hosts,
            |_| {},
            |server| {
                server.redirect(&format!(
                    "http://localhost:{}/asset",
                    server.address().port()
                ));
            },
        );
        assert_eq!(result, Err(Deferred::Failed));
        let (result, ..) = exchange(
            BODY,
            hosts,
            |_| {},
            |server| {
                server.redirect(&format!("http://user@{}/download/asset", server.address()));
            },
        );
        assert_eq!(result, Err(Deferred::Failed));
        let (result, ..) = exchange(
            BODY,
            hosts,
            |_| {},
            |server| {
                server.redirect(&format!(
                    "http://{}/different-repository/asset",
                    server.address()
                ));
            },
        );
        assert_eq!(result, Err(Deferred::Failed));
        let (result, requests, ..) = exchange(
            BODY,
            hosts,
            |_| {},
            |server| {
                for redirect in 1..=4 {
                    server.redirect(&format!(
                        "http://{}/download/asset-{redirect}",
                        server.address()
                    ));
                }
            },
        );
        assert_eq!(result, Err(Deferred::Failed));
        assert_eq!(requests.len(), 4);
    }

    #[test]
    fn native_download_drops_partial_and_final_files_on_hash_size_or_stream_errors() {
        let hosts = &["127.0.0.1"];
        let (result, ..) = exchange(
            b"another image",
            hosts,
            |_| {},
            |server| {
                server.respond(|socket| response(socket, BODY, Some(BODY.len())));
            },
        );
        assert_eq!(result, Err(Deferred::Failed));
        for (body, length) in [
            (BODY, Some(BODY.len() + 1)),
            (&[0xa5; BODY.len() + 1], None),
            (&BODY[..BODY.len() / 2], Some(BODY.len())),
        ] {
            let (result, ..) = exchange(
                BODY,
                hosts,
                |_| {},
                move |server| {
                    server.respond(|socket| response(socket, body, length));
                },
            );
            assert_eq!(result, Err(Deferred::Failed));
        }
    }

    #[test]
    fn native_download_cancels_when_headers_do_not_arrive() {
        let (result, ..) = exchange(
            BODY,
            &["127.0.0.1"],
            |policy| policy.idle = 0.35,
            |server| {
                server.respond(|_| thread::sleep(Duration::from_millis(750)));
            },
        );
        assert_eq!(result, Err(Deferred::Slow));
    }

    #[test]
    fn native_download_cancels_a_slow_trickle_after_data_is_delivered() {
        let (result, _, received, elapsed) = exchange(
            BODY,
            &["127.0.0.1"],
            |policy| {
                policy.idle = 2.0;
                policy.total = 8.0;
                policy.grace = 0.2;
                policy.sample = 0.15;
                policy.window = 0.3;
                policy.minimum_rate = 128.0 * 1024.0;
            },
            |server| {
                server.respond(|socket| {
                    socket.set_nodelay(true).unwrap();
                    response_headers(socket, Some(BODY.len()));
                    let (initial, remaining) = BODY.split_at(64 * 1024);
                    socket.write_all(initial).unwrap();
                    for chunk in remaining.chunks(1024) {
                        thread::sleep(Duration::from_millis(80));
                        if socket.write_all(chunk).is_err() {
                            break;
                        }
                    }
                });
            },
        );
        assert_eq!(result, Err(Deferred::Slow));
        assert!(received > 0 && received < BODY.len() as u64);
        assert!(elapsed < Duration::from_secs(2), "elapsed {elapsed:?}");
    }

    #[test]
    fn native_download_cancels_when_headers_are_followed_by_silence() {
        let (result, _, received, _) = exchange(
            BODY,
            &["127.0.0.1"],
            |policy| {
                policy.idle = 0.35;
                policy.grace = 3.0;
            },
            |server| {
                server.respond(|socket| {
                    response_headers(socket, Some(BODY.len()));
                    thread::sleep(Duration::from_millis(800));
                    let _ = socket.write_all(BODY);
                });
            },
        );
        assert_eq!(result, Err(Deferred::Slow));
        assert_eq!(received, 0);
    }

    #[test]
    fn native_download_accepts_delayed_headers_followed_by_a_fast_body() {
        let (result, _, received, _) = exchange(
            BODY,
            &["127.0.0.1"],
            |policy| {
                policy.idle = 2.0;
                policy.total = 4.0;
                policy.grace = 0.15;
                policy.sample = 0.15;
            },
            |server| {
                server.respond(|socket| {
                    thread::sleep(Duration::from_millis(650));
                    response(socket, BODY, Some(BODY.len()));
                });
            },
        );
        assert_eq!(result, Ok(()));
        assert_eq!(received, BODY.len() as u64);
    }
}
