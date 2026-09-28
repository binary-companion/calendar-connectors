import Foundation

/// RFC 5545 TEXT escaping (section 3.3.11).
public enum ICalText {
    public static func escape(_ text: String) -> String {
        var out = ""
        for character in text {
            switch character {
            case "\\": out += "\\\\"
            case ";": out += "\\;"
            case ",": out += "\\,"
            case "\n", "\r\n": out += "\\n"
            case "\r": continue
            default: out.append(character)
            }
        }
        return out
    }

    public static func unescape(_ text: String) -> String {
        var out = ""
        var escaping = false
        for character in text {
            if escaping {
                out += (character == "n" || character == "N") ? "\n" : String(character)
                escaping = false
            } else if character == "\\" {
                escaping = true
            } else {
                out.append(character)
            }
        }
        if escaping { out += "\\" }
        return out
    }
}
