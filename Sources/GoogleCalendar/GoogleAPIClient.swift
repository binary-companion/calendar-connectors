import CalendarCore
import CalendarOAuth
import Foundation

/// Provider-specific outcomes that callers handle; never leaves the `GoogleCalendar` module.
enum GoogleAPIError: Error, Equatable {
    case gone       // 410: the sync token is no longer valid (or the event was already deleted)
    case notFound   // 404
    case forbidden  // 403 without a rate-limit reason
    case preconditionFailed   // 412 (write mode only): the `If-Match` version is stale
    case badRequest(String)   // 400 (write mode only), with Google's message
    case conflict             // 409 (write mode only): e.g. an insert with a client-chosen id that already exists

    /// What a public `CalendarSource` method throws if it cannot handle the case itself.
    var sourceError: SourceError { .invalidResponse("google: \(self)") }
}

/// Reads keep the original error mapping. Writes additionally map a 400 to `badRequest`, a 409 to `conflict` and a 412 to
/// `preconditionFailed`, and treat every non-rate-limit 403 (other than `insufficientPermissions`) as `forbidden`.
enum GoogleRequestMode: Sendable { case read, write }

struct GoogleAPIClient: Sendable {
    static let base = "https://www.googleapis.com/calendar/v3"
    private static let maxRateLimitRetries = 3

    let transport: any HTTPTransport
    let tokens: AccessTokenProvider
    let sleep: Sleeper

    /// ASCII-only allow-list: `CharacterSet.alphanumerics` also admits non-ASCII letters, which are not valid in
    /// `URLComponents.percentEncodedPath` (setting one traps).
    static func percentEncode(_ text: String) -> String {
        let allowed = CharacterSet(charactersIn: "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-._~")
        return text.addingPercentEncoding(withAllowedCharacters: allowed) ?? text
    }

    /// `path` must already be percent-encoded, e.g. `/calendars/me%40x.com/events`.
    static func calendarPath(_ calendarID: String, _ tail: String) -> String {
        "/calendars/\(percentEncode(calendarID))\(tail)"
    }

    static func eventPath(_ calendarID: String, _ eventID: String) -> String {
        calendarPath(calendarID, "/events/\(percentEncode(eventID))")
    }

    func get(path: String, query: [URLQueryItem]) async throws -> Data {
        try await send(method: "GET", path: path, query: query)
    }

    /// One request with the shared 401-refresh, rate-limit backoff and 5xx handling. `path` is percent-encoded.
    func send(
        method: String, path: String, query: [URLQueryItem] = [], body: Data? = nil,
        headers extraHeaders: [String: String] = [:], mode: GoogleRequestMode = .read
    ) async throws -> Data {
        var rateLimitRetries = 0
        var refreshedAfter401 = false
        while true {
            try Task.checkCancellation()
            let token = try await tokens.accessToken()
            var components = URLComponents(string: Self.base)!
            components.percentEncodedPath += path
            components.queryItems = query.isEmpty ? nil : query
            var headers = ["Authorization": "Bearer \(token)", "Accept": "application/json"]
            if body != nil { headers["Content-Type"] = "application/json" }
            for (name, value) in extraHeaders { headers[name] = value }
            let response = try await transport.send(HTTPRequest(url: components.url!, method: method, headers: headers, body: body))
            switch response.status {
            case 200..<300:
                return response.body
            case 401:
                if refreshedAfter401 { throw SourceError.authExpired }
                refreshedAfter401 = true
                await tokens.invalidate()
            case 400 where mode == .write:
                throw GoogleAPIError.badRequest(Self.message(response))
            case 410:
                throw GoogleAPIError.gone
            case 404:
                throw GoogleAPIError.notFound
            case 409 where mode == .write:
                throw GoogleAPIError.conflict
            case 412 where mode == .write:
                throw GoogleAPIError.preconditionFailed
            case 403, 429:
                guard Self.isRateLimit(response) else {
                    if response.status == 403 { throw Self.classifyForbidden(response, mode: mode) }
                    throw SourceError.invalidResponse("HTTP \(response.status)")
                }
                let retryAfter = response.header("retry-after").flatMap(TimeInterval.init)
                rateLimitRetries += 1
                if rateLimitRetries > Self.maxRateLimitRetries { throw SourceError.rateLimited(retryAfter: retryAfter) }
                // Honor Retry-After exactly; otherwise exponential backoff with jitter so several clients de-synchronize.
                let fallback = Double(1 << (rateLimitRetries - 1)) * Double.random(in: 0.5...1.0)
                try await sleep(.seconds(retryAfter ?? fallback))
            case 500...:
                throw SourceError.server(status: response.status)
            default:
                throw SourceError.invalidResponse("HTTP \(response.status)")
            }
        }
    }

