import CalendarCore
import CalendarTestSupport
import Foundation
import Testing
@testable import GoogleCalendar

/// Runs `body` and records an issue unless it throws exactly `expected`.
func expectWriteError(_ expected: WriteError, _ body: () async throws -> Void) async {
    do {
        try await body()
        Issue.record("expected \(expected), but nothing was thrown")
    } catch let error as WriteError {
        #expect(error == expected)
    } catch {
        Issue.record("expected \(expected), got \(error)")
    }
}

/// A minimal Google event resource; `extra` overrides or adds keys.
func googleEvent(id: String, etag: String = "e1", summary: String = "Standup", extra: [String: Any] = [:]) -> [String: Any] {
    var json: [String: Any] = [
        "id": id, "etag": etag, "summary": summary,
        "start": ["dateTime": "2026-09-21T10:00:00Z", "timeZone": "UTC"], "end": ["dateTime": "2026-09-21T10:30:00Z", "timeZone": "UTC"],
    ]
    for (key, value) in extra { json[key] = value }
    return json
}

func bodyJSON(_ request: HTTPRequest) -> [String: Any] {
    (try? JSONSerialization.jsonObject(with: request.body ?? Data())) as? [String: Any] ?? [:]
}

func googleError(_ reason: String, message: String = "m", status: Int, headers: [String: String] = [:]) -> HTTPResponse {
    .json(["error": ["errors": [["reason": reason]], "message": message]], status: status, headers: headers)
}
