import CalendarCore
import Foundation

public enum AttendeeMapper {
    public static func read(_ vevent: ICalComponent, selfAddresses: Set<String>) -> (attendees: [Attendee], organizer: Attendee?) {
        let organizer = vevent.property("ORGANIZER").map { property -> Attendee in
            let email = email(of: property)
            return Attendee(name: property.parameter("CN"), email: email, role: .required, response: .accepted,
                            isSelf: isSelf(property.value, email: email, selfAddresses: selfAddresses), isOrganizer: true)
        }
        let attendees = vevent.properties(named: "ATTENDEE").map { property -> Attendee in
            let email = email(of: property)
            return Attendee(
                name: property.parameter("CN"), email: email, role: role(of: property), response: response(of: property),
                isSelf: isSelf(property.value, email: email, selfAddresses: selfAddresses),
                isOrganizer: email != nil && email == organizer?.email)
        }
        return (attendees, organizer)
    }

    /// The address as mail: from the value (`mailto:`), else from the `EMAIL` parameter (RFC 7986).
    static func email(of property: ICalProperty) -> String? {
        CalendarUserAddress.email(from: property.value) ?? CalendarUserAddress.email(from: property.parameter("EMAIL"))
    }

    public static func isSelf(_ address: String, email: String?, selfAddresses: Set<String>) -> Bool {
        if selfAddresses.contains(address.lowercased()) { return true }
        guard let email else { return false }
        return selfAddresses.contains("mailto:" + email)
    }

    static func role(of property: ICalProperty) -> AttendeeRole {
        switch property.parameter("CUTYPE")?.uppercased() {
        case "RESOURCE", "ROOM": return .resource
        default: break
        }
        switch property.parameter("ROLE")?.uppercased() {
        case "OPT-PARTICIPANT", "NON-PARTICIPANT": return .optional
        default: return .required
        }
    }

    static func response(of property: ICalProperty) -> ResponseStatus {
        switch property.parameter("PARTSTAT")?.uppercased() {
        case "ACCEPTED": return .accepted
        case "TENTATIVE": return .tentative
        case "DECLINED": return .declined
        default: return .needsAction
        }
    }

    static func partstat(_ response: ResponseStatus) -> String {
        switch response {
        case .accepted: return "ACCEPTED"
        case .tentative: return "TENTATIVE"
        case .declined: return "DECLINED"
        case .needsAction: return "NEEDS-ACTION"
        }
    }

    /// The same rule as the Google and Microsoft mappers: your attendee entry's response; an organizer who is you with
    /// no attendee entry is accepted; otherwise not invited. nil when the account's own addresses are unknown.
    public static func participation(attendees: [Attendee], organizer: Attendee?, knowsSelf: Bool) -> Participation? {
        guard knowsSelf else { return nil }
        if let me = attendees.first(where: \.isSelf) { return .invited(me.response) }
        if organizer?.isSelf == true { return .invited(.accepted) }
        return .notInvited
    }

    public static func property(for draft: AttendeeDraft) -> ICalProperty {
        var parameters: [ICalParameter] = []
        if let name = draft.name, !name.isEmpty { parameters.append(ICalParameter("CN", name)) }
        switch draft.role {
        case .required: parameters.append(ICalParameter("ROLE", "REQ-PARTICIPANT"))
        case .optional: parameters.append(ICalParameter("ROLE", "OPT-PARTICIPANT"))
        case .resource: parameters += [ICalParameter("ROLE", "REQ-PARTICIPANT"), ICalParameter("CUTYPE", "RESOURCE")]
        }
        parameters += [ICalParameter("PARTSTAT", "NEEDS-ACTION"), ICalParameter("RSVP", "TRUE")]
        return ICalProperty(name: "ATTENDEE", parameters: parameters, value: "mailto:" + draft.email)
    }

    public static func organizerProperty(address: String) -> ICalProperty {
        ICalProperty(name: "ORGANIZER", value: address)
    }

    /// Sets the account's own `PARTSTAT` and clears `RSVP` on each of its `ATTENDEE` entries. Returns false when the
    /// account is not an attendee.
    public static func setResponse(_ response: ResponseStatus, in vevent: inout ICalComponent, selfAddresses: Set<String>) -> Bool {
        var found = false
        for index in vevent.properties.indices where vevent.properties[index].name == "ATTENDEE" {
            let property = vevent.properties[index]
            guard isSelf(property.value, email: email(of: property), selfAddresses: selfAddresses) else { continue }
            vevent.properties[index].setParameter("PARTSTAT", partstat(response))
            vevent.properties[index].setParameter("RSVP", nil)
            found = true
        }
        return found
    }
}
