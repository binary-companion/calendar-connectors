//! Diagnostics: where a connector reports what it is doing. Port of `DiagnosticLog.swift`.
//!
//! One deliberate difference: Swift's `DiagnosticEvent(date: Date = Date())` and `record(..., date: Date = Date())`
//! read the system clock when no date is given. No crate of this library reads the clock, so the date is a required
//! argument here; callers pass the `now` they already hold.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

use jiff::Timestamp;

use crate::civil;
use crate::time::Zone;

/// How serious a diagnostic event is. Ordered, so a sink can filter with `event.level >= DiagnosticLevel::Info`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DiagnosticLevel {
    Debug,
    Info,
    Notice,
    Warning,
    Error,
}

impl DiagnosticLevel {
    /// The lower-case word used in rendered reports.
    pub fn label(&self) -> &'static str {
        match self {
            DiagnosticLevel::Debug => "debug",
            DiagnosticLevel::Info => "info",
            DiagnosticLevel::Notice => "notice",
            DiagnosticLevel::Warning => "warning",
            DiagnosticLevel::Error => "error",
        }
    }
}

/// The value of a [`DiagnosticField`] (Swift `DiagnosticField.Value`).
#[derive(Clone, Debug, PartialEq)]
pub enum DiagnosticFieldValue {
    Int(i64),
    Double(f64),
    Bool(bool),
    String(String),
}

/// One named value of a [`DiagnosticEvent`]. Privacy is by default: a string is private unless the caller says
/// otherwise, numbers and flags are public. A connector must never put a credential, a link, a title or any other
/// personal text in a field, private or not: `is_private` only decides whether a report shows the value by default.
#[derive(Clone, Debug, PartialEq)]
pub struct DiagnosticField {
    pub name: String,
    pub value: DiagnosticFieldValue,
    pub is_private: bool,
}

impl DiagnosticField {
    pub fn new(
        name: impl Into<String>,
        value: DiagnosticFieldValue,
        is_private: bool,
    ) -> DiagnosticField {
        DiagnosticField {
            name: name.into(),
            value,
            is_private,
        }
    }

    /// A public number (`DiagnosticField.int(_:_:)`); use [`DiagnosticField::new`] for a private one.
    pub fn int(name: impl Into<String>, value: i64) -> DiagnosticField {
        DiagnosticField::new(name, DiagnosticFieldValue::Int(value), false)
    }

    /// A public number.
    pub fn double(name: impl Into<String>, value: f64) -> DiagnosticField {
        DiagnosticField::new(name, DiagnosticFieldValue::Double(value), false)
    }

    /// A public flag.
    pub fn bool(name: impl Into<String>, value: bool) -> DiagnosticField {
        DiagnosticField::new(name, DiagnosticFieldValue::Bool(value), false)
    }

    /// Strings are private unless made public with [`DiagnosticField::public_string`].
    pub fn string(name: impl Into<String>, value: impl Into<String>) -> DiagnosticField {
        DiagnosticField::new(name, DiagnosticFieldValue::String(value.into()), true)
    }

    /// `.string(name, value, private: false)`: fixed tokens and host names.
    pub fn public_string(name: impl Into<String>, value: impl Into<String>) -> DiagnosticField {
        DiagnosticField::new(name, DiagnosticFieldValue::String(value.into()), false)
    }

    /// The value as text, ignoring `is_private`. For tests and for a report that has already decided what to show.
    pub fn unredacted_text(&self) -> String {
        match &self.value {
            DiagnosticFieldValue::Int(number) => number.to_string(),
            DiagnosticFieldValue::Double(number) => swift_double_description(*number),
            DiagnosticFieldValue::Bool(flag) => flag.to_string(),
            DiagnosticFieldValue::String(text) => text.clone(),
        }
    }
}

/// Something worth knowing about, with a stable `category` (the connector, for example `icalsub`) and a stable
/// camelCase `name` (for example `fetchCompleted`) that tests and support can rely on.
#[derive(Clone, Debug, PartialEq)]
pub struct DiagnosticEvent {
    pub level: DiagnosticLevel,
    pub category: String,
    pub name: String,
    pub fields: Vec<DiagnosticField>,
    pub date: Timestamp,
}

impl DiagnosticEvent {
    pub fn new(
        level: DiagnosticLevel,
        category: impl Into<String>,
        name: impl Into<String>,
        fields: Vec<DiagnosticField>,
        date: Timestamp,
    ) -> DiagnosticEvent {
        DiagnosticEvent {
            level,
            category: category.into(),
            name: name.into(),
            fields,
            date,
        }
    }

    /// The first field called `name`.
    pub fn field(&self, name: &str) -> Option<&DiagnosticField> {
        self.fields.iter().find(|field| field.name == name)
    }
}

/// Where a connector reports what it is doing. Adapters (the OS logger, a file, an in-app report) live outside the
/// library. `log` is synchronous, never fails and must not block: it is called on the connector's own path.
/// (Swift names this requirement `record(_:)`.)
pub trait DiagnosticLog: Send + Sync {
    fn log(&self, event: DiagnosticEvent);

