import CalendarCore
import Foundation

/// Provider-specific outcomes that callers handle; never leaves the `GoogleCalendar` module.
enum GoogleAPIError: Error, Equatable {
    case gone       // 410: the sync token is no longer valid
    case notFound   // 404
    case forbidden  // 403 without a rate-limit reason

    /// What a public `CalendarSource` method throws if it cannot handle the case itself.
    var sourceError: SourceError { .invalidResponse("google: \(self)") }
}

struct GoogleAPIClient: Sendable {
    static let base = "https://www.googleapis.com/calendar/v3"
    private static let maxRateLimitRetries = 3

    let transport: any HTTPTransport
    let tokens: AccessTokenProvider
    let sleep: Sleeper

    /// `path` must already be percent-encoded, e.g. `/calendars/me%40x.com/events`.
    static func calendarPath(_ calendarID: String, _ tail: String) -> String {
        let allowed = CharacterSet.alphanumerics.union(CharacterSet(charactersIn: "-._~"))
        return "/calendars/\(calendarID.addingPercentEncoding(withAllowedCharacters: allowed) ?? calendarID)\(tail)"
    }

    func get(path: String, query: [URLQueryItem]) async throws -> Data {
        var rateLimitRetries = 0
        var refreshedAfter401 = false
        while true {
            try Task.checkCancellation()
            let token = try await tokens.accessToken()
            var components = URLComponents(string: Self.base)!
            components.percentEncodedPath += path
            components.queryItems = query
            let response = try await transport.send(HTTPRequest(
                url: components.url!, headers: ["Authorization": "Bearer \(token)", "Accept": "application/json"]))
            switch response.status {
            case 200..<300:
                return response.body
            case 401:
                if refreshedAfter401 { throw SourceError.authExpired }
                refreshedAfter401 = true
                await tokens.invalidate()
            case 410:
                throw GoogleAPIError.gone
            case 404:
                throw GoogleAPIError.notFound
            case 403, 429:
                guard Self.isRateLimit(response) else {
                    if response.status == 403 { throw Self.classifyForbidden(response) }
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
        struct Inner: Decodable { let errors: [Detail]? }
        let error: Inner?
    }

    /// A non-rate-limit 403. Only reason `forbidden` (one unreadable calendar) is skippable; the rest affect every calendar.
    private static func classifyForbidden(_ response: HTTPResponse) -> Error {
        let reason = (try? JSONDecoder().decode(ErrorBody.self, from: response.body))?.error?.errors?.first?.reason
        switch reason {
        case "forbidden": return GoogleAPIError.forbidden
        case "insufficientPermissions": return SourceError.authExpired
        default: return SourceError.invalidResponse("HTTP 403: \(reason ?? "unknown")")
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
                    URLQueryItem(name: "fields", value: "nextPageToken,items(id,summary,summaryOverride,backgroundColor,accessRole,primary,timeZone,hidden,deleted)"),
                ],
                next: { $0.nextPageToken }, handle: { entries += $0.items ?? [] })
        } catch let error as GoogleAPIError {
            throw error.sourceError
        }
        return entries
    }
}
