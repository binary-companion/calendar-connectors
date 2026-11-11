import Foundation
for id in TimeZone.knownTimeZoneIdentifiers.sorted() {
  let z = TimeZone(identifier: id)!
  let a = Date(timeIntervalSince1970: 1768478400), b = Date(timeIntervalSince1970: 1784116800)
  print(id, z.abbreviation(for: a) ?? "nil", z.abbreviation(for: b) ?? "nil", z.isDaylightSavingTime(for: a), z.isDaylightSavingTime(for: b), z.secondsFromGMT(for: a), z.secondsFromGMT(for: b), separator: "\t")
}
