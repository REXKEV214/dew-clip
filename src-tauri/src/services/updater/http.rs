use super::{CheckError, Release};
use semver::Version;
use serde::Deserialize;
use std::cmp::Ordering;
use std::time::Duration;
use tauri::Url;

pub(super) const ENDPOINT: &str = "https://api.github.com/repos/REXKEV214/dew-clip/releases/latest";
pub(super) const RELEASES: &str = "https://github.com/REXKEV214/dew-clip/releases/";
const MAX_RETRY_AFTER_SECS: u64 = 24 * 60 * 60;

#[derive(Default)]
pub(super) struct Response {
    status: i32,
    body: Vec<u8>,
    etag: String,
    rate_limit_reset: String,
    retry_after: String,
}

pub(super) enum CheckResult {
    Fresh {
        release: Option<Release>,
        etag: Option<String>,
    },
    Unchanged,
}

#[derive(Debug)]
pub(super) struct Failure {
    pub(super) kind: CheckError,
    pub(super) retry_after: Option<Duration>,
    pub(super) detail: &'static str,
}

impl Failure {
    pub(super) fn new(kind: CheckError, detail: &'static str) -> Self {
        Self {
            kind,
            retry_after: None,
            detail,
        }
    }
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    html_url: String,
    draft: bool,
    prerelease: bool,
    body: Option<String>,
}

pub(super) fn parse_version(tag: &str) -> Option<Version> {
    let tag = tag.trim();
    let version = tag.strip_prefix(['v', 'V']).unwrap_or(tag);
    if version.len() > 128 {
        return None;
    }
    Version::parse(version).ok()
}

pub(super) fn release_url(raw: &str, base: &str) -> String {
    if raw.len() <= 512
        && raw.starts_with(base)
        && !raw.chars().any(|ch| ch.is_control() || ch == '\\')
    {
        if let (Ok(url), Ok(base)) = (Url::parse(raw), Url::parse(base)) {
            if url.as_str().len() <= 512
                && url.origin() == base.origin()
                && url.username().is_empty()
                && url.password().is_none()
                && url.path().starts_with(base.path())
            {
                return url.into();
            }
        }
    }
    format!("{base}latest")
}

fn release_from_body(body: &[u8], base: &str) -> Result<Option<Release>, Failure> {
    let remote: GithubRelease = serde_json::from_slice(body)
        .map_err(|_| Failure::new(CheckError::Failed, "invalid release JSON"))?;
    let Some(version) = parse_version(&remote.tag_name) else {
        return Ok(None);
    };
    if remote.draft || remote.prerelease || !version.pre.is_empty() {
        return Ok(None);
    }
    let notes = remote.body.unwrap_or_default();
    let mut notes_preview = String::new();
    // Leave room for JSON escaping and the release URL in the 2 KB cache.
    for line in notes
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(5)
    {
        if !notes_preview.is_empty() {
            notes_preview.push('\n');
        }
        for ch in line.chars().filter(|ch| !ch.is_control()) {
            if notes_preview.len() + ch.len_utf8() > 600 {
                break;
            }
            notes_preview.push(ch);
        }
        if notes_preview.len() >= 597 {
            break;
        }
    }
    Ok(Some(Release {
        version,
        release_url: release_url(&remote.html_url, base),
        notes_preview,
    }))
}

pub(super) fn newer_than(version: &Version, other: &Version) -> bool {
    version.cmp_precedence(other) == Ordering::Greater
}

pub(super) fn valid_etag(etag: &str) -> bool {
    !etag.is_empty() && etag.len() <= 512 && etag.bytes().all(|byte| (0x20..=0x7e).contains(&byte))
}

impl Response {
    pub(super) fn interpret(self, base: &str, now_ms: i64) -> Result<CheckResult, Failure> {
        match self.status {
            200 => Ok(CheckResult::Fresh {
                release: release_from_body(&self.body, base)?,
                etag: valid_etag(&self.etag).then_some(self.etag),
            }),
            304 => Ok(CheckResult::Unchanged),
            404 => Ok(CheckResult::Fresh {
                release: None,
                etag: None,
            }),
            403 | 429 => {
                let now = now_ms.div_euclid(1000);
                let until = |seconds: i64| u64::try_from(seconds.saturating_sub(now)).ok();
                let reset = self.rate_limit_reset.parse::<i64>().ok().and_then(until);
                let retry = self.retry_after.parse::<u64>().ok().or_else(|| {
                    chrono::DateTime::parse_from_rfc2822(&self.retry_after)
                        .ok()
                        .and_then(|date| until(date.timestamp()))
                });
                Err(Failure {
                    kind: CheckError::RateLimited,
                    retry_after: reset
                        .max(retry)
                        .map(|seconds| Duration::from_secs(seconds.min(MAX_RETRY_AFTER_SECS))),
                    detail: "GitHub rate limit",
                })
            }
            -1 => Err(Failure::new(CheckError::Offline, "network unavailable")),
            -2 => Err(Failure::new(CheckError::Timeout, "request timed out")),
            -4 => Err(Failure::new(
                CheckError::Failed,
                "release response exceeds 256 KB",
            )),
            _ => Err(Failure::new(CheckError::Failed, "release request failed")),
        }
    }
}

