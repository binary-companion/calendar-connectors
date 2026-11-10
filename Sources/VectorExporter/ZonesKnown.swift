import Foundation

// zones/known.json
//
// Entry point: TimeZone(identifier:) and its `identifier`
//
//   input:    { "identifier": "<text>" }
//   expected: { "accepted": <Bool>, "identifier": "<canonical identifier>" | null }
//
// Every id of TimeZone.knownTimeZoneIdentifiers (sorted), then 40 hand-written ids that are invalid, aliases,
// spelled in another case, or surrounded by space. "identifier" is what the Swift zone reports for itself.
// The first part of this file depends on the zone database of the macOS that ran the exporter.

func exportZonesKnown() -> [VectorCase] {
    var texts = TimeZone.knownTimeZoneIdentifiers.sorted()
    texts += [
        "US/Pacific", "US/Eastern", "US/Central", "US/Mountain", "GMT+5", "GMT-5", "GMT+05:30", "UTC", "UTC+1", "Z",
        "Etc/GMT-14", "Etc/GMT+12", "Etc/GMT+13", "Etc/UTC", "Etc/GMT0", "GMT0", "Universal", "Zulu", "EST", "PST8PDT",
        "america/new_york", "AMERICA/NEW_YORK", "utc", "gmt", "europe/london", "", " ", "America/New_York ", " Europe/Paris",
        "Mars/Olympus", "Nowhere/Special", "Asia/Calcutta", "Asia/Kolkata", "Asia/Katmandu", "Europe/Kiev", "Europe/Kyiv",
        "America//New_York", "../etc/localtime", "America/New York", "Pacific/Samoa",
    ]
    return texts.enumerated().map { index, text in
        let tz = TimeZone(identifier: text)
        return VectorCase(
            name: "known-\(fourDigits(index + 1))", input: ["identifier": text],
            expected: ["accepted": tz != nil, "identifier": orNull(tz?.identifier)])
    }
}
