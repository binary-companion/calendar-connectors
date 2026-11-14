//! Not ported from Swift: the `URL` accessors that `UrlString` gained for the model (path, query, user info),
//! pinned here because the conference detector and the join-link policy depend on them.

use crate::UrlString;

fn url(text: &str) -> UrlString {
    UrlString::parse(text).unwrap()
}

#[test]
fn path_is_percent_decoded_and_empty_when_absent() {
    assert_eq!(url("https://example.test/a%20b/c").path(), "/a b/c");
    assert_eq!(url("https://example.test").path(), "");
    assert_eq!(url("https://example.test?q=1").path(), "");
    assert_eq!(url("zoommtg://zoom.us/join?confno=1#frag").path(), "/join");
    assert_eq!(url("mailto:a@example.test").path(), "a@example.test");
}

#[test]
fn query_is_kept_as_written_and_items_are_decoded() {
    let link = url("https://example.test/p?a=1&b=x%20y&flag&a=2#top");
    assert_eq!(link.query().as_deref(), Some("a=1&b=x%20y&flag&a=2"));
    assert_eq!(
        link.query_items(),
        vec![
            ("a".to_string(), Some("1".to_string())),
            ("b".to_string(), Some("x y".to_string())),
            ("flag".to_string(), None),
            ("a".to_string(), Some("2".to_string())),
        ]
    );
    assert_eq!(link.query_value("a").as_deref(), Some("1"));
    assert_eq!(link.query_value("flag"), None);
    assert_eq!(link.query_value("missing"), None);
    assert_eq!(url("https://example.test/p").query(), None);
    assert!(url("https://example.test/p").query_items().is_empty());
}

#[test]
fn user_and_password_come_from_the_user_info() {
    let link = url("https://bo%40x:secret@example.test/");
    assert_eq!(link.user().as_deref(), Some("bo@x"));
    assert_eq!(link.password().as_deref(), Some("secret"));
    assert_eq!(link.host().as_deref(), Some("example.test"));
    let user_only = url("https://bo@example.test/");
    assert_eq!(user_only.user().as_deref(), Some("bo"));
    assert_eq!(user_only.password(), None);
    assert_eq!(url("https://example.test/").user(), None);
}

#[test]
fn malformed_escapes_and_non_utf8_decodings_keep_the_text() {
    assert_eq!(url("https://example.test/100%").path(), "/100%");
    assert_eq!(url("https://example.test/%zz").path(), "/%zz");
    assert_eq!(url("https://example.test/%ff").path(), "/%ff");
}
