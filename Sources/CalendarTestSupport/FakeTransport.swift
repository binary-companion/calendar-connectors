import CalendarCore
import Foundation

/// Routes requests by URL substring. The MOST RECENTLY registered route whose text is contained in the request URL
/// answers (so register general routes first and specific ones after); it consumes its responses in order and
/// repeats the last one. Unmatched requests get 404.
public actor FakeTransport: HTTPTransport {
    private struct Route {
        let match: String
        var responses: [HTTPResponse]
    }
    private var routes: [Route] = []
    public private(set) var requests: [HTTPRequest] = []

    public init() {}

    public func route(_ match: String, _ responses: [HTTPResponse]) {
        routes.append(Route(match: match, responses: responses))
    }

    public func requests(matching text: String) -> [HTTPRequest] {
        requests.filter { $0.url.absoluteString.contains(text) }
    }

    public func send(_ request: HTTPRequest) async throws -> HTTPResponse {
        requests.append(request)
        let url = request.url.absoluteString
        for index in routes.indices.reversed() where url.contains(routes[index].match) {
            if routes[index].responses.count > 1 { return routes[index].responses.removeFirst() }
            return routes[index].responses[0]
        }
        return HTTPResponse(status: 404, body: Data("no route for \(url)".utf8))
    }
}

extension HTTPResponse {
    public static func json(_ object: Any, status: Int = 200, headers: [String: String] = [:]) -> HTTPResponse {
        let data = (try? JSONSerialization.data(withJSONObject: object)) ?? Data()
        return HTTPResponse(status: status, headers: headers, body: data)
    }
    public static func text(_ string: String, status: Int = 200) -> HTTPResponse {
        HTTPResponse(status: status, body: Data(string.utf8))
    }
}

/// A controllable clock for tests: `provider` is the `@Sendable () -> Date` the library takes.
public final class TestNow: @unchecked Sendable {
    private let lock = NSLock()
    private var value: Date
    public init(_ start: Date = Date(timeIntervalSince1970: 1_800_000_000)) { value = start }
    public var date: Date { lock.withLock { value } }
    public func advance(_ seconds: TimeInterval) { lock.withLock { value = value.addingTimeInterval(seconds) } }
    public var provider: @Sendable () -> Date { { [self] in date } }
}
