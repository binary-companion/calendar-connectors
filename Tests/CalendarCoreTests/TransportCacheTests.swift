import Foundation
import Testing
@testable import CalendarCore

@Suite struct TransportCacheTests {
    @Test func theAPISessionKeepsNoURLCache() {
        let configuration = URLSessionTransport.apiConfiguration
        #expect(configuration.urlCache == nil)
        #expect(configuration.requestCachePolicy == .reloadIgnoringLocalCacheData)
    }
}
