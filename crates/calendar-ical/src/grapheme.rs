//! Swift `Character` semantics for the few places `ICalendar` is written in terms of `Character`s: a content line is an
//! `Array(line)`, TEXT escaping loops over `Character`s, and `prefix(n)` counts them. A `Character` is an extended
//! grapheme cluster, so `";"` followed by a combining mark is not a semicolon and a combining mark after a folded
//! line's leading space means the line is not a continuation. No Unicode segmentation crate is allowed, so the
//! cluster rules (UAX 29 GB3 to GB13) are written here over four tables read from Swift (`grapheme_tables.rs`).
//!
//! Known gap, which cannot change which ASCII separator is found: GB9 and GB11 treat a spacing mark like an extender
//! (they differ only for an emoji followed by a spacing mark and a joiner, and for a virama after one).

use crate::grapheme_tables::{
    CONTROL, EXTEND_LIKE, EXTENDED_PICTOGRAPHIC, INDIC_CONSONANT, PREPEND,
};

fn in_table(table: &[(u32, u32)], c: char) -> bool {
    let value = c as u32;
    table
        .binary_search_by(|&(low, high)| {
            if value < low {
                std::cmp::Ordering::Greater
            } else if value > high {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

fn is_regional_indicator(c: char) -> bool {
    ('\u{1F1E6}'..='\u{1F1FF}').contains(&c)
}

fn is_hangul_l(c: char) -> bool {
    matches!(c as u32, 0x1100..=0x115F | 0xA960..=0xA97C)
}

fn is_hangul_v(c: char) -> bool {
    matches!(c as u32, 0x1160..=0x11A7 | 0xD7B0..=0xD7C6)
}

fn is_hangul_t(c: char) -> bool {
    matches!(c as u32, 0x11A8..=0x11FF | 0xD7CB..=0xD7FB)
}

fn is_hangul_lv(c: char) -> bool {
    matches!(c as u32, 0xAC00..=0xD7A3) && (c as u32 - 0xAC00).is_multiple_of(28)
}

fn is_hangul_lvt(c: char) -> bool {
    matches!(c as u32, 0xAC00..=0xD7A3) && !(c as u32 - 0xAC00).is_multiple_of(28)
}

const ZERO_WIDTH_JOINER: char = '\u{200D}';

/// The Indic conjunct rule (GB9c): `Consonant [Extend Linker]* Linker [Extend Linker]* x Consonant`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Conjunct {
    No,
    /// Inside `Consonant Extend*`.
    Consonant,
    /// After a linker (a virama) that follows a consonant.
    Linked,
}

/// The viramas that link consonants (InCB=Linker).
fn is_linker(c: char) -> bool {
    matches!(
        c as u32,
        0x094D | 0x09CD | 0x0ACD | 0x0B4D | 0x0C4D | 0x0D4D
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Pictographic {
    /// Not inside `ExtPict Extend*`.
    No,
    /// Inside `ExtPict Extend*`.
    Run,
    /// After `ExtPict Extend* ZWJ`.
    Joined,
}

/// Whether a cluster boundary lies between `previous` and `next` when no emoji or flag state applies.
fn breaks_between(previous: char, next: char) -> bool {
    if previous == '\r' && next == '\n' {
        return false;
    }
    if previous.is_ascii() && next.is_ascii() {
        return true;
    }
    if in_table(CONTROL, previous) || in_table(CONTROL, next) {
        return true;
    }
    if is_hangul_l(previous)
        && (is_hangul_l(next) || is_hangul_v(next) || is_hangul_lv(next) || is_hangul_lvt(next))
    {
        return false;
    }
    if (is_hangul_lv(previous) || is_hangul_v(previous)) && (is_hangul_v(next) || is_hangul_t(next))
    {
        return false;
    }
    if (is_hangul_lvt(previous) || is_hangul_t(previous)) && is_hangul_t(next) {
        return false;
    }
    if in_table(EXTEND_LIKE, next) {
        return false;
    }
    if in_table(PREPEND, previous) {
        return false;
    }
    true
}

/// The `Character`s of a string, lazily.
pub(crate) struct Characters<'a> {
    rest: &'a str,
}

impl<'a> Iterator for Characters<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        let mut chars = self.rest.char_indices();
        let (_, first) = chars.next()?;
        let mut regional_indicators = usize::from(is_regional_indicator(first));
        let mut pictographic = if in_table(EXTENDED_PICTOGRAPHIC, first) {
            Pictographic::Run
        } else {
            Pictographic::No
        };
        let mut conjunct = if in_table(INDIC_CONSONANT, first) {
            Conjunct::Consonant
        } else {
            Conjunct::No
        };
        let mut previous = first;
        let mut end = self.rest.len();
        for (index, next) in chars {
            let mut breaks = breaks_between(previous, next);
            if breaks {
                let emoji_sequence =
                    pictographic == Pictographic::Joined && in_table(EXTENDED_PICTOGRAPHIC, next);
                let flag_pair = is_regional_indicator(previous)
                    && is_regional_indicator(next)
                    && regional_indicators % 2 == 1;
                let conjunct_link = conjunct == Conjunct::Linked && in_table(INDIC_CONSONANT, next);
                breaks = !(emoji_sequence || flag_pair || conjunct_link);
            }
            if breaks {
                end = index;
                break;
            }
            regional_indicators = if is_regional_indicator(next) {
                regional_indicators + 1
            } else {
                0
            };
            pictographic = if in_table(EXTENDED_PICTOGRAPHIC, next) {
                Pictographic::Run
            } else if next == ZERO_WIDTH_JOINER && pictographic == Pictographic::Run {
                Pictographic::Joined
            } else if in_table(EXTEND_LIKE, next) && pictographic == Pictographic::Run {
                Pictographic::Run
            } else {
                Pictographic::No
            };
            conjunct = if in_table(INDIC_CONSONANT, next) {
                Conjunct::Consonant
            } else if conjunct != Conjunct::No && in_table(EXTEND_LIKE, next) {
                if is_linker(next) {
                    Conjunct::Linked
                } else {
                    conjunct
                }
            } else {
                Conjunct::No
            };
            previous = next;
        }
        let (character, rest) = self.rest.split_at(end);
        self.rest = rest;
        Some(character)
    }
}

/// `Array(text)`: the extended grapheme clusters.
pub(crate) fn characters(text: &str) -> Characters<'_> {
    Characters { rest: text }
}

/// `character == "x"` for an ASCII `x`. Swift compares `Character`s by canonical equivalence, and U+037E (Greek
/// question mark) is canonically equivalent to `;`.
pub(crate) fn is(character: &str, expected: char) -> bool {
    let mut chars = character.chars();
    match (chars.next(), chars.next()) {
        (Some(only), None) => only == expected || (expected == ';' && only == '\u{37E}'),
        _ => false,
    }
}

/// `text.prefix(count)`.
pub(crate) fn prefix(text: &str, count: usize) -> &str {
    let mut end = 0;
    for character in characters(text).take(count) {
        end += character.len();
    }
    &text[..end]
}

/// `text.dropFirst(count)`.
pub(crate) fn drop_first(text: &str, count: usize) -> &str {
    &text[prefix(text, count).len()..]
}

/// `text.split(separator: ch)`: splits on the `Character` (omitting empty pieces, as Swift does by default).
pub(crate) fn split(text: &str, separator: char) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut start = 0;
    let mut offset = 0;
    for character in characters(text) {
        if is(character, separator) {
            if offset > start {
                pieces.push(&text[start..offset]);
            }
            start = offset + character.len();
        }
        offset += character.len();
    }
    if offset > start {
        pieces.push(&text[start..offset]);
    }
    pieces
}
