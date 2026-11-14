//! Port of `XMLTree.swift`: a namespace-resolved XML element, enough DOM for WebDAV multistatus bodies.
//!
//! Swift builds the tree from Foundation's `XMLParser` (namespace processing on, external entities off). Here the same
//! tree comes from `quick-xml`'s namespace-aware reader, with the rules Foundation applies (probed on macOS, pinned by
//! `tests/xml_tree_tests.rs`) written out: line ends and attribute whitespace are normalised, only the five predefined
//! entities and character references are resolved, a reference to an entity declared in the `DOCTYPE` yields no text
//! (and is never expanded, so there is nothing to amplify), characters outside the XML `Char` range are an error, and
//! any well-formedness error fails the whole parse. Documents in UTF-8, UTF-16 (with a byte order mark), ISO-8859-1,
//! US-ASCII, windows-1252 and ISO-8859-15 are read; any other declared encoding is an error (Foundation reads more).
//!
//! Foundation has no depth limit (tested to 100,000 levels). Here nesting deeper than [`XmlTree::MAX_DEPTH`] is an
//! error, so hostile input cannot build a tree whose drop or traversal recurses without bound; a multistatus is under
//! ten levels deep.

use std::collections::{BTreeMap, BTreeSet};

use calendar_core::failure::{Failure, Outcome};
use calendar_core::source_types::SourceError;
use quick_xml::events::Event;
use quick_xml::name::ResolveResult;
use quick_xml::reader::NsReader;

/// A namespace-resolved XML element.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub(crate) struct XmlTree {
    pub namespace: String,
    pub name: String,
    pub attributes: BTreeMap<String, String>,
    pub children: Vec<XmlTree>,
    /// The element's own character data (CDATA included), children's text excluded.
    pub text: String,
}

impl XmlTree {
    /// Deepest nesting accepted.
    pub(crate) const MAX_DEPTH: usize = 256;

    pub(crate) fn trimmed_text(&self) -> &str {
        self.text.trim()
    }

    pub(crate) fn child(&self, namespace: &str, name: &str) -> Option<&XmlTree> {
        self.children
            .iter()
            .find(|c| c.namespace == namespace && c.name == name)
    }

    pub(crate) fn children_named(&self, namespace: &str, name: &str) -> Vec<&XmlTree> {
        self.children
            .iter()
            .filter(|c| c.namespace == namespace && c.name == name)
            .collect()
    }

    /// Depth-first, `self` included.
    pub(crate) fn first(&self, namespace: &str, name: &str) -> Option<&XmlTree> {
        if self.namespace == namespace && self.name == name {
            return Some(self);
        }
        self.children.iter().find_map(|c| c.first(namespace, name))
    }

    pub(crate) fn parse(data: &[u8]) -> Outcome<XmlTree> {
        build(data).map_err(|()| {
            Failure::Source(SourceError::InvalidResponse(
                "the server sent XML that could not be read".to_string(),
            ))
        })
    }
}

// MARK: Input text

const WINDOWS_1252_HIGH: [char; 32] = [
    '\u{20AC}', '\u{81}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}',
    '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{8D}', '\u{017D}', '\u{8F}',
    '\u{90}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}',
    '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}', '\u{9D}', '\u{017E}', '\u{0178}',
];

fn declared_encoding(data: &[u8]) -> Option<String> {
    let head = &data[..data.len().min(200)];
    let head: String = head
        .iter()
        .map(|&b| if b < 0x80 { b as char } else { '?' })
        .collect();
    if !head.starts_with("<?xml") {
        return None;
    }
    let declaration = &head[..head.find("?>")?];
    let after = &declaration[declaration.find("encoding")? + "encoding".len()..];
    let after = after.trim_start().strip_prefix('=')?.trim_start();
    let quote = after.chars().next().filter(|c| *c == '"' || *c == '\'')?;
    let value = &after[1..];
    Some(value[..value.find(quote)?].to_lowercase())
}