    private struct ErrorBody: Decodable {
        struct Detail: Decodable { let reason: String? }
        struct Inner: Decodable {
            let errors: [Detail]?
            let message: String?
        }
        let error: Inner?
    }

    private static func message(_ response: HTTPResponse) -> String {
        (try? JSONDecoder().decode(ErrorBody.self, from: response.body))?.error?.message ?? "HTTP \(response.status)"
    }

    private static let quotaReasons: Set<String> = ["quotaExceeded", "calendarUsageLimitsExceeded", "dailyLimitExceeded"]

    /// A non-rate-limit 403. Reads: only reason `forbidden` (one unreadable calendar) is skippable and the rest affect
    /// every calendar. Writes: a quota reason is a rate limit (retry later, not a permission problem); any other reason
    /// but `insufficientPermissions` is a permission problem on this event.
    private static func classifyForbidden(_ response: HTTPResponse, mode: GoogleRequestMode) -> Error {
        let errors = (try? JSONDecoder().decode(ErrorBody.self, from: response.body))?.error?.errors ?? []
        // Only the write-mode quota check looks at every error; reads judge the first error's own reason, as they always did.
        if mode == .write, errors.contains(where: { $0.reason.map(quotaReasons.contains) ?? false }) {
            return SourceError.rateLimited(retryAfter: response.header("retry-after").flatMap(TimeInterval.init))
        }
        let firstReason = errors.first?.reason
        switch firstReason {
        case "forbidden": return GoogleAPIError.forbidden
        case "insufficientPermissions": return SourceError.authExpired
        default:
            if mode == .write { return GoogleAPIError.forbidden }
            return SourceError.invalidResponse("HTTP 403: \(firstReason ?? "unknown")")
        }
    }

    private static func isRateLimit(_ response: HTTPResponse) -> Bool {
        if response.status == 429 { return true }
        let body = String(decoding: response.body, as: UTF8.self)
        return body.contains("rateLimitExceeded") || body.contains("userRateLimitExceeded")
    }

    func decode<T: Decodable>(_ type: T.Type, from data: Data) throws -> T {
        do { return try JSONDecoder().decode(type, from: data) }
        catch { throw SourceError.invalidResponse("could not decode \(T.self)") }
    }

    /// Walks every page, calling `handle` for each, until `next` returns nil. No page cap; honors cancellation.
    func pages<Page: Decodable>(
        _ type: Page.Type, path: String, query: [URLQueryItem],
        next: (Page) -> String?, handle: (Page) throws -> Void
    ) async throws {
        var pageToken: String?
        repeat {
            var pageQuery = query
            if let pageToken { pageQuery.append(URLQueryItem(name: "pageToken", value: pageToken)) }
            let page = try decode(Page.self, from: try await get(path: path, query: pageQuery))
            try handle(page)
            pageToken = next(page)
        } while pageToken != nil
    }

    func calendarList() async throws -> [GoogleCalendarListEntryDTO] {
        var entries: [GoogleCalendarListEntryDTO] = []
        do {
            try await pages(
                GoogleCalendarListPageDTO.self, path: "/users/me/calendarList",
                query: [
                    URLQueryItem(name: "showHidden", value: "false"),
                    URLQueryItem(name: "minAccessRole", value: "freeBusyReader"),
                    URLQueryItem(name: "maxResults", value: "250"),
                    URLQueryItem(name: "fields", value: "nextPageToken,items(id,summary,summaryOverride,backgroundColor,accessRole,primary,timeZone,hidden,deleted,defaultReminders(minutes))"),
                ],
                next: { $0.nextPageToken }, handle: { entries += $0.items ?? [] })
        } catch let error as GoogleAPIError {
            throw error.sourceError
        }
        return entries
    }
}
