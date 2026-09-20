// swift-tools-version: 6.0
import PackageDescription

let package = Package(
    name: "CalendarConnectors",
    platforms: [.macOS(.v14)],
    products: [
        .library(name: "CalendarCore", targets: ["CalendarCore"]),
        .library(name: "GoogleCalendar", targets: ["GoogleCalendar"]),
    ],
    dependencies: [
        .package(url: "https://github.com/apple/swift-crypto.git", exact: "5.0.0"),
    ],
    targets: [
        .target(name: "CalendarCore", dependencies: [.product(name: "Crypto", package: "swift-crypto")]),
        .target(name: "GoogleCalendar", dependencies: ["CalendarCore"]),
        .target(name: "CalendarTestSupport", dependencies: ["CalendarCore"]),
        .testTarget(name: "CalendarCoreTests", dependencies: ["CalendarCore", "CalendarTestSupport"]),
        .testTarget(name: "GoogleCalendarTests", dependencies: ["GoogleCalendar", "CalendarCore", "CalendarTestSupport"]),
    ]
)
