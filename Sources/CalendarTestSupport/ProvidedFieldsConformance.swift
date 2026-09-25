import CalendarCore
import Foundation

/// Every connector's mapper tests run their fixtures through this: a field a source lists in `providedFields` must
/// never be nil on its events or calendars. Fields that are not listed may be nil.
public enum ProvidedFieldsConformance {
    /// The declared event fields that are missing on `event`. An empty result means the event conforms.
    public static func violations(event: CalendarEvent, capabilities: SourceCapabilities) -> [String] {
        var found: [String] = []
        for field in capabilities.providedFields.sorted(by: { $0.rawValue < $1.rawValue }) {
            let missing: Bool
            switch field {
            case .kind: missing = event.kind == nil
            case .visibility: missing = event.visibility == nil
            case .availability: missing = event.availability == nil
            case .reminders: missing = event.reminders == nil
            case .series: missing = event.series == nil
            case .participation: missing = event.participation == nil
            case .version: missing = event.version == nil
            case .lastModified: missing = event.lastModified == nil
            case .created: missing = event.created == nil
            case .uidScope: missing = event.uidScope == nil
            case .structuredConference: missing = false   // a source without a conference has none to provide
            case .isDefault, .calendarTimeZone, .defaultReminders, .provider, .supportedAvailabilities, .permissionDetails:
                missing = false                           // calendar fields
            }
            if missing { found.append("declared field \(field.rawValue) is nil") }
        }
        return found
    }

    /// The declared calendar fields that are missing on `calendar`.
    public static func violations(calendar: CalendarDescriptor, capabilities: SourceCapabilities) -> [String] {
        var found: [String] = []
        for field in capabilities.providedFields.sorted(by: { $0.rawValue < $1.rawValue }) {
            let missing: Bool
            switch field {
            case .isDefault: missing = calendar.isDefault == nil
            case .calendarTimeZone: missing = calendar.timeZone == nil
            case .defaultReminders: missing = calendar.defaultReminders == nil
            case .provider: missing = calendar.provider == nil
            case .supportedAvailabilities: missing = calendar.supportedAvailabilities == nil
            case .permissionDetails: missing = calendar.permissions.canShare == nil || calendar.permissions.canViewPrivate == nil
            default: missing = false                      // event fields
            }
            if missing { found.append("declared field \(field.rawValue) is nil") }
        }
        return found
    }
}
