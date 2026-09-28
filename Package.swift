// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "CalendarConnectors",
    platforms: [.macOS(.v14)],
    products: [
        .library(name: "CalendarCore", targets: ["CalendarCore"]),
        .library(name: "CalendarOAuth", targets: ["CalendarOAuth"]),
        .library(name: "GoogleCalendar", targets: ["GoogleCalendar"]),
        .library(name: "MicrosoftCalendar", targets: ["MicrosoftCalendar"]),
        .library(name: "ICalendar", targets: ["ICalendar"]),
        .library(name: "CalDAVCalendar", targets: ["CalDAVCalendar"]),
        .library(name: "CalendarTestSupport", targets: ["CalendarTestSupport"]),
    ],
    targets: [
        .target(name: "CalendarCore"),
        .target(name: "CalendarOAuth", dependencies: ["CalendarCore"]),
        .target(name: "GoogleCalendar", dependencies: ["CalendarCore", "CalendarOAuth"]),
        .target(name: "MicrosoftCalendar", dependencies: ["CalendarCore", "CalendarOAuth"]),
        .target(name: "ICalendar", dependencies: ["CalendarCore"]),
        .target(name: "CalDAVCalendar", dependencies: ["CalendarCore", "ICalendar"]),
        .target(name: "CalendarTestSupport", dependencies: ["CalendarCore"]),
        .testTarget(name: "CalendarCoreTests", dependencies: ["CalendarCore", "CalendarTestSupport"]),
        .testTarget(name: "CalendarOAuthTests", dependencies: ["CalendarOAuth", "CalendarCore", "CalendarTestSupport"]),
        .testTarget(name: "GoogleCalendarTests", dependencies: ["GoogleCalendar", "CalendarOAuth", "CalendarCore", "CalendarTestSupport"]),
        .testTarget(name: "MicrosoftCalendarTests", dependencies: ["MicrosoftCalendar", "CalendarOAuth", "CalendarCore", "CalendarTestSupport"]),
        .testTarget(name: "ICalendarTests", dependencies: ["ICalendar", "CalendarCore", "CalendarTestSupport"]),
        .testTarget(name: "CalDAVCalendarTests", dependencies: ["CalDAVCalendar", "ICalendar", "CalendarCore", "CalendarTestSupport"]),
    ]
)
