//! Port of `ContentLine.swift`: the content-line parser and serializer.

use crate::component::{ICalComponent, ICalParameter, ICalProperty};
use crate::grapheme::{characters, is, prefix};

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ICalError {
    #[error("malformed: {0}")]
    Malformed(String),
}

pub enum ICalParser {}

impl ICalParser {
    /// Unfolds at the byte level first (a server may fold inside a UTF-8 sequence), then parses. Bytes that are not
    /// UTF-8 become U+FFFD, as Swift's `String(decoding:as:)` does; there is no Latin-1 fallback.
    pub fn parse(data: &[u8]) -> Result<ICalComponent, ICalError> {
        let mut bytes: Vec<u8> = Vec::with_capacity(data.len());
        let mut i = 0;
        while i < data.len() {
            // CRLF or LF followed by a space or tab is a fold: drop all three (or two).
            if data[i] == 0x0D
                && i + 2 < data.len()
                && data[i + 1] == 0x0A
                && (data[i + 2] == 0x20 || data[i + 2] == 0x09)
            {
                i += 3;
                continue;
            }
            if data[i] == 0x0A && i + 1 < data.len() && (data[i + 1] == 0x20 || data[i + 1] == 0x09)
            {
                i += 2;
                continue;
            }
            bytes.push(data[i]);
            i += 1;
        }
        ICalParser::parse_text(&String::from_utf8_lossy(&bytes))
    }

    /// Accepts CRLF, LF or CR line ends, folded lines and a missing final line end. (Swift `parse(_ text: String)`.)
    pub fn parse_text(text: &str) -> Result<ICalComponent, ICalError> {
        let mut stack: Vec<ICalComponent> = Vec::new();
        let mut root: Option<ICalComponent> = None;
        for line in ICalParser::unfold(text) {
            if line.trim_matches(is_whitespace).is_empty() {
                continue;
            }
            let property = ICalParser::parse_line(&line)?;
            match property.name.as_str() {
                "BEGIN" => {
                    if root.is_some() {
                        return Err(ICalError::Malformed(
                            "content after the top-level component".to_string(),
                        ));
                    }
                    stack.push(ICalComponent::new(&property.value, Vec::new(), Vec::new()));
                }
                "END" => {
                    let done = match stack.pop() {
                        Some(done) if done.name == property.value.to_uppercase() => done,
                        _ => {
                            return Err(ICalError::Malformed(format!(
                                "unbalanced END:{}",
                                property.value
                            )));
                        }
                    };
                    match stack.last_mut() {
                        Some(parent) => parent.components.push(done),
                        None => root = Some(done),
                    }
                }
                _ => match stack.last_mut() {
                    Some(current) => current.properties.push(property),
                    None => {
                        return Err(ICalError::Malformed(format!(
                            "property outside a component: {}",
                            property.name
                        )));
                    }
                },
            }
        }
        match root {
            Some(root) if stack.is_empty() => Ok(root),
            _ => Err(ICalError::Malformed("unterminated component".to_string())),
        }
    }

    pub(crate) fn unfold(text: &str) -> Vec<String> {
        let mut lines: Vec<String> = Vec::new();
        let push_raw = |raw: &str, lines: &mut Vec<String>| match characters(raw).next() {
            Some(first) if (first == " " || first == "\t") && !lines.is_empty() => {
                let rest = &raw[first.len()..];
                if let Some(last) = lines.last_mut() {
                    last.push_str(rest);
                }
            }
            _ => lines.push(raw.to_string()),
        };
        // `\n`, `\r\n` and `\r` are always clusters of their own (a control character breaks on both sides), so the
        // lines can be cut on the scalars.
        let mut start = 0;
        let mut iter = text.char_indices().peekable();
        while let Some((index, c)) = iter.next() {
            if c == '\n' {
                push_raw(&text[start..index], &mut lines);
                start = index + 1;
            } else if c == '\r' {
                push_raw(&text[start..index], &mut lines);
                if let Some(&(next_index, '\n')) = iter.peek() {
                    iter.next();
                    start = next_index + 1;
                } else {
                    start = index + 1;
                }
            }
        }
        push_raw(&text[start..], &mut lines);
        lines
    }

