//! JSON helpers for the three families of conventions section 6.

use jiff::Timestamp;
use serde::Serializer;

/// Integral values below this magnitude are written as integers.
const INTEGER_LIMIT: f64 = 9_007_199_254_740_992.0;

/// `#[serde(serialize_with = "calendar_core::json::f64_swift")]`: writes `120.0` as `120`, as Swift's `JSONEncoder`
/// does, because C# reads some of these fields into integers. Integral values within +/-2^53 are written as integers;
/// everything else goes through `serde_json`'s shortest round-trip `f64` writer (`NaN` and infinities become `null`).
pub fn f64_swift<S: Serializer>(value: &f64, serializer: S) -> Result<S::Ok, S::Error> {
    let v = *value;
    if v.is_finite() && v.fract() == 0.0 && v.abs() < INTEGER_LIMIT {
        serializer.serialize_i64(v as i64)
    } else {
        serializer.serialize_f64(v)
    }
}

/// [`f64_swift`] for an `Option<f64>`; pair it with `skip_serializing_if = "Option::is_none"`.
pub fn f64_swift_option<S: Serializer>(
    value: &Option<f64>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match value {
        Some(v) => f64_swift(v, serializer),
        None => serializer.serialize_none(),
    }
}

/// Largest magnitude (in seconds, or milliseconds) a stored date may have; far beyond any real date, and it keeps
/// the conversions free of overflow.
const DATE_LIMIT: f64 = 1.0e14;

/// Seconds from 1970-01-01T00:00:00Z to 2001-01-01T00:00:00Z.
const REFERENCE_OFFSET_SECONDS: i64 = 978_307_200;

fn reference_instant() -> Timestamp {
    Timestamp::from_second(REFERENCE_OFFSET_SECONDS).unwrap_or(Timestamp::UNIX_EPOCH)
}

macro_rules! date_module {
    ($(#[$meta:meta])* $name:ident, $to_f64:expr, $from_f64:expr) => {
        $(#[$meta])*
        pub mod $name {
            use jiff::Timestamp;
            use serde::{Deserialize, Deserializer, Serializer, de::Error};

            /// The JSON number for `t`.
            pub fn to_number(t: &Timestamp) -> f64 {
                ($to_f64)(t)
            }

            /// The instant a JSON number stands for; `None` when it is not finite or absurdly large.
            pub fn from_number(value: f64) -> Option<Timestamp> {
                if !value.is_finite() || value.abs() >= crate::json::DATE_LIMIT {
                    return None;
                }
                // `add_seconds` saturates; a saturated result means the number was beyond the range of `Timestamp`.
                Some(($from_f64)(value)).filter(|t| *t != Timestamp::MIN && *t != Timestamp::MAX)
            }

            /// Writes `t` as a JSON number.
            pub fn serialize<S: Serializer>(t: &Timestamp, serializer: S) -> Result<S::Ok, S::Error> {
                crate::json::f64_swift(&to_number(t), serializer)
            }

            /// Reads a JSON number as an instant; fails when it is out of range.
            pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Timestamp, D::Error> {
                let value = f64::deserialize(deserializer)?;
                from_number(value).ok_or_else(|| D::Error::custom("date out of range"))
            }

            /// The same for `Option<Timestamp>`: `null` and a missing field are `None`; `None` is omitted when the field
            /// also has `skip_serializing_if = "Option::is_none"`, and written as `null` otherwise.
            pub mod option {
                use jiff::Timestamp;
                use serde::{Deserialize, Deserializer, Serializer, de::Error};

                /// Writes `Some(t)` as a JSON number and `None` as `null`.
                pub fn serialize<S: Serializer>(t: &Option<Timestamp>, serializer: S) -> Result<S::Ok, S::Error> {
                    match t {
                        Some(t) => super::serialize(t, serializer),
                        None => serializer.serialize_none(),
                    }
                }

                /// Reads a JSON number or `null`; fails when a number is out of range.
                pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Timestamp>, D::Error> {
                    match Option::<f64>::deserialize(deserializer)? {
                        None => Ok(None),
                        Some(value) => super::from_number(value)
                            .map(Some)
                            .ok_or_else(|| D::Error::custom("date out of range")),
                    }
                }
            }
        }
    };
}

date_module!(
    /// Family A dates (conventions section 6): Swift's default `Date` coding, a number of seconds since
    /// 2001-01-01T00:00:00Z, possibly fractional. `821692200` is 2027-01-15T07:50:00Z.
    swift_reference_date,
    |t: &Timestamp| {
        let seconds = t.as_second() - crate::json::REFERENCE_OFFSET_SECONDS;
        seconds as f64 + f64::from(t.subsec_nanosecond()) / 1e9
    },
    |value: f64| crate::time::add_seconds(crate::json::reference_instant(), value)
);

date_module!(
    /// The adjudication store's dates (`.millisecondsSince1970`): milliseconds since 1970-01-01T00:00:00Z.
    millis_since_1970,
    |t: &Timestamp| t.as_nanosecond() as f64 / 1e6,
    |value: f64| crate::time::add_seconds(Timestamp::UNIX_EPOCH, value / 1000.0)
);
