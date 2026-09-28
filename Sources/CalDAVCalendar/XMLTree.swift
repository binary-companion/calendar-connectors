import CalendarCore
import Foundation
#if canImport(FoundationXML)
import FoundationXML
#endif

/// A namespace-resolved XML element: enough DOM for WebDAV multistatus bodies.
struct XMLTree: Sendable, Equatable {
    var namespace: String
    var name: String
    var attributes: [String: String] = [:]
    var children: [XMLTree] = []
    /// The element's own character data (CDATA included), children's text excluded.
    var text: String = ""

    var trimmedText: String { text.trimmingCharacters(in: .whitespacesAndNewlines) }

    func child(_ namespace: String, _ name: String) -> XMLTree? {
        children.first { $0.namespace == namespace && $0.name == name }
    }

    func children(_ namespace: String, _ name: String) -> [XMLTree] {
        children.filter { $0.namespace == namespace && $0.name == name }
    }

    /// Depth-first, `self` included.
    func first(_ namespace: String, _ name: String) -> XMLTree? {
        if self.namespace == namespace && self.name == name { return self }
        for child in children { if let found = child.first(namespace, name) { return found } }
        return nil
    }

    static func parse(_ data: Data) throws -> XMLTree {
        let builder = Builder()
        let parser = XMLParser(data: data)
        parser.shouldProcessNamespaces = true
        parser.shouldResolveExternalEntities = false
        parser.delegate = builder
        guard parser.parse(), let root = builder.root else {
            throw SourceError.invalidResponse("the server sent XML that could not be read")
        }
        return root
    }

    private final class Builder: NSObject, XMLParserDelegate {
        var stack: [XMLTree] = []
        var root: XMLTree?

        func parser(_ parser: XMLParser, didStartElement elementName: String, namespaceURI: String?, qualifiedName: String?,
                    attributes: [String: String] = [:]) {
            stack.append(XMLTree(namespace: namespaceURI ?? "", name: elementName, attributes: attributes))
        }

        func parser(_ parser: XMLParser, foundCharacters string: String) {
            if !stack.isEmpty { stack[stack.count - 1].text += string }
        }

        func parser(_ parser: XMLParser, foundCDATA CDATABlock: Data) {
            if !stack.isEmpty { stack[stack.count - 1].text += String(decoding: CDATABlock, as: UTF8.self) }
        }

        func parser(_ parser: XMLParser, didEndElement elementName: String, namespaceURI: String?, qualifiedName: String?) {
            guard let done = stack.popLast() else { return }
            if stack.isEmpty { root = done } else { stack[stack.count - 1].children.append(done) }
        }
    }
}
