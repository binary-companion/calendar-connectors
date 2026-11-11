//! Port of `ICloudLiveSmokeTests.swift`. Opt-in, never in CI: `TIMETUG_LIVE_ICLOUD=1 cargo test -p calendar-caldav --
//! --ignored i_cloud_live_smoke`.
//!
//! Swift builds `ICloudConnectorKind()` with its default `URLSessionTransport`, signs in with an Apple ID and an
//! app-specific password read from the git-ignored `~/.config/timetug/icloud-live`, and writes a few events to a
//! calendar named "TimeTug Live Test". The Rust library has no default transport, and a test must not invent one, so
//! the ignored test checks what the library can check on its own (the file holds two non-empty lines) and stops where a
//! host would supply its own `HttpTransport`. The credentials are never printed.

#[test]
#[ignore = "live: needs TIMETUG_LIVE_ICLOUD=1, ~/.config/timetug/icloud-live and a host HttpTransport"]
fn i_cloud_live_smoke() {
    if std::env::var("TIMETUG_LIVE_ICLOUD").as_deref() != Ok("1") {
        return;
    }
    let home = std::env::var("HOME").unwrap_or_default();
    let text =
        std::fs::read_to_string(format!("{home}/.config/timetug/icloud-live")).unwrap_or_default();
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    assert!(
        lines.len() >= 2 && !lines[0].is_empty() && !lines[1].is_empty(),
        "~/.config/timetug/icloud-live needs two lines: the Apple ID, then an app-specific password"
    );
}