#[cfg(target_os = "macos")]
pub(super) fn fetch(endpoint: &str, etag: Option<&str>) -> Response {
    use std::ffi::{c_char, c_void, CStr, CString};

    type Callback = unsafe extern "C" fn(
        *mut c_void,
        i32,
        *const u8,
        usize,
        *const c_char,
        *const c_char,
        *const c_char,
    );
    extern "C" {
        fn dewclip_fetch_latest_release(
            url: *const c_char,
            etag: *const c_char,
            context: *mut c_void,
            callback: Callback,
        );
    }

    unsafe extern "C" fn receive(
        context: *mut c_void,
        status: i32,
        body: *const u8,
        len: usize,
        etag: *const c_char,
        reset: *const c_char,
        retry: *const c_char,
    ) {
        let header = |ptr| {
            if ptr == std::ptr::null() {
                String::new()
            } else {
                unsafe { CStr::from_ptr(ptr) }
                    .to_string_lossy()
                    .into_owned()
            }
        };
        // The bridge calls back synchronously once, with buffers alive through this function.
        *context.cast::<Response>() = Response {
            status,
            body: if len == 0 {
                Vec::new()
            } else {
                std::slice::from_raw_parts(body, len).to_vec()
            },
            etag: header(etag),
            rate_limit_reset: header(reset),
            retry_after: header(retry),
        };
    }

    let endpoint = CString::new(endpoint).expect("release endpoint without NUL");
    let etag = CString::new(etag.unwrap_or_default()).expect("validated ETag");
    let mut response = Response::default();
    unsafe {
        dewclip_fetch_latest_release(
            endpoint.as_ptr(),
            etag.as_ptr(),
            (&mut response as *mut Response).cast(),
            receive,
        );
    }
    response
}

