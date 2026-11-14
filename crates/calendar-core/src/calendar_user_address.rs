//! Reads a calendar user address (an attendee's or organizer's `mailto:` URL or address) as an email. Port of
//! `CalendarUserAddress.swift`.

use crate::url_string::percent_decode_strict;

/// The one shared parser, with no I/O, so every connector agrees. An address it cannot turn into an email (an opaque
/// `urn:uuid:` id, a CalDAV or iCloud principal URL, an Exchange directory path) is `None`: the connector may still
/// resolve it by other means, but the library keeps no raw identifier.
pub enum CalendarUserAddress {}

impl CalendarUserAddress {
    /// Swift's `email(from:)`.
    pub fn email_from(address: Option<&str>) -> Option<String> {
        let trimmed = address?.trim();
        if trimmed.is_empty() {
            return None;
        }
        let mut text = trimmed.to_string();
        let lowered = text.to_lowercase();
        if lowered.starts_with("mailto:") {
            let mut rest: String = text.chars().skip("mailto:".len()).collect();
            if let Some(query) = rest.find('?') {
                rest.truncate(query);
            }
            text = percent_decode_strict(&rest).unwrap_or(rest);
        } else if lowered.starts_with("urn:") {
            // A URN whose last part is an email address: `urn:foo:bar:user@example.com`.
            text = text
                .split(':')
                .rfind(|part| !part.is_empty())
                .unwrap_or("")
                .to_string();
        }
        let text = text.trim().to_lowercase();
        is_email(&text).then_some(text)
    }
}

/// One `@` with something on both sides and no whitespace, control characters or `, ; < > : /`.
fn is_email(text: &str) -> bool {
    let mut parts = text.split('@');
    let (Some(local), Some(domain), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    if local.is_empty() || domain.is_empty() {
        return false;
    }
    !text.chars().any(|c| {
        c.is_whitespace() || c.is_control() || is_format_character(c) || ",;<>:/".contains(c)
    })
}

/// Unicode general category Cf, which Foundation's `CharacterSet.controlCharacters` includes beside Cc.
fn is_format_character(c: char) -> bool {
    matches!(c as u32,
        0x00AD | 0x0600..=0x0605 | 0x061C | 0x06DD | 0x070F | 0x0890..=0x0891 | 0x08E2 | 0x180E
        | 0x200B..=0x200F | 0x202A..=0x202E | 0x2060..=0x2064 | 0x2066..=0x206F | 0xFEFF
        | 0xFFF9..=0xFFFB | 0x110BD | 0x110CD | 0x13430..=0x1343F | 0x1BCA0..=0x1BCA3
        | 0x1D173..=0x1D17A | 0xE0001 | 0xE0020..=0xE007F)
}
