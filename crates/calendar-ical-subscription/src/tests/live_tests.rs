//! Port of `LiveTests.swift`. Opt-in, never in CI:
//! `TIMETUG_LIVE_ICALSUB=1 cargo test -p calendar-ical-subscription -- --ignored live_feed_loads`.
//!
//! Swift reads one feed link from the git-ignored `~/.config/timetug/icalsub-live`, loads it through the default
//! transport and prints counts only. The Rust library has no default transport, and a test must not invent one, so the
//! ignored test checks what the library can check on its own (the link in that file is an acceptable feed link) and
//! stops where a host would supply its own `HttpTransport`. The link is a credential: nothing is printed.

use crate::feed_location::FeedLocation;

#[test]
#[ignore = "live: needs TIMETUG_LIVE_ICALSUB=1 and a link in ~/.config/timetug/icalsub-live"]
fn live_feed_loads() {
    if std::env::var("TIMETUG_LIVE_ICALSUB").as_deref() != Ok("1") {
        return;
    }
    let home = std::env::var("HOME").unwrap_or_default();
    let text =
        std::fs::read_to_string(format!("{home}/.config/timetug/icalsub-live")).unwrap_or_default();
    assert!(
        FeedLocation::url_from(&text).is_ok(),
        "the live link is missing or is not an acceptable feed link"
    );
}
