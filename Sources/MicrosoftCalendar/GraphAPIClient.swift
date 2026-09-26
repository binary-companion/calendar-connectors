import CalendarCore
import CalendarOAuth
import Foundation

/// Provider-specific outcomes that callers handle; never leaves the `MicrosoftCalendar` module.
enum GraphAPIError: Error, Equatable {
    case gone                 // 410, or a 400 that says the delta token is no longer valid
    case notFound             // 404
    case forbidden            // 403
    case conflict             // 409
    case preconditionFailed   // 412
    case badRequest(String)   // 400, with Graph's message

    /// What a public `CalendarSource` method throws if it cannot handle the case itself.
    var sourceError: SourceError { .invalidResponse("microsoft: \(self)") }
}

/// A page of a Graph collection: its items and the link to the next page.
protocol GraphPage: Decodable {
    var nextLink: String? { get }
}

struct GraphAPIClient: Sendable {
    static let base = "https://graph.microsoft.com/v1.0"
    static let host = "graph.microsoft.com"
    private static let maxThrottleRetries = 3
    /// Error codes Graph uses for a delta token it no longer accepts.
    private static let resyncCodes: Set<String> = ["syncStateNotFound", "ResyncRequired", "ErrorInvalidSyncStateData", "syncStateInvalid"]

    let transport: any HTTPTransport
    let tokens: AccessTokenProvider
    let sleep: Sleeper

    /// ASCII-only allow-list: `CharacterSet.alphanumerics` also admits non-ASCII letters, which are not valid in
    /// `URLComponents.percentEncodedPath`. Graph ids contain `=`, `-` and `_`.
    static func percentEncode(_ text: String) -> String {
        let allowed = CharacterSet(charactersIn: "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-._~")
        return text.addingPercentEncoding(withAllowedCharacters: allowed) ?? text
    }

    static func calendarPath(_ calendarID: String, _ tail: String = "") -> String {
        "/me/calendars/\(percentEncode(calendarID))\(tail)"
    }

    /// Events are addressed through their calendar, which also works for a shared calendar.
    static func eventPath(_ calendarID: String, _ eventID: String) -> String {
        calendarPath(calendarID, "/events/\(percentEncode(eventID))")
    }

    /// `path` must already be percent-encoded.
    func url(path: String, query: [URLQueryItem] = []) -> URL {
        var components = URLComponents(string: Self.base)!
        components.percentEncodedPath += path
        components.queryItems = query.isEmpty ? nil : query
        return components.url!
    }

    func get(url: URL, prefer: [String] = []) async throws -> Data {
        try await send(method: "GET", url: url, prefer: prefer)
    }

    /// One request with the shared 401-refresh, throttling backoff and 5xx handling. Every request asks for immutable
    /// ids (they survive a move between folders); `prefer` adds more preferences.
    func send(
        method: String, url: URL, body: Data? = nil, prefer: [String] = [], headers extraHeaders: [String: String] = [:]
    ) async throws -> Data {
        var retries = 0
        var refreshedAfter401 = false
        while true {
            try Task.checkCancellation()
            let token = try await tokens.accessToken()
            var headers = ["Authorization": "Bearer \(token)", "Accept": "application/json"]
            if body != nil { headers["Content-Type"] = "application/json" }
            headers["Prefer"] = (["IdType=\"ImmutableId\""] + prefer).joined(separator: ", ")
            for (name, value) in extraHeaders { headers[name] = value }
            let response = try await transport.send(HTTPRequest(url: url, method: method, headers: headers, body: body))
            switch response.status {
            case 200..<300:
                return response.body
            case 401:
                if refreshedAfter401 { throw SourceError.authExpired }
                refreshedAfter401 = true
                await tokens.invalidate()
            case 400:
                let (code, message) = Self.error(response)
                if let code, Self.resyncCodes.contains(code) { throw GraphAPIError.gone }
                throw GraphAPIError.badRequest(message)
            case 403: throw GraphAPIError.forbidden
            case 404: throw GraphAPIError.notFound
            case 409: throw GraphAPIError.conflict
            case 410: throw GraphAPIError.gone
            case 412: throw GraphAPIError.preconditionFailed
            case 429, 503, 504:
                let retryAfter = response.header("retry-after").flatMap(TimeInterval.init)
                retries += 1
                if retries > Self.maxThrottleRetries {
                    if response.status == 429 { throw SourceError.rateLimited(retryAfter: retryAfter) }
                    throw SourceError.server(status: response.status)
                }
                // Honor Retry-After exactly; otherwise exponential backoff with jitter.
                let fallback = Double(1 << (retries - 1)) * Double.random(in: 0.5...1.0)
                try await sleep(.seconds(retryAfter ?? fallback))
            case 500...:
                throw SourceError.server(status: response.status)
            default:
                throw SourceError.invalidResponse("HTTP \(response.status)")
            }
        }
    }

    private struct ErrorBody: Decodable {
        struct Inner: Decodable { let code: String?; let message: String? }
        let error: Inner?
    }

    private static func error(_ response: HTTPResponse) -> (code: String?, message: String) {
        let inner = (try? JSONDecoder().decode(ErrorBody.self, from: response.body))?.error
        return (inner?.code, inner?.message ?? "HTTP \(response.status)")
    }

    func decode<T: Decodable>(_ type: T.Type, from data: Data) throws -> T {
        do { return try JSONDecoder().decode(type, from: data) }
        catch { throw SourceError.invalidResponse("could not decode \(T.self)") }
    }

    /// Walks every page from `first`, calling `handle` for each, following `@odata.nextLink`. A next link must stay on
    /// Graph's own host: the bearer token is only ever sent there. No page cap; honors cancellation.
    func pages<Page: GraphPage>(
        _ type: Page.Type, from first: URL, prefer: [String] = [], handle: (Page) throws -> Void
    ) async throws {
        var next: URL? = first
        while let url = next {
            guard url.scheme == "https", url.host == Self.host else { throw SourceError.invalidResponse("unexpected link host") }
            let page = try decode(Page.self, from: try await get(url: url, prefer: prefer))
            try handle(page)
            next = page.nextLink.flatMap(URL.init(string:))
        }
    }
}
