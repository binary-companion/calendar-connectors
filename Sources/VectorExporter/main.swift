import Foundation

// The vector exporter: writes every golden vector file under the directory given as the first argument
// (default "vectors"). Run it through scripts/dev/export-vectors.sh. It is deterministic: two runs give the same bytes.

let outputDirectory = URL(fileURLWithPath: CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "vectors", isDirectory: true)

let exporters: [(path: String, run: () -> [VectorCase], minimum: Int)] = [
    ("zones/offsets.json", exportZonesOffsets, 2400),
    ("zones/wall-clock.json", exportZonesWallClock, 600),
    ("zones/all-day.json", exportZonesAllDay, 500),
    ("zones/known.json", exportZonesKnown, 1),
    ("zones/windows-names.json", exportZonesWindowsNames, 1),
    ("recurrence/rule-instances.json", exportRecurrenceRuleInstances, 2000),
    ("recurrence/set-occurrences.json", exportRecurrenceSetOccurrences, 300),
    ("recurrence/rule-parse.json", exportRecurrenceRuleParse, 400),
    ("ical/content-lines.json", exportICalContentLines, 300),
    ("ical/timezones.json", exportICalTimezones, 150),
    ("microsoft/graph-time.json", exportMicrosoftGraphTime, 300),
    ("core/conference.json", exportCoreConference, 400),
    ("core/url-accept.json", exportCoreUrlAccept, 300),
    ("oauth/pkce.json", exportOAuthPkce, 50),
]

for exporter in exporters {
    let cases = exporter.run()
    guard cases.count >= exporter.minimum else {
        FileHandle.standardError.write(Data("\(exporter.path): \(cases.count) cases, the plan needs at least \(exporter.minimum)\n".utf8))
        exit(1)
    }
    let file = VectorFile(path: exporter.path, area: String(exporter.path.dropLast(".json".count)), cases: cases)
    try writeVectorFile(file, to: outputDirectory)
    print("\(exporter.path): \(cases.count) cases")
}
