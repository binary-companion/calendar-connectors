@testable import CalendarCore
import Foundation

// zones/windows-names.json
//
// Entry points: WindowsTimeZones.timeZone(for: String) -> TimeZone?  and  WindowsTimeZones.windowsName(for: TimeZone) -> String?
//
//   fn "timeZone":    input { "fn", "name": "<text>" }      expected { "identifier": "<zone id>" | null }
//   fn "windowsName": input { "fn", "zone": "<IANA id>" }   expected { "name": "<Windows name>" | null }
//
// "timeZone" runs for every name of the table (WindowsTimeZones.table, internal, read with @testable), every IANA
// id the table maps to, the spelling aliases of the reverse table, and 24 unknown, padded or odd names.
// "windowsName" runs for every IANA id of the table and every alias, plus ids the table does not map.
// A zone the system rejects cannot be passed to windowsName, so such an id has only its "timeZone" case.

func exportZonesWindowsNames() -> [VectorCase] {
    var cases: [VectorCase] = []
    func timeZone(_ name: String) {
        cases.append(VectorCase(
            name: "windows-timezone-\(fourDigits(cases.count + 1))", input: ["fn": "timeZone", "name": name],
            expected: ["identifier": orNull(WindowsTimeZones.timeZone(for: name)?.identifier)]))
    }
    func windowsName(_ id: String) {
        guard let tz = TimeZone(identifier: id) else { return }
        cases.append(VectorCase(
            name: "windows-name-\(fourDigits(cases.count + 1))", input: ["fn": "windowsName", "zone": id],
            expected: ["name": orNull(WindowsTimeZones.windowsName(for: tz))]))
    }
    // The aliases of the reverse table are private; they are spelled here so the vectors cover them.
    let aliases = ["Asia/Kolkata", "Asia/Kathmandu", "Asia/Yangon", "Europe/Kyiv", "America/Nuuk", "America/Buenos_Aires",
                   "America/Indiana/Indianapolis", "America/Chihuahua", "UTC", "GMT", "Etc/GMT", "Etc/Universal", "Z"]
    let names = WindowsTimeZones.table.keys.sorted()
    let ids = Set(WindowsTimeZones.table.values).sorted()
    for name in names { timeZone(name) }
    for id in ids { timeZone(id) }
    for id in aliases { timeZone(id) }
    for name in ["Customized Time Zone", "", " ", "  Eastern Standard Time  ", "\tPacific Standard Time\n", "eastern standard time",
                 "EASTERN STANDARD TIME", "Eastern Standard Time (Mexico) ", "Nowhere Standard Time", "Mars/Olympus",
                 "(UTC-05:00) Eastern Time (US & Canada)", "Eastern", "Standard Time", "Etc/GMT+5", "Etc/GMT-14",
                 "America/New_York ", " Europe/Paris", "europe/paris", "Pacific Standard Time (Mexico)", "UTC+1",
                 "Romance Standard Time", "W. Europe Standard Time", "Tokyo Standard Time", "Singapore Standard Time"] { timeZone(name) }
    for id in ids { windowsName(id) }
    for id in aliases { windowsName(id) }
    for id in ["Pacific/Fakaofo", "Pacific/Midway", "America/Phoenix", "Europe/Dublin", "Africa/Lagos", "Asia/Kabul",
               "America/Santiago", "Australia/Eucla", "Pacific/Apia", "Asia/Tokyo", "Etc/GMT+5", "Antarctica/Troll"] { windowsName(id) }
    return cases
}
