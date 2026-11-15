//! Helpers of the opt-in live tests (`live_write_smoke.rs` of `calendar-google` and `calendar-microsoft`): a real HTTP
//! transport (the system `curl`), a browser sign-in on a loopback address, and a cleaner that deletes only the events
//! a smoke test created. The library has no HTTP client, so these are test code, not a model for a host.
//!
//! Unix only: the transport runs `curl`, and the sign-in opens the browser with `open` (macOS) or `xdg-open`.

#![allow(clippy::print_stdout)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use calendar_core::connection::{AuthorizationInteraction, CredentialField, OAuthRedirectSession};
use calendar_core::failure::{Failure, Outcome};
use calendar_core::http::{HttpRequest, HttpResponse, HttpTransport};
use calendar_core::model::CalendarEvent;
use calendar_core::source_types::{CalendarSource, SourceError};
use calendar_core::time::{DateInterval, Now, Sleeper};
use calendar_core::url_string::UrlString;
use calendar_core::write::types::{EventRef, NotifyPolicy, RecurrenceScope};
use jiff::Timestamp;

/// True when the environment variable `name` is `1` (the switch of a live test, for example `TIMETUG_LIVE_GOOGLE`).
pub fn switch_on(name: &str) -> bool {
    std::env::var(name).is_ok_and(|value| value == "1")
}

/// The value of an environment variable a live test needs. The panic names the variable, never its value.
pub fn require_env(name: &str) -> String {
    match std::env::var(name) {
        Ok(value) if !value.is_empty() => value,
        _ => panic!("the live test needs the environment variable {name}"),
    }
}

/// The wall clock, for a connector kind.
pub fn live_now() -> Now {
    Arc::new(Timestamp::now)
}

/// A sleeper that really waits, for a connector kind.
pub fn live_sleeper() -> Sleeper {
    Arc::new(|seconds| {
        Box::pin(async move {
            tokio::time::sleep(Duration::from_secs_f64(seconds.max(0.0))).await;
            Ok(())
        })
    })
}

fn host_failure(message: &str) -> Failure {
    Failure::Host(message.to_string())
}

/// A directory only this process can read, removed on drop. Request headers (which carry the bearer token) and bodies
/// (which carry the client secret and refresh token at sign-in) pass through it, never through the command line.
struct PrivateDir(PathBuf);

impl PrivateDir {
    fn new() -> std::io::Result<PrivateDir> {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.subsec_nanos());
        let path = std::env::temp_dir().join(format!(
            "calendar-live-{}-{nanos}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        fs::DirBuilder::new().mode(0o700).create(&path)?;
        Ok(PrivateDir(path))
    }

    fn write(&self, name: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
        let path = self.0.join(name);
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?;
        file.write_all(bytes)?;
        Ok(path)
    }
}

impl Drop for PrivateDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// Parses the header dump of `curl -D`: the last block (earlier ones are `100 Continue` or proxy answers).
fn parse_headers(dump: &str) -> Option<(i64, BTreeMap<String, String>)> {
    let block = dump
        .split("\r\n\r\n")
        .filter(|block| block.trim_start().starts_with("HTTP/"))
        .last()?;
    let mut lines = block.trim_start().lines();
    let status = lines.next()?.split_whitespace().nth(1)?.parse().ok()?;
    let mut headers = BTreeMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_string(), value.trim().to_string());
        }
    }
    Some((status, headers))
}

fn run_curl(request: &HttpRequest) -> Outcome<HttpResponse> {
    let dir =
        PrivateDir::new().map_err(|_| host_failure("could not create a private directory"))?;
    let mut header_text = String::from("Expect:\n");
    for (name, value) in &request.headers {
        header_text.push_str(&format!("{name}: {value}\n"));
    }
    let header_file = dir
        .write("request-headers", header_text.as_bytes())
        .map_err(|_| host_failure("could not write the request headers"))?;
    let out_headers = dir.0.join("response-headers");
    let out_body = dir.0.join("response-body");
    let mut command = Command::new("curl");
    command
        .arg("-q")
        .args(["-sS", "--max-time", "120", "--connect-timeout", "30"])
        .args(["-X", request.method.as_str()])
        .arg("-H")
        .arg(format!("@{}", path_text(&header_file)))
        .arg("-D")
        .arg(&out_headers)
        .arg("-o")
        .arg(&out_body);
    if let Some(body) = &request.body {
        let body_file = dir
            .write("request-body", body)
            .map_err(|_| host_failure("could not write the request body"))?;
        command
            .arg("--data-binary")
            .arg(format!("@{}", path_text(&body_file)));
    }
    command.arg("--url").arg(request.url.as_str());
    let output = command
        .output()
        .map_err(|_| host_failure("could not run curl"))?;
    if !output.status.success() {
        // curl's own message can repeat the address, so only the exit code is reported.
        let code = output
            .status
            .code()
            .map_or("none".to_string(), |c| c.to_string());
        return Err(Failure::Source(SourceError::Network(format!(
            "curl failed with exit code {code}"
        ))));
    }
    let dump = fs::read(&out_headers).map_err(|_| host_failure("curl wrote no headers"))?;
    let (status, headers) = parse_headers(&String::from_utf8_lossy(&dump)).ok_or_else(|| {
        Failure::Source(SourceError::InvalidResponse("no status line".to_string()))
    })?;
    let body = fs::read(&out_body).unwrap_or_default();
    Ok(HttpResponse::new(status, headers, body))
}

