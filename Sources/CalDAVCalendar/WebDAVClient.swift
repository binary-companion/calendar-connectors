import CalendarCore
import Foundation

struct WebDAVCredentials: Sendable, Equatable {
    var username: String
    var password: String
}

struct WebDAVReply: Sendable {
    var response: HTTPResponse
    /// The URL that answered, after redirects: relative hrefs in the body resolve against it.
    var url: URL
}

/// HTTP for one CalDAV account. Adds Basic auth to every request, only over HTTPS (plain HTTP only to the loopback
/// host) and only to hosts inside `hostBase`, and follows redirects itself under the same rule (the transport must not
/// follow them: `URLSessionTransport(followsRedirects: false)`). The password never appears in an error.
struct WebDAVClient: Sendable {
    static let maxRedirects = 5
    private static let loopback: Set<String> = ["localhost", "127.0.0.1"]

    let transport: any HTTPTransport
    let hostBase: String
    /// Read per request, so a sign-in again (new secrets in the store) reaches a source that already exists.
    let credentials: @Sendable () async throws -> WebDAVCredentials

    init(transport: any HTTPTransport, hostBase: String, credentials: @escaping @Sendable () async throws -> WebDAVCredentials) {
        self.transport = transport
        self.hostBase = hostBase.lowercased()
        self.credentials = credentials
    }

    /// `https` to `hostBase` or a subdomain of it (matched at a label boundary); `http` only to the loopback host.
    static func isAllowed(_ url: URL, hostBase: String) -> Bool {
        guard let host = url.host?.lowercased(), let scheme = url.scheme?.lowercased() else { return false }
        let base = hostBase.lowercased()
        guard host == base || host.hasSuffix("." + base) else { return false }
        switch scheme {
        case "https": return true
        case "http": return loopback.contains(host)
        default: return false
        }
    }

    static func basicAuthorization(_ credentials: WebDAVCredentials) -> String {
        "Basic " + Data("\(credentials.username):\(credentials.password)".utf8).base64EncodedString()
    }

    /// An href from a response, resolved against the URL that sent it. One that points outside `hostBase` is refused.
    func resolve(_ href: String, against base: URL) throws -> URL {
        guard let url = URL(string: href, relativeTo: base)?.absoluteURL, Self.isAllowed(url, hostBase: hostBase) else {
            throw SourceError.invalidResponse("the server pointed to another host")
        }
        return url
    }

    func send(_ method: String, _ url: URL, headers: [String: String] = [:], body: Data? = nil) async throws -> WebDAVReply {
        var method = method, url = url, body = body, headers = headers
        for _ in 0...Self.maxRedirects {
            guard Self.isAllowed(url, hostBase: hostBase) else {
                throw SourceError.invalidResponse("refused to send credentials to \(url.scheme ?? "?")://\(url.host ?? "?")")
            }
            var request = HTTPRequest(url: url, method: method, headers: headers, body: body)
            request.headers["Authorization"] = Self.basicAuthorization(try await credentials())
            if body != nil, request.headers["Content-Type"] == nil { request.headers["Content-Type"] = "application/xml; charset=utf-8" }
            let response = try await transport.send(request)
            switch response.status {
            case 301, 302, 303, 307, 308:
                guard let location = response.header("Location"), let next = URL(string: location, relativeTo: url)?.absoluteURL else {
                    throw SourceError.invalidResponse("a redirect without a location")
                }
                guard Self.isAllowed(next, hostBase: hostBase) else {
                    throw SourceError.invalidResponse("the server redirected to another host")
                }
                if response.status == 303 {
                    method = "GET"
                    body = nil
                    headers["Content-Type"] = nil
                }
                url = next
            case 401:
                throw SourceError.authExpired
            case 429, 503:
                throw SourceError.rateLimited(retryAfter: response.header("Retry-After").flatMap(TimeInterval.init))
            case 500...599:
                throw SourceError.server(status: response.status)
            default:
                return WebDAVReply(response: response, url: url)
            }
        }
        throw SourceError.invalidResponse("too many redirects")
    }

    /// A `PROPFIND` that must answer 207; returns the multistatus and the URL that answered.
    func propfind(_ url: URL, depth: Int, _ properties: [DAVProperty]) async throws -> (Multistatus, URL) {
        let reply = try await send("PROPFIND", url, headers: ["Depth": String(depth)], body: DAVXML.propfind(properties))
        guard reply.response.status == 207 else {
            throw SourceError.invalidResponse("PROPFIND answered \(reply.response.status)")
        }
        return (try DAVXML.multistatus(reply.response.body), reply.url)
    }

    func report(_ url: URL, depth: Int, body: Data) async throws -> WebDAVReply {
        try await send("REPORT", url, headers: ["Depth": String(depth)], body: body)
    }
}
