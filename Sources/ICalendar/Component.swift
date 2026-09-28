import Foundation

public struct ICalParameter: Hashable, Sendable {
    public var name: String
    /// Unquoted values; the serializer quotes a value that needs it.
    public var values: [String]
    public init(name: String, values: [String]) {
        self.name = name.uppercased()
        self.values = values
    }
    public init(_ name: String, _ value: String) { self.init(name: name, values: [value]) }
}

/// One content line. `value` is the text after the colon exactly as it appears on the wire (TEXT escapes kept), so a
/// property the library does not model is written back unchanged.
public struct ICalProperty: Hashable, Sendable {
    public var name: String
    public var parameters: [ICalParameter]
    public var value: String

    public init(name: String, parameters: [ICalParameter] = [], value: String) {
        self.name = name.uppercased()
        self.parameters = parameters
        self.value = value
    }

    /// A TEXT property: `text` is escaped for the wire.
    public init(name: String, text: String, parameters: [ICalParameter] = []) {
        self.init(name: name, parameters: parameters, value: ICalText.escape(text))
    }

    /// The value read as TEXT.
    public var text: String { ICalText.unescape(value) }

    /// The first value of the parameter, or nil.
    public func parameter(_ name: String) -> String? {
        let key = name.uppercased()
        return parameters.first { $0.name == key }?.values.first
    }

    /// Sets the parameter to one value, or removes it for nil.
    public mutating func setParameter(_ name: String, _ value: String?) {
        let key = name.uppercased()
        guard let value else { parameters.removeAll { $0.name == key }; return }
        if let index = parameters.firstIndex(where: { $0.name == key }) {
            parameters[index].values = [value]
        } else {
            parameters.append(ICalParameter(key, value))
        }
    }
}

/// A component (`VCALENDAR`, `VEVENT`, `VALARM`, `VTIMEZONE`, ...) with its properties and children, in file order.
public struct ICalComponent: Hashable, Sendable {
    public var name: String
    public var properties: [ICalProperty]
    public var components: [ICalComponent]

    public init(name: String, properties: [ICalProperty] = [], components: [ICalComponent] = []) {
        self.name = name.uppercased()
        self.properties = properties
        self.components = components
    }

    public func property(_ name: String) -> ICalProperty? {
        let key = name.uppercased()
        return properties.first { $0.name == key }
    }

    public func properties(named name: String) -> [ICalProperty] {
        let key = name.uppercased()
        return properties.filter { $0.name == key }
    }

    public func components(named name: String) -> [ICalComponent] {
        let key = name.uppercased()
        return components.filter { $0.name == key }
    }

    /// Replaces every property with this name by `property`, at the position of the first one (or appends it).
    public mutating func set(_ property: ICalProperty) {
        if let first = properties.firstIndex(where: { $0.name == property.name }) {
            properties[first] = property
            var index = properties.index(after: first)
            while index < properties.endIndex {
                if properties[index].name == property.name { properties.remove(at: index) } else { index = properties.index(after: index) }
            }
        } else {
            properties.append(property)
        }
    }

    /// Sets a TEXT property, or removes it for nil or empty text.
    public mutating func setText(_ name: String, _ text: String?) {
        guard let text, !text.isEmpty else { removeProperties(named: name); return }
        set(ICalProperty(name: name, text: text))
    }

    public mutating func removeProperties(named name: String) {
        let key = name.uppercased()
        properties.removeAll { $0.name == key }
    }

    public mutating func append(_ property: ICalProperty) { properties.append(property) }
}