/// An `HttpTransport` over the system `curl`: every answer is a response, nothing is cached, no cookies are kept and a
/// redirect is returned, not followed (`docs/host-http-contract.md`). Headers and bodies pass through private files.
pub struct CurlTransport;

#[async_trait]
impl HttpTransport for CurlTransport {
    async fn send(&self, request: HttpRequest) -> Outcome<HttpResponse> {
        match tokio::task::spawn_blocking(move || run_curl(&request)).await {
            Ok(outcome) => outcome,
            Err(error) if error.is_cancelled() => Err(Failure::Cancelled),
            Err(_) => Err(host_failure("the curl task failed")),
        }
    }
}

/// Opens `url` in the user's browser.
fn open_in_browser(url: &str) -> bool {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    Command::new(opener)
        .arg(url)
        .status()
        .is_ok_and(|status| status.success())
}

/// Reads one request head from `stream` and returns its request target (`/?code=...`).
fn read_target(stream: &mut TcpStream) -> Option<String> {
    stream.set_nonblocking(false).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 2048];
    while !buffer.windows(4).any(|window| window == b"\r\n\r\n") && buffer.len() < 16_384 {
        let count = stream.read(&mut chunk).ok()?;
        if count == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..count]);
    }
    let text = String::from_utf8_lossy(&buffer);
    let line = text.lines().next()?;
    let mut parts = line.split_whitespace();
    (parts.next()? == "GET").then(|| parts.next().map(str::to_string))?
}

fn answer(stream: &mut TcpStream, found: bool) {
    let (status, body) = if found {
        (
            "200 OK",
            "<!doctype html><title>Live test</title><p>Signed in. You can close this window.</p>",
        )
    } else {
        ("404 Not Found", "")
    };
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body.as_bytes());
}

