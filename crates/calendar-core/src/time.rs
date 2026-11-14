//! Instants, zones and intervals with Foundation's semantics (conventions section 4).

use std::cmp::Ordering;
use std::future::Future;
use std::hash::{Hash, Hasher};
use std::pin::Pin;
use std::sync::Arc;

use jiff::Timestamp;
use jiff::tz::{Offset, TimeZone};

use crate::failure::Outcome;

/// Swift's `now: @Sendable () -> Date` parameters.
pub type Now = Arc<dyn Fn() -> Timestamp + Send + Sync>;

/// Swift's `Sleeper` (`(Duration) async throws -> Void`) parameters, in seconds. The engine passes one built on its
/// clock; tests pass one that returns at once and records the durations.
pub type Sleeper =
    Arc<dyn Fn(f64) -> Pin<Box<dyn Future<Output = Outcome<()>> + Send>> + Send + Sync>;

/// Foundation's limit for `TimeZone(secondsFromGMT:)`: 18 hours either side of GMT.
const MAX_FIXED_OFFSET_SECONDS: i32 = 18 * 3600;

/// Identifiers in `vectors/zones/known.json` and `vectors/zones/windows-names.json` that a zone database without the legacy "backward" links (the system
/// database of current Linux images) lacks, each with the zone it is a link to. Only used when the direct lookup
/// fails, so macOS and the bundled Windows database never reach it; the identifier a [`Zone`] reports stays the
/// spelling asked for.
const ALIASES: &[(&str, &str)] = &[
    ("America/Buenos_Aires", "America/Argentina/Buenos_Aires"),
    ("America/Godthab", "America/Nuuk"),
    ("America/Indianapolis", "America/Indiana/Indianapolis"),
    ("Antarctica/South_Pole", "Pacific/Auckland"),
    ("Asia/Calcutta", "Asia/Kolkata"),
    ("Asia/Choibalsan", "Asia/Ulaanbaatar"),
    ("Asia/Katmandu", "Asia/Kathmandu"),
    ("Asia/Rangoon", "Asia/Yangon"),
    ("EST", "America/Panama"),
    ("Europe/Kiev", "Europe/Kyiv"),
    ("Europe/Uzhgorod", "Europe/Kyiv"),
    ("Europe/Zaporozhye", "Europe/Kyiv"),
    ("GMT0", "GMT"),
    ("PST8PDT", "America/Los_Angeles"),
    ("Pacific/Enderbury", "Pacific/Kanton"),
    ("Pacific/Ponape", "Pacific/Pohnpei"),
    ("Pacific/Truk", "Pacific/Chuuk"),
    ("US/Central", "America/Chicago"),
    ("US/Eastern", "America/New_York"),
    ("US/Mountain", "America/Denver"),
    ("US/Pacific", "America/Los_Angeles"),
    ("Universal", "UTC"),
    ("Zulu", "UTC"),
];

/// Swift's `TimeZone`: an IANA zone or a fixed offset. Equality, ordering and hashing use [`Zone::identifier`].
#[derive(Clone, Debug)]
pub struct Zone {
    identifier: String,
    tz: TimeZone,
}

impl Zone {
    /// `TimeZone(identifier:)`: the exact spelling of an identifier Foundation knows (case matters), or one of the
    /// custom forms `GMT+5`, `GMT-0530`, `UTC+1`, `GMT+05:30`. `UTC` is reported as `GMT`, as Foundation does.
    pub fn named(identifier: &str) -> Option<Zone> {
        if let Some(seconds) = custom_offset(identifier) {
            return Zone::fixed(seconds);
        }
        if identifier == "UTC" {
            return Some(Zone::utc());
        }
        if !plausible_identifier(identifier) {
            return None;
        }
        let exact = |name: &str| {
            TimeZone::get(name)
                .ok()
                .filter(|tz| tz.iana_name() == Some(name))
        };
        let tz = exact(identifier).or_else(|| {
            let (_, target) = ALIASES.iter().find(|(spelled, _)| *spelled == identifier)?;
            exact(target)
        })?;
        Some(Zone {
            identifier: identifier.to_string(),
            tz,
        })
    }

    /// `TimeZone(secondsFromGMT:)`: nil beyond 18 hours. The identifier is `GMT`, or `GMT+HHMM` / `GMT-HHMM` with the offset
    /// rounded to the nearest minute.
    pub fn fixed(seconds_from_gmt: i32) -> Option<Zone> {
        if seconds_from_gmt.unsigned_abs() > MAX_FIXED_OFFSET_SECONDS.unsigned_abs() {
            return None;
        }
        let offset = Offset::from_seconds(seconds_from_gmt).ok()?;
        // The identifier names whole minutes, rounded half up (`GMT+0131` for 5,445 s); under 30 s it is `GMT`.
        let minutes = (seconds_from_gmt.unsigned_abs() + 30) / 60;
        let identifier = if minutes == 0 {
            "GMT".to_string()
        } else {
            let sign = if seconds_from_gmt < 0 { '-' } else { '+' };
            format!("GMT{sign}{:02}{:02}", minutes / 60, minutes % 60)
        };
        Some(Zone {
            identifier,
            tz: TimeZone::fixed(offset),
        })
    }

