//! Port of `ICalText.swift`.

use crate::grapheme::{characters, is};

/// RFC 5545 TEXT escaping (section 3.3.11). Works on `Character`s, as the Swift code does.
pub enum ICalText {}

impl ICalText {
    /// Escapes TEXT per RFC 5545: backslash, semicolon, comma and newline.
    pub fn escape(text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        for character in characters(text) {
            if is(character, '\\') {
                out.push_str("\\\\");
            } else if is(character, ';') {
                out.push_str("\\;");
            } else if is(character, ',') {
                out.push_str("\\,");
            } else if character == "\n" || character == "\r\n" {
                out.push_str("\\n");
            } else if character == "\r" {
                continue;
            } else {
                out.push_str(character);
            }
        }
        out
    }

    /// Reverses `escape`: turns `\n`, `\,`, `\;` and `\\` back into the characters they stand for.
    pub fn unescape(text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut escaping = false;
        for character in characters(text) {
            if escaping {
                out.push_str(if character == "n" || character == "N" {
                    "\n"
                } else {
                    character
                });
                escaping = false;
            } else if is(character, '\\') {
                escaping = true;
            } else {
                out.push_str(character);
            }
        }
        if escaping {
            out.push('\\');
        }
        out
    }
}