/// Waits on `listener` for the redirect that carries `code` or `error`; other requests (a favicon) get a 404.
fn wait_for_redirect(listener: &TcpListener, port: u16, timeout: Duration) -> Outcome<UrlString> {
    listener
        .set_nonblocking(true)
        .map_err(|_| host_failure("could not listen"))?;
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        match listener.accept() {
            Ok((mut stream, _)) => {
                let target = read_target(&mut stream);
                let is_redirect = target.as_deref().is_some_and(|t| {
                    t.split_once('?').is_some_and(|(_, query)| {
                        query
                            .split('&')
                            .any(|pair| pair.starts_with("code=") || pair.starts_with("error="))
                    })
                });
                answer(&mut stream, is_redirect);
                if let (true, Some(target)) = (is_redirect, target) {
                    return UrlString::parse(&format!("http://127.0.0.1:{port}{target}"))
                        .ok_or_else(|| host_failure("the redirect was not a URL"));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(_) => return Err(host_failure("the loopback listener failed")),
        }
    }
    Err(host_failure("timed out waiting for the browser sign-in"))
}

struct LoopbackSession {
    listener: TcpListener,
    port: u16,
}

#[async_trait]
impl OAuthRedirectSession for LoopbackSession {
    fn redirect_uri(&self) -> UrlString {
        // `http://127.0.0.1:<port>` always parses.
        UrlString::parse(&format!("http://127.0.0.1:{}", self.port)).unwrap()
    }

    async fn authorize(&self, authorization_url: &UrlString) -> Outcome<UrlString> {
        if !open_in_browser(authorization_url.as_str()) {
            return Err(host_failure("could not open the browser"));
        }
        let listener = self
            .listener
            .try_clone()
            .map_err(|_| host_failure("could not share the listener"))?;
        let port = self.port;
        match tokio::task::spawn_blocking(move || {
            wait_for_redirect(&listener, port, Duration::from_secs(300))
        })
        .await
        {
            Ok(outcome) => outcome,
            Err(_) => Err(Failure::Cancelled),
        }
    }

    async fn close(&self) {}
}

/// Sign-in through the user's browser with a redirect to a loopback port. There is no credential form.
pub struct BrowserInteraction;

#[async_trait]
impl AuthorizationInteraction for BrowserInteraction {
    async fn begin_oauth_redirect(&self) -> Outcome<Box<dyn OAuthRedirectSession>> {
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .map_err(|_| host_failure("could not listen on a loopback port"))?;
        let port = listener
            .local_addr()
            .map_err(|_| host_failure("could not read the loopback port"))?
            .port();
        Ok(Box::new(LoopbackSession { listener, port }))
    }

    async fn prompt_credentials(
        &self,
        _fields: &[CredentialField],
    ) -> Outcome<BTreeMap<String, String>> {
        Err(host_failure("the live tests have no credential form"))
    }
}

/// Deletes the events a smoke test created. Every delete goes through here, so only events whose title starts with the
/// prefix are ever touched and each series (or single event) is deleted at most once, whichever path reaches it first.
pub struct SmokeCleaner {
    source: Arc<dyn CalendarSource>,
    prefix: String,
    calendar_id: String,
    window: DateInterval,
    deleted_keys: Mutex<BTreeSet<String>>,
    created: Mutex<Vec<CalendarEvent>>,
}

impl SmokeCleaner {
    /// A cleaner for the events of `source` in `calendar_id` and `window` whose title starts with `prefix`.
    pub fn new(
        source: Arc<dyn CalendarSource>,
        prefix: &str,
        calendar_id: &str,
        window: DateInterval,
    ) -> SmokeCleaner {
        SmokeCleaner {
            source,
            prefix: prefix.to_string(),
            calendar_id: calendar_id.to_string(),
            window,
            deleted_keys: Mutex::new(BTreeSet::new()),
            created: Mutex::new(Vec::new()),
        }
    }

    /// Records an event the test created (a series master), so a failure can delete it even if listing fails.
    pub fn record_created(&self, event: &CalendarEvent) {
        self.created
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(event.clone());
    }

    /// Our events in the calendar, oldest first. The prefix and calendar filter keeps every delete away from the
    /// user's own events.
    pub async fn mine(&self) -> Outcome<Vec<CalendarEvent>> {
        let mut events: Vec<CalendarEvent> = self
            .source
            .events_in(self.window)
            .await?
            .into_iter()
            .filter(|event| {
                event.title.starts_with(&self.prefix) && event.calendar_id == self.calendar_id
            })
            .collect();
        events.sort_by_key(|event| event.start);
        Ok(events)
    }

    /// Polls `mine()` every second, up to 15 times, until `ready` holds. When it never does, the last list is
    /// returned, so the caller's check reports what was seen.
    pub async fn poll(
        &self,
        ready: impl Fn(&[CalendarEvent]) -> bool,
    ) -> Outcome<Vec<CalendarEvent>> {
        let mut list = self.mine().await?;
        let mut tries = 0;
        while !ready(&list) && tries < 15 {
            tokio::time::sleep(Duration::from_secs(1)).await;
            list = self.mine().await?;
            tries += 1;
        }
        Ok(list)
    }

    /// Deletes `event` with `scope`. A series is keyed by its master id (`series_id`, or `event_id` for the master
    /// itself). A key is recorded only after the delete succeeded, so a failed delete can be retried but a deleted
    /// series is never deleted again.
    pub async fn remove(&self, event: &CalendarEvent, scope: RecurrenceScope) -> Outcome<()> {
        if !event.title.starts_with(&self.prefix) {
            return Ok(());
        }
        let key = event.series_id().unwrap_or(&event.event_id).to_string();
        if self
            .deleted_keys
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&key)
        {
            return Ok(());
        }
        let Some(writable) = self.source.as_writable() else {
            return Err(host_failure("the source cannot write"));
        };
        writable
            .delete(&EventRef::from_event(event), scope, NotifyPolicy::None)
            .await?;
        self.deleted_keys
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key);
        Ok(())
    }

    /// Deletes the recorded events, then every smoke event still in the window (this finds the new series a split
    /// created). Failures are printed, not returned.
    pub async fn clean_up(&self) {
        let created = self
            .created
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        for event in &created {
            if let Err(error) = self.remove(event, RecurrenceScope::AllInSeries).await {
                println!("LIVE cleanup could not delete \"{}\": {error}", event.title);
            }
        }
        match self.mine().await {
            Ok(events) => {
                for event in &events {
                    if let Err(error) = self.remove(event, RecurrenceScope::AllInSeries).await {
                        println!("LIVE cleanup could not delete \"{}\": {error}", event.title);
                    }
                }
            }
            Err(error) => println!("LIVE cleanup could not list events: {error}"),
        }
    }
}
