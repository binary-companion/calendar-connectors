use jiff::Timestamp;
use serde::{Deserialize, Serialize};

use crate::json::{f64_swift, millis_since_1970, swift_reference_date};

#[derive(Serialize)]
struct Number {
    #[serde(serialize_with = "f64_swift")]
    value: f64,
}

fn render(value: f64) -> String {
    serde_json::to_string(&Number { value }).unwrap()
}

#[test]
fn f64_swift_writes_integral_values_as_integers() {
    assert_eq!(render(120.0), "{\"value\":120}");
    assert_eq!(render(0.5), "{\"value\":0.5}");
    assert_eq!(render(-3.0), "{\"value\":-3}");
    assert_eq!(render(0.0), "{\"value\":0}");
    assert_eq!(render(-0.0), "{\"value\":0}");
    assert_eq!(render(1.0e-7), "{\"value\":1e-7}");
}

#[test]
fn f64_swift_integer_cutoff_is_two_to_the_53() {
    // Just below 2^53 is an integer; 2^53 and above go through the f64 writer.
    assert_eq!(
        render(9_007_199_254_740_991.0),
        "{\"value\":9007199254740991}"
    );
    let at = render(9_007_199_254_740_992.0);
    assert_eq!(at, "{\"value\":9007199254740992.0}");
    assert_eq!(
        render(-9_007_199_254_740_991.0),
        "{\"value\":-9007199254740991}"
    );
    // Beyond i64 never overflows.
    let big = render(1e300);
    let back: serde_json::Value = serde_json::from_str(&big).unwrap();
    assert_eq!(back["value"].as_f64(), Some(1e300));
}

#[test]
fn f64_swift_non_finite_does_not_panic() {
    for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(render(v), "{\"value\":null}");
    }
}

#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct Ref {
    #[serde(with = "swift_reference_date")]
    at: Timestamp,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "swift_reference_date::option"
    )]
    maybe: Option<Timestamp>,
}

#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct Millis {
    #[serde(with = "millis_since_1970")]
    at: Timestamp,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "millis_since_1970::option"
    )]
    maybe: Option<Timestamp>,
}

#[test]
fn swift_reference_date_reads_seconds_since_2001() {
    let r: Ref = serde_json::from_str("{\"at\":821692200}").unwrap();
    // 821692200 + 978307200 = 1799999400 Unix seconds. (The plan text says 06:30:00Z; the arithmetic gives 07:50:00Z.)
    assert_eq!(r.at, "2027-01-15T07:50:00Z".parse::<Timestamp>().unwrap());
    assert_eq!(r.maybe, None);
    let json = serde_json::to_string(&r).unwrap();
    assert_eq!(json, "{\"at\":821692200}");
}

#[test]
fn swift_reference_date_fractional_and_reference_instant() {
    let r: Ref = serde_json::from_str("{\"at\":0.5,\"maybe\":-1}").unwrap();
    assert_eq!(r.at, "2001-01-01T00:00:00.5Z".parse::<Timestamp>().unwrap());
    assert_eq!(
        r.maybe,
        Some("2000-12-31T23:59:59Z".parse::<Timestamp>().unwrap())
    );
    assert_eq!(
        serde_json::to_string(&r).unwrap(),
        "{\"at\":0.5,\"maybe\":-1}"
    );
    let nulled: Ref = serde_json::from_str("{\"at\":3,\"maybe\":null}").unwrap();
    assert_eq!(nulled.maybe, None);
    // Fractional seconds survive a round trip to the millisecond.
    let t = "2026-11-10T12:34:56.789Z".parse::<Timestamp>().unwrap();
    let json = serde_json::to_string(&Ref { at: t, maybe: None }).unwrap();
    let back: Ref = serde_json::from_str(&json).unwrap();
    let drift = (back.at.as_nanosecond() - t.as_nanosecond()).abs();
    assert!(drift < 1_000, "drift of {drift} ns");
    assert!(serde_json::from_str::<Ref>("{\"at\":\"x\"}").is_err());
    assert!(serde_json::from_str::<Ref>("{\"at\":1e300}").is_err());
}

#[test]
fn millis_since_1970_reads_and_writes_integers() {
    let m: Millis = serde_json::from_str("{\"at\":1799999400000,\"maybe\":1}").unwrap();
    assert_eq!(m.at, "2027-01-15T07:50:00Z".parse::<Timestamp>().unwrap());
    assert_eq!(m.maybe, Some(Timestamp::from_millisecond(1).unwrap()));
    assert_eq!(
        serde_json::to_string(&m).unwrap(),
        "{\"at\":1799999400000,\"maybe\":1}"
    );
    let n: Millis = serde_json::from_str("{\"at\":-5}").unwrap();
    assert_eq!(serde_json::to_string(&n).unwrap(), "{\"at\":-5}");
    // A fractional number is accepted and rounded to the millisecond.
    let f: Millis = serde_json::from_str("{\"at\":1500.4}").unwrap();
    assert_eq!(f.at.as_millisecond(), 1500);
    assert!(serde_json::from_str::<Millis>("{\"at\":\"x\"}").is_err());
}