fn decode(data: &[u8]) -> Result<String, ()> {
    if let Some(rest) = data.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8(rest.to_vec()).map_err(|_| ());
    }
    for (bom, little) in [([0xFF, 0xFE], true), ([0xFE, 0xFF], false)] {
        if let Some(rest) = data.strip_prefix(&bom) {
            if rest.len() % 2 != 0 {
                return Err(());
            }
            let units: Vec<u16> = rest
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| {
                    if little {
                        u16::from_le_bytes(*c)
                    } else {
                        u16::from_be_bytes(*c)
                    }
                })
                .collect();
            return String::from_utf16(&units).map_err(|_| ());
        }
    }
    match declared_encoding(data).as_deref() {
        None | Some("utf-8" | "utf8") => String::from_utf8(data.to_vec()).map_err(|_| ()),
        Some("us-ascii" | "ascii") => {
            if data.is_ascii() {
                Ok(data.iter().map(|&b| b as char).collect())
            } else {
                Err(())
            }
        }
        Some("iso-8859-1" | "iso_8859-1" | "latin1" | "l1") => {
            Ok(data.iter().map(|&b| b as char).collect())
        }
        Some("windows-1252" | "cp1252") => Ok(data
            .iter()
            .map(|&b| {
                if (0x80..0xA0).contains(&b) {
                    WINDOWS_1252_HIGH[usize::from(b - 0x80)]
                } else {
                    b as char
                }
            })
            .collect()),
        Some("iso-8859-15" | "latin9") => Ok(data
            .iter()
            .map(|&b| match b {
                0xA4 => '\u{20AC}',
                0xA6 => '\u{0160}',
                0xA8 => '\u{0161}',
                0xB4 => '\u{017D}',
                0xB8 => '\u{017E}',
                0xBC => '\u{0152}',
                0xBD => '\u{0153}',
                0xBE => '\u{0178}',
                other => other as char,
            })
            .collect()),
        Some(_) => Err(()),
    }
}

fn is_xml_char(c: char) -> bool {
    matches!(c, '\t' | '\n' | '\r' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..='\u{10FFFF}')
}

/// Decodes, checks every character and turns `\r\n` and `\r` into `\n` (XML 1.0 section 2.11).
fn prepare(data: &[u8]) -> Result<String, ()> {
    let text = decode(data)?;
    if !text.chars().all(is_xml_char) {
        return Err(());
    }
    if !text.contains('\r') {
        return Ok(text);
    }
    Ok(text.replace("\r\n", "\n").replace('\r', "\n"))
}

// MARK: References

/// Names of the general entities a `DOCTYPE` declares; `Err` when a declaration's value itself holds a reference to an
/// entity (the shape of an expansion bomb), which Foundation also refuses.
fn declared_entities(doctype: &str) -> Result<BTreeSet<String>, ()> {
    let mut names = BTreeSet::new();
    let mut rest = doctype;
    while let Some(at) = rest.find("<!ENTITY") {
        rest = &rest[at + "<!ENTITY".len()..];
        let declaration = rest.trim_start();
        if declaration.starts_with('%') {
            continue;
        }
        let end = declaration
            .find(|c: char| c.is_whitespace())
            .unwrap_or(declaration.len());
        names.insert(declaration[..end].to_string());
        let value = declaration[end..].trim_start();
        if let Some(quote) = value.chars().next().filter(|c| *c == '"' || *c == '\'') {
            let inner = &value[1..];
            let inner = &inner[..inner.find(quote).unwrap_or(inner.len())];
            let mut references = inner.split('&').skip(1);
            if references.any(|r| !r.starts_with('#')) {
                return Err(());
            }
        }
    }
    Ok(names)
}

fn resolve_reference(
    content: &str,
    declared: &BTreeSet<String>,
    out: &mut String,
) -> Result<(), ()> {
    match content {
        "amp" => out.push('&'),
        "lt" => out.push('<'),
        "gt" => out.push('>'),
        "quot" => out.push('"'),
        "apos" => out.push('\''),
        _ => {
            if let Some(number) = content.strip_prefix('#') {
                let value = match number.strip_prefix('x') {
                    Some(hex) if !hex.is_empty() && hex.bytes().all(|b| b.is_ascii_hexdigit()) => {
                        u32::from_str_radix(hex, 16).map_err(|_| ())?
                    }
                    None if !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()) => {
                        number.parse::<u32>().map_err(|_| ())?
                    }
                    _ => return Err(()),
                };
                let c = char::from_u32(value)
                    .filter(|c| is_xml_char(*c))
                    .ok_or(())?;
                out.push(c);
            } else if !declared.contains(content) {
                return Err(());
            }
        }
    }
    Ok(())
}

