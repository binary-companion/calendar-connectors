//! New in the Rust port: the tree `quick-xml` builds must equal the one Foundation's `XMLParser` builds with namespace
//! processing on. The expected values were read from Foundation (macOS) with a probe that mirrors Swift's `Builder`.

use crate::xml_tree::XmlTree;

fn parse(text: &str) -> XmlTree {
    XmlTree::parse(text.as_bytes()).unwrap()
}

#[test]
fn a_default_namespace_applies_to_unprefixed_elements_and_can_be_redeclared() {
    let tree = parse(r#"<a xmlns="DAV:"><b/><c xmlns="urn:x"><d/></c></a>"#);
    assert_eq!((tree.namespace.as_str(), tree.name.as_str()), ("DAV:", "a"));
    assert_eq!(tree.children[0].namespace, "DAV:");
    assert_eq!(tree.children[1].namespace, "urn:x");
    assert_eq!(tree.children[1].children[0].namespace, "urn:x");
}

#[test]
fn a_prefix_resolves_to_its_namespace_and_attributes_keep_their_qualified_names() {
    let tree =
        parse(r#"<d:a xmlns:d="DAV:" xmlns:c="urn:c" c:foo="1" d:bar="2" baz="3"><c:b/></d:a>"#);
    assert_eq!((tree.namespace.as_str(), tree.name.as_str()), ("DAV:", "a"));
    // Namespace declarations are not attributes; prefixed attribute names stay qualified.
    let keys: Vec<&str> = tree.attributes.keys().map(String::as_str).collect();
    assert_eq!(keys, ["baz", "c:foo", "d:bar"]);
    assert_eq!(tree.children[0].namespace, "urn:c");
    assert_eq!(tree.children[0].name, "b");
}

#[test]
fn a_redeclared_prefix_changes_meaning_only_inside_its_element() {
    let tree = parse(r#"<p:a xmlns:p="urn:one"><p:b xmlns:p="urn:two"><p:c/></p:b><p:d/></p:a>"#);
    assert_eq!(tree.namespace, "urn:one");
    assert_eq!(tree.children[0].namespace, "urn:two");
    assert_eq!(tree.children[0].children[0].namespace, "urn:two");
    assert_eq!(tree.children[1].namespace, "urn:one");
}

#[test]
fn text_joins_entities_cdata_and_character_references() {
    let tree = parse("<a>x &amp; y <![CDATA[<z> & ]]>&lt;w&gt; &#65;&#x42; &apos;&quot;</a>");
    assert_eq!(tree.text, "x & y <z> & <w> AB '\"");
}

#[test]
fn whitespace_between_children_is_kept_in_the_parents_text() {
    let tree = parse("<a>\n  <b/>  \n  <c> </c>\n</a>");
    assert_eq!(tree.text, "\n    \n  \n");
    assert_eq!(tree.children[1].text, " ");
}

#[test]
fn text_around_children_is_the_elements_own_text_only() {
    let tree = parse("<a>one<b>in</b>two<c/>three</a>");
    assert_eq!(tree.text, "onetwothree");
    assert_eq!(tree.children[0].text, "in");
}

#[test]
fn attribute_values_resolve_entities_and_normalise_whitespace() {
    let tree = parse("<a x=\"1 &amp; 2 &lt; &#65;\" y='a\nb\tc'/>");
    assert_eq!(tree.attributes["x"], "1 & 2 < A");
    assert_eq!(tree.attributes["y"], "a b c");
}

#[test]
fn line_ends_become_line_feeds() {
    assert_eq!(parse("<a>x\r\ny\rz</a>").text, "x\ny\nz");
}

#[test]
fn an_unbound_prefix_gives_no_namespace_and_the_local_name() {
    let tree = parse("<d:a/>");
    assert_eq!((tree.namespace.as_str(), tree.name.as_str()), ("", "a"));
    let tree = parse(r#"<a x:foo="1"/>"#);
    assert_eq!(tree.attributes["x:foo"], "1");
}

#[test]
fn comments_processing_instructions_and_a_byte_order_mark_are_ignored() {
    let tree = parse(
        "\u{FEFF}<?xml version=\"1.0\"?><!-- c --><?pi x?><a><!-- c2 -->t<?pi2 y?></a><!-- end -->",
    );
    assert_eq!(tree.text, "t");
}

#[test]
fn an_empty_namespace_declaration_unbinds_the_default() {
    let tree = parse(r#"<a xmlns="u"><b xmlns=""/></a>"#);
    assert_eq!(tree.children[0].namespace, "");
}

#[test]
fn what_foundation_refuses_is_refused() {
    for (label, text) in [
        ("duplicate attribute", "<a x=\"1\" x=\"2\"/>"),
        ("two roots", "<a/><b/>"),
        ("text after the root", "<a/>junk"),
        ("empty", ""),
        ("declaration only", "<?xml version=\"1.0\"?>"),
        ("undeclared entity", "<a>&nbsp;</a>"),
        ("unclosed", "<a><b>"),
        ("mismatched", "<a></b>"),
        ("less-than in an attribute", "<a x=\"<\"/>"),
        ("html", "<html><body>oops"),
        ("control character", "<a>x\u{1}y</a>"),
        ("nul", "<a>x\0y</a>"),
        ("character reference to a control character", "<a>&#1;</a>"),
    ] {
        assert!(XmlTree::parse(text.as_bytes()).is_err(), "{label}");
    }
    assert!(XmlTree::parse(b"<a>\xFF\xFE</a>").is_err(), "invalid UTF-8");
}

#[test]
fn declared_encodings_are_honoured() {
    let latin1 = [
        b"<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><a>caf".as_slice(),
        &[0xE9],
        b"</a>",
    ]
    .concat();
    assert_eq!(XmlTree::parse(&latin1).unwrap().text, "caf\u{e9}");
    let mut utf16 = vec![0xFF, 0xFE];
    for unit in "<a>hi</a>".encode_utf16() {
        utf16.extend(unit.to_le_bytes());
    }
    assert_eq!(XmlTree::parse(&utf16).unwrap().text, "hi");
    assert!(XmlTree::parse(b"<?xml version=\"1.0\" encoding=\"bogus\"?><a/>").is_err());
}

#[test]
fn declared_entities_are_never_expanded() {
    // Foundation delivers no text for a declared entity and refuses a nested (amplifying) declaration.
    let simple = parse("<!DOCTYPE a [<!ENTITY e \"hello\">]><a>&e;</a>");
    assert_eq!(simple.text, "");
    let external = parse("<!DOCTYPE a [<!ENTITY e SYSTEM \"file:///etc/hostname\">]><a>&e;</a>");
    assert_eq!(external.text, "");
    let bomb =
        "<!DOCTYPE a [<!ENTITY a0 \"aaaaaaaaaa\"><!ENTITY a1 \"&a0;&a0;&a0;&a0;\">]><a>&a1;</a>";
    assert!(XmlTree::parse(bomb.as_bytes()).is_err());
}

#[test]
fn nesting_is_bounded() {
    let deep = |n: usize| format!("{}{}", "<a>".repeat(n), "</a>".repeat(n));
    assert!(XmlTree::parse(deep(200).as_bytes()).is_ok());
    assert!(XmlTree::parse(deep(100_000).as_bytes()).is_err());
}

#[test]
fn lookups_find_children_and_descendants() {
    let tree = parse(r#"<a xmlns="u"><b><c/></b><b/></a>"#);
    assert_eq!(tree.children_named("u", "b").len(), 2);
    assert!(tree.child("u", "c").is_none());
    assert!(tree.first("u", "c").is_some());
    assert_eq!(tree.first("u", "a").unwrap().name, "a");
}
