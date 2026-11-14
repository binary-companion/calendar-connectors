//! Review Focus 2 of the phase 1 plan: a parser given garbage returns an error and never panics. 2,000 pseudo-random
//! byte strings and 200 truncations of a real sample go through `XmlTree::parse` and `DavXml::multistatus`; damaged
//! copies of the sample go through them too. (Not a port: Swift has no such test.)

use crate::dav_xml::DavXml;
use crate::tests::dav_xml_tests::HOME_MULTISTATUS;
use crate::xml_tree::XmlTree;

/// The generator the vector exporter uses (conventions section 9), so the cases are the same on every machine.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.0 >> 33
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }
}

const PIECES: &[&str] = &[
    "<",
    ">",
    "</",
    "/>",
    "<!--",
    "-->",
    "<![CDATA[",
    "]]>",
    "<?xml ",
    "?>",
    "<!DOCTYPE a [",
    "]>",
    "<!ENTITY e \"",
    "&",
    "&amp;",
    "&#",
    "&#x",
    ";",
    "\"",
    "'",
    "=",
    " ",
    "\r\n",
    "\n",
    "xmlns=\"DAV:\"",
    "xmlns:d=\"DAV:\"",
    "d:",
    "multistatus",
    "response",
    "href",
    "propstat",
    "prop",
    "status",
    "HTTP/1.1 200 OK",
    "\u{0301}",
    "é",
    "🙂",
    "\0",
    "\u{1}",
    "\u{FEFF}",
];

fn exercise(bytes: &[u8]) {
    let _ = XmlTree::parse(bytes);
    let _ = DavXml::multistatus(bytes);
}

#[test]
fn random_bytes_never_panic() {
    let mut rng = Lcg(0x1234_5678);
    for _ in 0..2000 {
        let length = rng.below(300);
        let bytes: Vec<u8> = (0..length).map(|_| rng.next() as u8).collect();
        exercise(&bytes);
    }
}

#[test]
fn random_markup_never_panics() {
    let mut rng = Lcg(0x9abc_def0);
    for _ in 0..2000 {
        let count = rng.below(40);
        let text: String = (0..count)
            .map(|_| PIECES[rng.below(PIECES.len())])
            .collect();
        exercise(text.as_bytes());
    }
}

#[test]
fn truncated_samples_never_panic_and_are_errors_when_cut_short() {
    let sample = HOME_MULTISTATUS.as_bytes();
    let step = (sample.len() / 200).max(1);
    let mut cuts = 0;
    for end in (0..sample.len()).step_by(step) {
        exercise(&sample[..end]);
        assert!(DavXml::multistatus(&sample[..end]).is_err(), "cut at {end}");
        cuts += 1;
    }
    assert!(cuts >= 190);
}

#[test]
fn damaged_samples_never_panic() {
    let mut rng = Lcg(42);
    for _ in 0..2000 {
        let mut bytes = HOME_MULTISTATUS.as_bytes().to_vec();
        for _ in 0..1 + rng.below(4) {
            let at = rng.below(bytes.len());
            match rng.below(3) {
                0 => bytes[at] = rng.next() as u8,
                1 => {
                    bytes.remove(at);
                }
                _ => bytes.insert(at, rng.next() as u8),
            }
        }
        exercise(&bytes);
    }
}

#[test]
fn hostile_nesting_and_size_are_refused_without_exhausting_memory() {
    let deep = format!("{}{}", "<a>".repeat(1_000_000), "</a>".repeat(1_000_000));
    assert!(XmlTree::parse(deep.as_bytes()).is_err());
    let wide_attributes = format!("<a {}/>", "x=\"1\" ".repeat(10));
    assert!(
        XmlTree::parse(wide_attributes.as_bytes()).is_err(),
        "duplicate attributes"
    );
}
