/// The sixty zones the zone vectors use: every zone named in the Swift tests of both libraries, the list of the
/// plan (Task 8), and sixteen more chosen for odd rules (half-hour and 45-minute offsets, southern-hemisphere DST,
/// a 2-hour DST step, no DST, Morocco-style inversions, the date-line zones).
let vectorZones: [String] = [
    // Named in the Swift tests (both libraries).
    "America/Chicago", "America/Denver", "America/Los_Angeles", "America/New_York", "America/Sao_Paulo",
    "Asia/Kolkata", "Asia/Tokyo", "Europe/Berlin", "Europe/Paris", "Pacific/Auckland", "Pacific/Fakaofo",
    "Pacific/Kiritimati", "Pacific/Midway", "UTC", "GMT",
    // The plan's list.
    "America/Phoenix", "America/Anchorage", "Pacific/Honolulu", "America/St_Johns", "America/Santiago",
    "America/Havana", "Europe/London", "Europe/Dublin", "Europe/Lisbon", "Europe/Athens", "Europe/Moscow",
    "Europe/Istanbul", "Africa/Cairo", "Africa/Casablanca", "Africa/Johannesburg", "Asia/Jerusalem", "Asia/Tehran",
    "Asia/Dubai", "Asia/Kathmandu", "Asia/Dhaka", "Asia/Bangkok", "Asia/Shanghai", "Asia/Seoul", "Australia/Perth",
    "Australia/Adelaide", "Australia/Sydney", "Australia/Lord_Howe", "Pacific/Chatham", "Pacific/Apia",
    // Sixteen more.
    "America/Halifax", "America/Mexico_City", "America/Bogota", "America/Asuncion", "Atlantic/Azores",
    "Atlantic/Reykjavik", "Africa/Lagos", "Africa/Nairobi", "Asia/Kabul", "Asia/Beirut", "Asia/Pyongyang",
    "Australia/Eucla", "Australia/Darwin", "Pacific/Norfolk", "Pacific/Marquesas", "Antarctica/Troll",
]

/// The zones the recurrence, iCalendar and Graph vectors draw from. Recurrence windows reach years ahead, so the
/// pool holds only zones whose rules are stable by law (no America/Havana, whose 2027 rule differs between zone
/// databases; Havana appears only in hand-placed cases that stop before 2026-11-01).
let rulesZones: [String] = [
    "America/New_York", "America/Los_Angeles", "America/Chicago", "Europe/London", "Europe/Berlin", "Asia/Tokyo",
    "Asia/Kolkata", "Australia/Sydney", "Australia/Lord_Howe", "America/Sao_Paulo", "Pacific/Auckland",
    "UTC",
]