    /// Swift's `record(_ level:, _ category:, _ name:, _ fields:, date:)` convenience.
    fn record(
        &self,
        level: DiagnosticLevel,
        category: &str,
        name: &str,
        fields: Vec<DiagnosticField>,
        date: Timestamp,
    ) {
        self.log(DiagnosticEvent::new(level, category, name, fields, date));
    }
}

/// Discards everything. The default for every connector (Swift `.none`).
#[derive(Clone, Copy, Debug, Default)]
pub struct NullDiagnosticLog;

impl DiagnosticLog for NullDiagnosticLog {
    fn log(&self, _event: DiagnosticEvent) {}
}

/// Sends each event to every log, in order.
pub struct FanOutDiagnosticLog {
    logs: Vec<Arc<dyn DiagnosticLog>>,
}

impl FanOutDiagnosticLog {
    pub fn new(logs: Vec<Arc<dyn DiagnosticLog>>) -> FanOutDiagnosticLog {
        FanOutDiagnosticLog { logs }
    }
}

impl DiagnosticLog for FanOutDiagnosticLog {
    fn log(&self, event: DiagnosticEvent) {
        for log in &self.logs {
            log.log(event.clone());
        }
    }
}

/// Keeps the last `capacity` events in memory, for an in-app report. Thread-safe.
pub struct RingBufferDiagnosticLog {
    capacity: usize,
    events: Mutex<VecDeque<DiagnosticEvent>>,
}

impl RingBufferDiagnosticLog {
    pub const DEFAULT_CAPACITY: usize = 500;

    /// A capacity below 1 is raised to 1.
    pub fn new(capacity: usize) -> RingBufferDiagnosticLog {
        RingBufferDiagnosticLog {
            capacity: capacity.max(1),
            events: Mutex::new(VecDeque::new()),
        }
    }

    /// `RingBufferDiagnosticLog()`: [`RingBufferDiagnosticLog::DEFAULT_CAPACITY`] events.
    pub fn with_default_capacity() -> RingBufferDiagnosticLog {
        RingBufferDiagnosticLog::new(RingBufferDiagnosticLog::DEFAULT_CAPACITY)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, VecDeque<DiagnosticEvent>> {
        self.events.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The kept events, oldest first.
    pub fn snapshot(&self) -> Vec<DiagnosticEvent> {
        self.lock().iter().cloned().collect()
    }

    pub fn clear(&self) {
        self.lock().clear();
    }

    /// A plain-text report, one line per event, oldest first:
    /// `2026-10-06T17:00:00Z info icalsub.fetchCompleted status=200 uid=<private>`.
    /// Private fields show `<private>` unless `include_private`.
    pub fn render(&self, include_private: bool) -> String {
        let utc = Zone::utc();
        self.snapshot()
            .iter()
            .map(|event| {
                let c = civil::components(event.date, &utc);
                let stamp = format!(
                    "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
                    c.year, c.month, c.day, c.hour, c.minute, c.second
                );
                let mut parts = vec![
                    stamp,
                    event.level.label().to_string(),
                    format!("{}.{}", event.category, event.name),
                ];
                for field in &event.fields {
                    let shown = if field.is_private && !include_private {
                        "<private>".to_string()
                    } else {
                        text(field)
                    };
                    parts.push(format!("{}={shown}", field.name));
                }
                parts.join(" ")
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl DiagnosticLog for RingBufferDiagnosticLog {
    fn log(&self, event: DiagnosticEvent) {
        let mut events = self.lock();
        events.push_back(event);
        while events.len() > self.capacity {
            events.pop_front();
        }
    }
}

/// A string value with a space, quote, equals sign or line break is quoted so a line stays one line.
fn text(field: &DiagnosticField) -> String {
    let DiagnosticFieldValue::String(value) = &field.value else {
        return field.unredacted_text();
    };
    if !value
        .chars()
        .any(|c| c.is_whitespace() || c == '"' || c == '=')
    {
        return field.unredacted_text();
    }
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r");
    format!("\"{escaped}\"")
}

/// Swift's `String(Double)`: the shortest round-trip digits, `1e+16` / `1e-05` style exponents (a sign and at least
/// two digits), `inf`, `-inf` and `nan`.
fn swift_double_description(number: f64) -> String {
    if number.is_nan() {
        return "nan".to_string();
    }
    if number.is_infinite() {
        return if number < 0.0 { "-inf" } else { "inf" }.to_string();
    }
    let shortest = format!("{number:?}");
    let Some((mantissa, exponent)) = shortest.split_once('e') else {
        return shortest;
    };
    let (sign, digits) = match exponent.strip_prefix('-') {
        Some(digits) => ('-', digits),
        None => ('+', exponent),
    };
    format!("{mantissa}e{sign}{digits:0>2}")
}
