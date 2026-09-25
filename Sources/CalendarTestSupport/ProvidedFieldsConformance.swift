import CalendarCore
import Foundation

/// Every connector's mapper tests run their fixtures through this: a field a source lists in `providedFields` must
/// never be nil on its events. Fields that are not listed may be nil.
public enum ProvidedFieldsConformance {
    /// The declared fields that are missing on `event`. An empty result means the event conforms.
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
            case .structuredConference: missing = false   // a source without a conference has none to provide
            }
            if missing { found.append("declared field \(field.rawValue) is nil") }
        }
        return found
    }
}