/// An attribute value as Foundation delivers it: whitespace characters become spaces, references are resolved, and a
/// literal `<` is an error.
fn attribute_value(raw: &str, declared: &BTreeSet<String>) -> Result<String, ()> {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        match c {
            '<' => return Err(()),
            '\t' | '\n' | '\r' => out.push(' '),
            '&' => {
                let rest = chars.as_str();
                let end = rest.find(';').ok_or(())?;
                resolve_reference(&rest[..end], declared, &mut out)?;
                chars = rest[end + 1..].chars();
            }
            other => out.push(other),
        }
    }
    Ok(out)
}

// MARK: The tree

fn is_xml_space(text: &str) -> bool {
    text.chars().all(|c| matches!(c, ' ' | '\t' | '\n' | '\r'))
}

fn element(
    start: &quick_xml::events::BytesStart<'_>,
    namespace: ResolveResult<'_>,
    declared: &BTreeSet<String>,
) -> Result<XmlTree, ()> {
    let element_namespace = match namespace {
        ResolveResult::Bound(ns) => ns.into_inner().to_string(),
        ResolveResult::Unbound | ResolveResult::Unknown(_) => String::new(),
    };
    let mut attributes = BTreeMap::new();
    for attribute in start.attributes() {
        let attribute = attribute.map_err(|_| ())?;
        let key = attribute.key.as_ref().to_string();
        if key == "xmlns" || key.starts_with("xmlns:") {
            continue;
        }
        attributes.insert(key, attribute_value(&attribute.value, declared)?);
    }
    let local = start.local_name().as_ref().to_string();
    Ok(XmlTree {
        namespace: element_namespace,
        name: local,
        attributes,
        children: Vec::new(),
        text: String::new(),
    })
}

/// Closes `done`: it becomes the last child of the open element, or the root.
fn attach(stack: &mut [XmlTree], root: &mut Option<XmlTree>, done: XmlTree) {
    match stack.last_mut() {
        Some(parent) => parent.children.push(done),
        None => *root = Some(done),
    }
}

fn build(data: &[u8]) -> Result<XmlTree, ()> {
    let text = prepare(data)?;
    let mut reader = NsReader::from_str(&text);
    let mut declared: BTreeSet<String> = BTreeSet::new();
    let mut stack: Vec<XmlTree> = Vec::new();
    let mut root: Option<XmlTree> = None;
    loop {
        let (namespace, event) = reader.read_resolved_event().map_err(|_| ())?;
        match event {
            Event::Start(start) => {
                if root.is_some() || stack.len() >= XmlTree::MAX_DEPTH {
                    return Err(());
                }
                stack.push(element(&start, namespace, &declared)?);
            }
            Event::Empty(start) => {
                if root.is_some() || stack.len() >= XmlTree::MAX_DEPTH {
                    return Err(());
                }
                let done = element(&start, namespace, &declared)?;
                attach(&mut stack, &mut root, done);
            }
            Event::End(_) => {
                let done = stack.pop().ok_or(())?;
                attach(&mut stack, &mut root, done);
            }
            Event::Text(content) => {
                let content: &str = &content;
                match stack.last_mut() {
                    Some(top) => top.text.push_str(content),
                    None if is_xml_space(content) => {}
                    None => return Err(()),
                }
            }
            Event::CData(content) => {
                let content: &str = &content;
                stack.last_mut().ok_or(())?.text.push_str(content);
            }
            Event::GeneralRef(reference) => {
                let top = stack.last_mut().ok_or(())?;
                resolve_reference(&reference, &declared, &mut top.text)?;
            }
            Event::DocType(content) => {
                if root.is_some() || !stack.is_empty() {
                    return Err(());
                }
                let content: &str = &content;
                declared.extend(declared_entities(content)?);
            }
            Event::Comment(_) | Event::Decl(_) | Event::PI(_) => {}
            Event::Eof => break,
        }
    }
    if !stack.is_empty() {
        return Err(());
    }
    root.ok_or(())
}
