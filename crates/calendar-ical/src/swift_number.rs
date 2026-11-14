//! `Double(String)` as Swift parses it: C `strtod` text that must be consumed completely (no spaces), so hexadecimal
//! floats (`0x1p3`) and `inf`/`nan` are numbers too.

/// Swift `Double(_ text:)`.
pub(crate) fn swift_double(text: &str) -> Option<f64> {
    let (negative, body) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    if let Some(hex) = body.strip_prefix("0x") {
        let value = hex_float(hex)?;
        return Some(if negative { -value } else { value });
    }
    if body.is_empty() || body.starts_with(['+', '-']) {
        return None;
    }
    if let Some(nan) = quiet_or_signaling_nan(body) {
        return Some(nan);
    }
    // Rust and C agree on decimal forms, `inf` and `infinity` (any case).
    if body.to_ascii_lowercase().starts_with("nan") {
        return None;
    }
    text.parse::<f64>().ok()
}

/// The digits after `0x`: `h[h...][.h[h...]][p[+-]d[d...]]`.
fn hex_float(text: &str) -> Option<f64> {
    let (mantissa, exponent) = match text.find(['p', 'P']) {
        Some(index) => (&text[..index], Some(&text[index + 1..])),
        None => (text, None),
    };
    let (whole, fraction) = match mantissa.split_once('.') {
        Some((whole, fraction)) => (whole, fraction),
        None => (mantissa, ""),
    };
    if whole.is_empty() && fraction.is_empty() {
        return None;
    }
    let mut value = 0.0_f64;
    for c in whole.chars() {
        value = value * 16.0 + f64::from(c.to_digit(16)?);
    }
    let mut scale = 1.0 / 16.0;
    for c in fraction.chars() {
        value += f64::from(c.to_digit(16)?) * scale;
        scale /= 16.0;
    }
    let exponent: i32 = match exponent {
        Some(text) => {
            let digits = text.strip_prefix(['+', '-']).unwrap_or(text);
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            text.parse().ok()?
        }
        None => 0,
    };
    Some(value * 2.0_f64.powi(exponent))
}

/// `nan`, `snan`, and either with a payload in parentheses (decimal digits or `0x` and hex digits), any case.
fn quiet_or_signaling_nan(body: &str) -> Option<f64> {
    let lower = body.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("snan")
        .or_else(|| lower.strip_prefix("nan"))?;
    if rest.is_empty() {
        return Some(f64::NAN);
    }
    let payload = rest.strip_prefix('(')?.strip_suffix(')')?;
    let valid = payload.is_empty()
        || payload.bytes().all(|b| b.is_ascii_digit())
        || payload
            .strip_prefix("0x")
            .is_some_and(|hex| !hex.is_empty() && hex.bytes().all(|b| b.is_ascii_hexdigit()));
    valid.then_some(f64::NAN)
}