    pub(crate) fn parse_line(line: &str) -> Result<ICalProperty, ICalError> {
        let chars: Vec<&str> = characters(line).collect();
        let fail = || ICalError::Malformed(format!("bad content line: {}", prefix(line, 60)));
        let at = |index: usize| chars.get(index).copied();
        let mut i = 0;
        let mut name = String::new();
        while let Some(c) = at(i) {
            if is(c, ';') || is(c, ':') {
                break;
            }
            name.push_str(c);
            i += 1;
        }
        if name.is_empty() || i >= chars.len() {
            return Err(fail());
        }
        let mut parameters: Vec<ICalParameter> = Vec::new();
        while at(i).is_some_and(|c| is(c, ';')) {
            i += 1;
            let mut parameter_name = String::new();
            while let Some(c) = at(i) {
                if is(c, '=') {
                    break;
                }
                parameter_name.push_str(c);
                i += 1;
            }
            if i >= chars.len() || parameter_name.is_empty() {
                return Err(fail());
            }
            i += 1;
            let mut values: Vec<String> = Vec::new();
            loop {
                let mut value = String::new();
                if at(i).is_some_and(|c| is(c, '"')) {
                    i += 1;
                    while let Some(c) = at(i) {
                        if is(c, '"') {
                            break;
                        }
                        value.push_str(c);
                        i += 1;
                    }
                    if i >= chars.len() {
                        return Err(fail());
                    }
                    i += 1;
                } else {
                    while let Some(c) = at(i) {
                        if is(c, ',') || is(c, ';') || is(c, ':') {
                            break;
                        }
                        value.push_str(c);
                        i += 1;
                    }
                }
                values.push(value);
                let Some(separator) = at(i) else {
                    return Err(fail());
                };
                if is(separator, ',') {
                    i += 1;
                    continue;
                }
                break;
            }
            parameters.push(ICalParameter::new(&parameter_name, values));
        }
        if !at(i).is_some_and(|c| is(c, ':')) {
            return Err(fail());
        }
        let value: String = chars[i + 1..].concat();
        Ok(ICalProperty::new(&name, parameters, value))
    }
}

/// `CharacterSet.whitespaces`: the Unicode category Zs and the tab.
pub(crate) fn is_whitespace(c: char) -> bool {
    matches!(
        c,
        '\t' | ' ' | '\u{A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}' | '\u{202F}' | '\u{205F}' | '\u{3000}'
    )
}

pub enum ICalSerializer {}

impl ICalSerializer {
    /// CRLF line ends, lines folded at 75 octets without splitting a UTF-8 sequence.
    pub fn serialize(component: &ICalComponent) -> String {
        let mut out = String::new();
        ICalSerializer::write(component, &mut out);
        out
    }

    /// One unfolded content line without its line end.
    pub fn content_line(property: &ICalProperty) -> String {
        let mut line = property.name.clone();
        for parameter in &property.parameters {
            line.push(';');
            line.push_str(&parameter.name);
            line.push('=');
            let values: Vec<String> = parameter
                .values
                .iter()
                .map(|v| ICalSerializer::quoted(v))
                .collect();
            line.push_str(&values.join(","));
        }
        line.push(':');
        line.push_str(&property.value);
        line
    }

    fn write(component: &ICalComponent, out: &mut String) {
        out.push_str(&ICalSerializer::fold(&format!("BEGIN:{}", component.name)));
        for property in &component.properties {
            out.push_str(&ICalSerializer::fold(&ICalSerializer::content_line(
                property,
            )));
        }
        for child in &component.components {
            ICalSerializer::write(child, out);
        }
        out.push_str(&ICalSerializer::fold(&format!("END:{}", component.name)));
    }

    fn quoted(value: &str) -> String {
        if characters(value).any(|c| is(c, ';') || is(c, ':') || is(c, ',')) {
            format!("\"{value}\"")
        } else {
            value.to_string()
        }
    }

    /// Folds at 75 octets, counting the UTF-8 length of each scalar; a continuation line starts with one space,
    /// which counts as one octet.
    fn fold(line: &str) -> String {
        let mut out = String::with_capacity(line.len() + line.len() / 70 * 3 + 2);
        let mut octets = 0;
        for c in line.chars() {
            let size = c.len_utf8();
            if octets + size > 75 {
                out.push_str("\r\n ");
                octets = 1;
            }
            out.push(c);
            octets += size;
        }
        out.push_str("\r\n");
        out
    }
}