    /// `TimeZone(identifier: "UTC")`, whose identifier Foundation reports as `GMT`.
    pub fn utc() -> Zone {
        Zone {
            identifier: "GMT".to_string(),
            tz: TimeZone::UTC,
        }
    }

    /// The zone's identifier, such as `Europe/Paris` or `GMT`.
    pub fn identifier(&self) -> &str {
        &self.identifier
    }

    /// `secondsFromGMT(for:)`.
    pub fn offset_seconds(&self, at: Timestamp) -> i32 {
        self.tz.to_offset(at).seconds()
    }

    /// The underlying `jiff` time zone.
    pub fn as_jiff(&self) -> &TimeZone {
        &self.tz
    }
}

/// Only the characters of IANA names, so no lookup can be steered at a path (`../etc/localtime`).
fn plausible_identifier(identifier: &str) -> bool {
    !identifier.is_empty()
        && identifier.len() <= 64
        && identifier
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'+' | b'-' | b'/'))
        && !identifier.starts_with('/')
        && !identifier.ends_with('/')
        && !identifier.contains("//")
}

/// `GMT` or `UTC` followed by a sign and `h`, `hh`, `hhmm` or `hh:mm` (hours up to 23, minutes up to 59).
fn custom_offset(identifier: &str) -> Option<i32> {
    let rest = identifier
        .strip_prefix("GMT")
        .or_else(|| identifier.strip_prefix("UTC"))?;
    let (sign, digits) = match rest.as_bytes().first()? {
        b'+' => (1, &rest[1..]),
        b'-' => (-1, &rest[1..]),
        _ => return None,
    };
    let (hours, minutes) = match digits.split_once(':') {
        Some((h, m)) if (1..=2).contains(&h.len()) && m.len() == 2 => (h, m),
        Some(_) => return None,
        None => match digits.len() {
            1 | 2 => (digits, "00"),
            4 => digits.split_at(2),
            _ => return None,
        },
    };
    if !hours.bytes().all(|b| b.is_ascii_digit()) || !minutes.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let (h, m): (i32, i32) = (hours.parse().ok()?, minutes.parse().ok()?);
    if h > 23 || m > 59 {
        return None;
    }
    Some(sign * (h * 3600 + m * 60))
}

impl PartialEq for Zone {
    fn eq(&self, other: &Zone) -> bool {
        self.identifier == other.identifier
    }
}

impl Eq for Zone {}

impl PartialOrd for Zone {
    fn partial_cmp(&self, other: &Zone) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Zone {
    fn cmp(&self, other: &Zone) -> Ordering {
        self.identifier.cmp(&other.identifier)
    }
}

impl Hash for Zone {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.identifier.hash(state);
    }
}

/// Foundation's `DateInterval`: `contains` includes both ends and `intersects` is true when the intervals only touch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DateInterval {
    /// First instant of the interval.
    pub start: Timestamp,
    /// Last instant of the interval; not before `start`.
    pub end: Timestamp,
}

impl DateInterval {
    /// Stores both ends as given. Foundation traps when `end` is before `start`; callers here never rely on that.
    pub fn new(start: Timestamp, end: Timestamp) -> DateInterval {
        DateInterval { start, end }
    }

    /// Seconds from `start` to `end`.
    pub fn duration(&self) -> f64 {
        seconds_between(self.start, self.end)
    }

    /// `start <= t <= end`.
    pub fn contains(&self, t: Timestamp) -> bool {
        self.start <= t && t <= self.end
    }

    /// True when the intervals overlap or only touch.
    pub fn intersects(&self, other: &DateInterval) -> bool {
        self.start <= other.end && other.start <= self.end
    }
}

/// `Date.addingTimeInterval`: the sum is rounded to the nearest nanosecond. A NaN leaves `t` unchanged; a result
/// beyond the range of `Timestamp` saturates.
pub fn add_seconds(t: Timestamp, seconds: f64) -> Timestamp {
    if seconds.is_nan() {
        return t;
    }
    let nanos = (seconds * 1e9).round();
    if nanos >= 1e30 {
        return Timestamp::MAX;
    }
    if nanos <= -1e30 {
        return Timestamp::MIN;
    }
    let total = t.as_nanosecond().saturating_add(nanos as i128);
    Timestamp::from_nanosecond(total).unwrap_or(if total < 0 {
        Timestamp::MIN
    } else {
        Timestamp::MAX
    })
}

/// `to.timeIntervalSince(from)`.
pub fn seconds_between(from: Timestamp, to: Timestamp) -> f64 {
    (to.as_nanosecond() - from.as_nanosecond()) as f64 / 1e9
}
