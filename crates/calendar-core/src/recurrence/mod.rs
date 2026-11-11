//! RFC 5545 recurrence: rules, their expansion on wall-clock time, and the sets that hold a series' repetition.
//! Port of `CalendarCore/Recurrence/`.

pub mod expander;
pub mod occurrences;
pub mod rule;
pub mod set;

/// Swift's `Collection.split(separator:maxSplits:omittingEmptySubsequences:)` for one separator character, with the
/// stdlib's exact treatment of empty pieces: an omitted empty piece does not count towards `max_splits`, so
/// `"=A=B"` split once gives `["A", "B"]`.
pub(crate) fn swift_split(
    text: &str,
    separator: char,
    max_splits: Option<usize>,
    omit_empty: bool,
) -> Vec<&str> {
    let mut result: Vec<&str> = Vec::new();
    if max_splits == Some(0) || text.is_empty() {
        if !(omit_empty && text.is_empty()) {
            result.push(text);
        }
        return result;
    }
    let mut start = 0;
    for (index, c) in text.char_indices() {
        if c != separator {
            continue;
        }
        let piece = &text[start..index];
        let appended = !(omit_empty && piece.is_empty());
        if appended {
            result.push(piece);
        }
        start = index + c.len_utf8();
        if appended && Some(result.len()) == max_splits {
            break;
        }
    }
    if start != text.len() || !omit_empty {
        result.push(&text[start..]);
    }
    result
}

/// `CharacterSet.whitespaces`: the space separators (Unicode `Zs`) and the tab, without line breaks.
pub(crate) fn is_swift_whitespace(c: char) -> bool {
    matches!(
        c,
        '\u{9}' | '\u{20}' | '\u{A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}'
    )
}
