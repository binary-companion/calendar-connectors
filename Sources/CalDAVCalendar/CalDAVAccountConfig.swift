import CalendarCore
import Foundation

/// What sign-in learned about a CalDAV account, kept in `Connection.config` (never a password).
struct CalDAVAccountConfig: Sendable, Equatable {
    var serverURL: URL
    var username: String
    var principalURL: URL
    var homeURL: URL
    /// The principal's `calendar-user-address-set`, lowercased: how the account appears as organizer or attendee.
    var userAddresses: [String]
    /// The server schedules invitations itself (`calendar-auto-schedule`, RFC 6638).
    var autoSchedule: Bool

    init(serverURL: URL, username: String, principalURL: URL, homeURL: URL, userAddresses: [String], autoSchedule: Bool) {
        self.serverURL = serverURL
        self.username = username
        self.principalURL = principalURL
        self.homeURL = homeURL
        self.userAddresses = userAddresses
        self.autoSchedule = autoSchedule
    }

    init(config: [String: String]) throws {
        func url(_ key: String) throws -> URL {
            guard let text = config[key], let url = URL(string: text) else {
                throw SourceError.invalidResponse("the account settings are incomplete; sign in again")
            }
            return url
        }
        guard let username = config["username"] else { throw SourceError.invalidResponse("the account settings are incomplete; sign in again") }
        self.init(
            serverURL: try url("serverURL"), username: username, principalURL: try url("principalURL"), homeURL: try url("homeURL"),
            userAddresses: (config["userAddresses"] ?? "").split(separator: "\n").map(String.init),
            autoSchedule: config["autoSchedule"] == "true")
    }

    var config: [String: String] {
        [
            "serverURL": serverURL.absoluteString, "username": username, "principalURL": principalURL.absoluteString,
            "homeURL": homeURL.absoluteString, "userAddresses": userAddresses.joined(separator: "\n"),
            "autoSchedule": autoSchedule ? "true" : "false",
        ]
    }
}
