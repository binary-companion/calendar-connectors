import Foundation

public enum ICalError: Error, Equatable, Sendable {
    case malformed(String)
}

public enum ICalParser {
    /// Unfolds at the byte level first (a server may fold inside a UTF-8 sequence), then parses.
    public static func parse(_ data: Data) throws -> ICalComponent {
        var bytes: [UInt8] = []
        bytes.reserveCapacity(data.count)
        let input = [UInt8](data)
        var i = 0
        while i < input.count {
            // CRLF or LF followed by a space or tab is a fold: drop all three (or two).
            if input[i] == 0x0D, i + 2 < input.count, input[i + 1] == 0x0A, input[i + 2] == 0x20 || input[i + 2] == 0x09 { i += 3; continue }
            if input[i] == 0x0A, i + 1 < input.count, input[i + 1] == 0x20 || input[i + 1] == 0x09 { i += 2; continue }
            bytes.append(input[i])
            i += 1
        }
        return try parse(String(decoding: bytes, as: UTF8.self))
    }

    /// Accepts CRLF, LF or CR line ends, folded lines and a missing final line end.
    public static func parse(_ text: String) throws -> ICalComponent {
        var stack: [ICalComponent] = []
        var root: ICalComponent?
        for line in unfold(text) where !line.trimmingCharacters(in: .whitespaces).isEmpty {
            let property = try parseLine(line)
            switch property.name {
            case "BEGIN":
                guard root == nil else { throw ICalError.malformed("content after the top-level component") }
                stack.append(ICalComponent(name: property.value))
            case "END":
                guard let done = stack.popLast(), done.name == property.value.uppercased() else {
                    throw ICalError.malformed("unbalanced END:\(property.value)")
                }
                if stack.isEmpty { root = done } else { stack[stack.count - 1].components.append(done) }
            default:
                guard !stack.isEmpty else { throw ICalError.malformed("property outside a component: \(property.name)") }
                stack[stack.count - 1].properties.append(property)
            }
        }
        guard stack.isEmpty, let root else { throw ICalError.malformed("unterminated component") }
        return root
    }

    static func unfold(_ text: String) -> [String] {
        var lines: [String] = []
        for raw in text.split(omittingEmptySubsequences: false, whereSeparator: { $0 == "\n" || $0 == "\r\n" || $0 == "\r" }) {
            if let first = raw.first, first == " " || first == "\t", !lines.isEmpty {
                lines[lines.count - 1] += raw.dropFirst()
            } else {
                lines.append(String(raw))
            }
        }
        return lines
    }

    static func parseLine(_ line: String) throws -> ICalProperty {
        let chars = Array(line)
        var i = 0
        func fail() -> ICalError { .malformed("bad content line: \(line.prefix(60))") }
        var name = ""
        while i < chars.count, chars[i] != ";", chars[i] != ":" { name.append(chars[i]); i += 1 }
        guard !name.isEmpty, i < chars.count else { throw fail() }
        var parameters: [ICalParameter] = []
        while chars[i] == ";" {
            i += 1
            var parameterName = ""
            while i < chars.count, chars[i] != "=" { parameterName.append(chars[i]); i += 1 }
            guard i < chars.count, !parameterName.isEmpty else { throw fail() }
            i += 1
            var values: [String] = []
            while true {
                var value = ""
                if i < chars.count, chars[i] == "\"" {
                    i += 1
                    while i < chars.count, chars[i] != "\"" { value.append(chars[i]); i += 1 }
                    guard i < chars.count else { throw fail() }
                    i += 1
                } else {
                    while i < chars.count, chars[i] != ",", chars[i] != ";", chars[i] != ":" { value.append(chars[i]); i += 1 }
                }
                values.append(value)
                guard i < chars.count else { throw fail() }
                if chars[i] == "," { i += 1; continue }
                break
            }
            parameters.append(ICalParameter(name: parameterName, values: values))
        }
        guard chars[i] == ":" else { throw fail() }
        return ICalProperty(name: name, parameters: parameters, value: String(chars[(i + 1)...]))
    }
}

public enum ICalSerializer {
    /// CRLF line ends, lines folded at 75 octets without splitting a UTF-8 sequence.
    public static func serialize(_ component: ICalComponent) -> String {
        var out = ""
        write(component, into: &out)
        return out
    }

    /// One unfolded content line without its line end.
    public static func contentLine(_ property: ICalProperty) -> String {
        var line = property.name
        for parameter in property.parameters {
            line += ";" + parameter.name + "=" + parameter.values.map(quoted).joined(separator: ",")
        }
        return line + ":" + property.value
    }

    private static func write(_ component: ICalComponent, into out: inout String) {
        out += fold("BEGIN:" + component.name)
        for property in component.properties { out += fold(contentLine(property)) }
        for child in component.components { write(child, into: &out) }
        out += fold("END:" + component.name)
    }

    private static func quoted(_ value: String) -> String {
        value.contains(where: { $0 == ";" || $0 == ":" || $0 == "," }) ? "\"" + value + "\"" : value
    }

    private static func fold(_ line: String) -> String {
        var out = ""
        var octets = 0
        for scalar in line.unicodeScalars {
            let size = String(scalar).utf8.count
            if octets + size > 75 {
                out += "\r\n "
                octets = 1
            }
            out.unicodeScalars.append(scalar)
            octets += size
        }
        return out + "\r\n"
    }
}