#[cfg(not(target_os = "macos"))]
pub(super) fn fetch(_endpoint: &str, _etag: Option<&str>) -> Response {
    Response::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{"tag_name":"v1.2.0","html_url":"https://github.com/REXKEV214/dew-clip/releases/tag/v1.2.0","draft":false,"prerelease":false,"body":"Line one\n\n<em>Line two</em>\nThree\nFour\nFive\nSix"}"#;

    #[test]
    fn semantic_versions_ignore_build_metadata_and_filter_unstable_releases() {
        assert_eq!(parse_version(" v1.1.0 ").unwrap(), Version::new(1, 1, 0));
        assert_eq!(parse_version("V1.2.0").unwrap(), Version::new(1, 2, 0));
        assert!(!newer_than(
            &parse_version("1.2.0+rebuilt").unwrap(),
            &Version::new(1, 2, 0)
        ));
        assert!(newer_than(&Version::new(1, 10, 0), &Version::new(1, 2, 0)));
        assert!(newer_than(
            &Version::new(1, 2, 0),
            &parse_version("1.2.0-beta.1").unwrap()
        ));
        for body in [
            SAMPLE.replace("\"draft\":false", "\"draft\":true"),
            SAMPLE.replace("\"prerelease\":false", "\"prerelease\":true"),
            SAMPLE.replace("v1.2.0", "v1.2.0-rc.1"),
            SAMPLE.replace("v1.2.0", "v1.2.0-beta"),
            SAMPLE.replace("v1.2.0", "vv1.2.0"),
        ] {
            assert!(release_from_body(body.as_bytes(), RELEASES)
                .unwrap()
                .is_none());
        }
        let release = release_from_body(SAMPLE.as_bytes(), RELEASES)
            .unwrap()
            .unwrap();
        assert_eq!(
            release.notes_preview,
            "Line one\n<em>Line two</em>\nThree\nFour\nFive"
        );
        let mut long: serde_json::Value = serde_json::from_str(SAMPLE).unwrap();
        long["html_url"] = format!("{RELEASES}{}", "x".repeat(512 - RELEASES.len())).into();
        long["body"] = "\\".repeat(4000).into();
        let release = release_from_body(&serde_json::to_vec(&long).unwrap(), RELEASES)
            .unwrap()
            .unwrap();
        assert!(serde_json::to_vec(&release).unwrap().len() <= 2048);
    }

    #[test]
    fn release_urls_stay_under_the_repository_release_path() {
        let valid = format!("{RELEASES}tag/v1.2.0");
        assert_eq!(release_url(&valid, RELEASES), valid);
        for invalid in [
            valid.replace("github.com", "github.com.example.com"),
            valid.replace("https://", "http://"),
            valid.replace("https://", "https://user:pass@"),
            format!("{RELEASES}tag/v1.2.0\n"),
            format!("{RELEASES}%2e%2e/issues"),
            format!("{RELEASES}\\../issues"),
        ] {
            assert_eq!(release_url(&invalid, RELEASES), format!("{RELEASES}latest"));
        }
        let local = "http://127.0.0.1:49152/releases/";
        assert_eq!(
            release_url(&format!("{local}tag/v2"), local),
            format!("{local}tag/v2")
        );
    }

    #[test]
    fn interprets_http_status_and_retry_headers_without_exposing_the_body() {
        let status = |status| Response {
            status,
            ..Default::default()
        };
        assert!(matches!(
            status(304).interpret(RELEASES, 0).unwrap(),
            CheckResult::Unchanged
        ));
        assert!(matches!(
            status(404).interpret(RELEASES, 0).unwrap(),
            CheckResult::Fresh {
                release: None,
                etag: None
            }
        ));
        let response = Response {
            status: 200,
            body: SAMPLE.as_bytes().to_vec(),
            etag: "\"example\"".to_string(),
            ..Default::default()
        };
        let CheckResult::Fresh { release, etag } = response.interpret(RELEASES, 0).unwrap() else {
            panic!("expected release")
        };
        assert_eq!(release.unwrap().version, Version::new(1, 2, 0));
        assert_eq!(etag.as_deref(), Some("\"example\""));
        for (status_code, expected) in [
            (-1, CheckError::Offline),
            (-2, CheckError::Timeout),
            (-4, CheckError::Failed),
            (200, CheckError::Failed),
            (500, CheckError::Failed),
        ] {
            let response = Response {
                status: status_code,
                body: b"unexpected content".to_vec(),
                ..Default::default()
            };
            let Err(failure) = response.interpret(RELEASES, 0) else {
                panic!("expected failure")
            };
            assert_eq!(failure.kind, expected);
            assert!(!format!("{failure:?}").contains("unexpected content"));
        }
        let response = Response {
            status: 429,
            rate_limit_reset: "240".into(),
            retry_after: "Thu, 01 Jan 1970 00:05:00 GMT".into(),
            ..Default::default()
        };
        let Err(failure) = response.interpret(RELEASES, 100_000) else {
            panic!("expected rate limit")
        };
        assert_eq!(failure.kind, CheckError::RateLimited);
        assert_eq!(failure.retry_after, Some(Duration::from_secs(200)));
    }

    #[test]
    fn caps_rate_limit_delays_from_either_header_at_one_day() {
        for (reset, retry) in [
            ("9223372036854775807", ""),
            ("", "18446744073709551615"),
            ("", "Sun, 04 Jan 1970 00:00:00 GMT"),
        ] {
            let response = Response {
                status: 429,
                rate_limit_reset: reset.into(),
                retry_after: retry.into(),
                ..Default::default()
            };
            let Err(failure) = response.interpret(RELEASES, 100_000) else {
                panic!("expected rate limit");
            };
            assert_eq!(failure.retry_after, Some(Duration::from_secs(86_400)));
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn native_bridge_reports_an_invalid_endpoint_before_opening_a_connection() {
        assert!(matches!(
            fetch("no network endpoint", None).interpret(RELEASES, 0),
            Err(Failure {
                kind: CheckError::Failed,
                ..
            })
        ));
    }

    #[cfg(target_os = "macos")]
    mod native_http {
        use super::*;
        use std::io::{BufRead, BufReader, Write};
        use std::net::{TcpListener, TcpStream};
        use std::thread;

        fn exchange(
            etag: Option<&str>,
            respond: impl FnOnce(&mut TcpStream) + Send + 'static,
        ) -> (Response, String, String) {
            let listener =
                TcpListener::bind("127.0.0.1:0").expect("bind local update test HTTP server");
            let address = listener.local_addr().unwrap();
            let server = thread::spawn(move || {
                let (mut socket, _) = listener.accept().unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut reader = BufReader::new(&mut socket);
                let mut request = String::new();
                loop {
                    let mut line = String::new();
                    assert_ne!(reader.read_line(&mut line).unwrap(), 0);
                    request.push_str(&line);
                    if line == "\r\n" {
                        break;
                    }
                }
                respond(&mut socket);
                request
            });
            let base = format!("http://{address}/releases/");
            let response = fetch(&format!("{base}latest"), etag);
            (response, server.join().unwrap().to_ascii_lowercase(), base)
        }

        fn reply(socket: &mut TcpStream, status: &str, headers: &str, body: &str) {
            write!(socket, "HTTP/1.1 {status}\r\nConnection: close\r\nContent-Length: {}\r\n{headers}\r\n{body}", body.len()).unwrap();
        }

        #[test]
        fn native_get_sends_only_the_expected_application_headers_and_revalidates_etags() {
            let (response, request, base) = exchange(None, |socket| {
                reply(
                    socket,
                    "200 OK",
                    "ETag: \"release\"\r\nSet-Cookie: update=sample; Path=/\r\n",
                    SAMPLE,
                );
            });
            assert!(request.starts_with("get /releases/latest http/1.1\r\n"));
            for header in [
                "user-agent: dewclip",
                "accept: application/vnd.github+json",
                "x-github-api-version: 2022-11-28",
            ] {
                assert!(request.lines().any(|line| line == header), "{request}");
            }
            assert!(!request
                .lines()
                .any(|line| line.starts_with("if-none-match:")));
            for header in ["cookie:", "authorization:", "accept-language:"] {
                assert!(
                    !request.lines().any(|line| line
                        .strip_prefix(header)
                        .is_some_and(|value| !value.trim().is_empty())),
                    "{request}"
                );
            }
            let CheckResult::Fresh { release, etag } = response.interpret(&base, 0).unwrap() else {
                panic!("expected release")
            };
            assert_eq!(release.unwrap().version, Version::new(1, 2, 0));
            assert_eq!(etag.as_deref(), Some("\"release\""));
            let (response, request, base) = exchange(etag.as_deref(), |socket| {
                reply(socket, "304 Not Modified", "", "")
            });
            assert!(
                request
                    .lines()
                    .any(|line| line == "if-none-match: \"release\""),
                "{request}"
            );
            assert!(
                !request.lines().any(|line| line.starts_with("cookie:")),
                "{request}"
            );
            assert!(matches!(
                response.interpret(&base, 10).unwrap(),
                CheckResult::Unchanged
            ));
        }

        #[test]
        fn native_http_treats_no_release_and_rate_limits_explicitly() {
            let (response, _, base) = exchange(None, |socket| {
                reply(socket, "404 Not Found", "", "no releases")
            });
            assert!(matches!(
                response.interpret(&base, 0).unwrap(),
                CheckResult::Fresh {
                    release: None,
                    etag: None
                }
            ));
            let (response, _, base) = exchange(None, |socket| {
                reply(
                    socket,
                    "403 Forbidden",
                    "X-RateLimit-Reset: 3800\r\n",
                    "rate limit",
                );
            });
            let Err(failure) = response.interpret(&base, 100_000) else {
                panic!("expected rate limit")
            };
            assert_eq!(failure.kind, CheckError::RateLimited);
            assert_eq!(failure.retry_after, Some(Duration::from_secs(3700)));
            let (response, _, base) = exchange(None, |socket| {
                reply(
                    socket,
                    "429 Too Many Requests",
                    "X-RateLimit-Reset: 120\r\nRetry-After: 240\r\n",
                    "rate limit",
                );
            });
            let Err(failure) = response.interpret(&base, 100_000) else {
                panic!("expected rate limit")
            };
            assert_eq!(failure.kind, CheckError::RateLimited);
            assert_eq!(failure.retry_after, Some(Duration::from_secs(240)));
        }

        #[test]
        fn native_http_rejects_server_errors_malformed_json_and_oversize_streams() {
            for status in ["500 Internal Server Error", "200 OK"] {
                let (response, _, base) = exchange(None, move |socket| {
                    reply(socket, status, "", "<html>raw failure</html>")
                });
                let Err(failure) = response.interpret(&base, 0) else {
                    panic!("expected failure")
                };
                assert_eq!(failure.kind, CheckError::Failed);
                assert!(!format!("{failure:?}").contains("raw failure"));
            }
            let (response, _, base) = exchange(None, |socket| {
                write!(socket, "HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n").unwrap();
                // Without a length header the transport must enforce the cap as bytes arrive.
                let _ = socket.write_all(&vec![b'a'; 256 * 1024 + 1]);
            });
            assert!(matches!(
                response.interpret(&base, 0),
                Err(Failure {
                    detail: "release response exceeds 256 KB",
                    ..
                })
            ));
        }

        #[test]
        fn native_request_times_out_waiting_for_headers() {
            let started = std::time::Instant::now();
            let (response, _, base) = exchange(None, |_| thread::sleep(Duration::from_secs(6)));
            assert!(matches!(
                response.interpret(&base, 0),
                Err(Failure {
                    kind: CheckError::Timeout,
                    ..
                })
            ));
            assert!(started.elapsed() < Duration::from_secs(10));
        }
    }
}
