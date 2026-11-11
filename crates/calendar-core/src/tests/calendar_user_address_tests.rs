use crate::calendar_user_address::CalendarUserAddress;

fn email(address: &str) -> Option<String> {
    CalendarUserAddress::email_from(Some(address))
}

#[test]
fn mailto_with_a_query_and_percent_encoding_is_decoded() {
    assert_eq!(
        email("mailto:Bo@X.test?subject=hi").as_deref(),
        Some("bo@x.test")
    );
    assert_eq!(
        email("MAILTO:first%2Blast@x.test").as_deref(),
        Some("first+last@x.test")
    );
}

#[test]
fn a_bare_address_and_an_email_urn_are_read() {
    assert_eq!(email("  Bo@X.test ").as_deref(), Some("bo@x.test"));
    assert_eq!(
        email("urn:x-vendor:user:bo@x.test").as_deref(),
        Some("bo@x.test")
    );
}

#[test]
fn addresses_that_are_not_emails_are_nil() {
    assert_eq!(email("urn:uuid:8f6b3a20-4c1e-4e46-8a3f-3d5f6f2f7a11"), None);
    assert_eq!(email("https://caldav.icloud.com/12345/principal/"), None);
    assert_eq!(email("/O=EXCH/OU=FIRST/CN=RECIPIENTS/CN=BO"), None);
    assert_eq!(email("https://x.test"), None);
    assert_eq!(email("mailto:"), None);
    assert_eq!(CalendarUserAddress::email_from(None), None);
    assert_eq!(email("a@b@c.test"), None);
}
