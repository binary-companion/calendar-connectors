import Testing
@testable import CalendarCore

@Test func mailtoWithAQueryAndPercentEncodingIsDecoded() {
    #expect(CalendarUserAddress.email(from: "mailto:Bo@X.test?subject=hi") == "bo@x.test")
    #expect(CalendarUserAddress.email(from: "MAILTO:first%2Blast@x.test") == "first+last@x.test")
}

@Test func aBareAddressAndAnEmailUrnAreRead() {
    #expect(CalendarUserAddress.email(from: "  Bo@X.test ") == "bo@x.test")
    #expect(CalendarUserAddress.email(from: "urn:x-vendor:user:bo@x.test") == "bo@x.test")
}

@Test func addressesThatAreNotEmailsAreNil() {
    #expect(CalendarUserAddress.email(from: "urn:uuid:8f6b3a20-4c1e-4e46-8a3f-3d5f6f2f7a11") == nil)
    #expect(CalendarUserAddress.email(from: "https://caldav.icloud.com/12345/principal/") == nil)
    #expect(CalendarUserAddress.email(from: "/O=EXCH/OU=FIRST/CN=RECIPIENTS/CN=BO") == nil)
    #expect(CalendarUserAddress.email(from: "https://x.test") == nil)
    #expect(CalendarUserAddress.email(from: "mailto:") == nil)
    #expect(CalendarUserAddress.email(from: nil) == nil)
    #expect(CalendarUserAddress.email(from: "a@b@c.test") == nil)
}
