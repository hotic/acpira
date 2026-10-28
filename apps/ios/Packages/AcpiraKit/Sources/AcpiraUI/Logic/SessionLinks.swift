import Foundation

// shared/sessionLinks.ts: `acpira://session/<id>` and the shells' clickable forms (`vscode://hotic.acpira/session/<id>`
// …) name a session; the TypeScript, the Rust mirror and this one agree on every case of test/fixtures/session-links.json.

enum SessionLinks {
  static let prefix = "acpira://session/"
  private static let reserved: Set<String> = ["index", "prefs", "categories", "trash"]

  /// A session id names files under the store root: ASCII letters, digits, `_` and `-` only
  static func isSessionId(_ id: String) -> Bool {
    !id.isEmpty && !reserved.contains(id) && id.unicodeScalars.allSatisfy { $0.isASCII && (CharacterSet.alphanumerics.contains($0) || $0 == "_" || $0 == "-") }
  }

  static func link(_ id: String) -> String { prefix + id }

  /// Surrounding `<…>` or backticks are what models and chat clients wrap links in
  private static func unwrap(_ input: String) -> String {
    let s = input.trimmingCharacters(in: .whitespacesAndNewlines)
    if s.count >= 2, (s.hasPrefix("<") && s.hasSuffix(">")) || (s.hasPrefix("`") && s.hasSuffix("`")) {
      return String(s.dropFirst().dropLast()).trimmingCharacters(in: .whitespacesAndNewlines)
    }
    return s
  }

  /// The session a link points at; query, fragment and one trailing slash are ignored, and bare ids are no links
  static func id(ofLink href: String) -> String? {
    var s = unwrap(href)
    if let cut = s.firstIndex(where: { $0 == "?" || $0 == "#" }) { s = String(s[..<cut]) }
    if s.hasSuffix("/") { s.removeLast() }
    guard let m = s.wholeMatch(of: /([A-Za-z][A-Za-z0-9+.\-]*):\/\/(.*)/) else { return nil }
    let scheme = m.1.lowercased()
    let rest = String(m.2)
    let slash = rest.firstIndex(of: "/")
    let authority = slash.map { String(rest[..<$0]) } ?? rest
    let path = slash.map { String(rest[$0...]) } ?? ""
    if scheme == "acpira" && authority.lowercased() == "session" {
      let id = String(path.dropFirst())
      return isSessionId(id) ? id : nil
    }
    // `/session/` sits in the path, and "acpira" anywhere before it (scheme, authority or an earlier path segment)
    guard let at = path.range(of: "/session/", options: .backwards) else { return nil }
    let id = String(path[at.upperBound...])
    let before = "\(scheme)://\(authority)\(path[..<at.lowerBound])".lowercased()
    return isSessionId(id) && before.contains("acpira") ? id : nil
  }

  /// A session reference as a tool or the composer takes it: a link, else a bare id
  static func ref(_ input: String) -> String? {
    let s = unwrap(input)
    return id(ofLink: s) ?? (isSessionId(s) ? s : nil)
  }
}
