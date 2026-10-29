import AcpiraModels
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

  /// Session tools carry a canonical link followed by a turn range or search query.
  static func splitTarget(_ target: String?) -> (id: String?, rest: String?) {
    let text = (target ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
    let space = text.firstIndex(of: " ")
    let head = space.map { String(text[..<$0]) } ?? text
    let id = id(ofLink: head)
    let rest = (id == nil ? text : space.map { String(text[text.index(after: $0)...]) } ?? "")
      .trimmingCharacters(in: .whitespacesAndNewlines)
    return (id, rest.isEmpty ? nil : rest)
  }

  static func isTool(_ block: ToolCallBlock) -> Bool {
    block.verbKey == "verb.readSession" || block.verbKey == "verb.searchSessions"
  }

  /// Local ids win over native aliases; an unknown session keeps its id rather than its URI.
  static func title(_ id: String, in summaries: [SessionSummary]) -> String {
    guard let session = summaries.first(where: { $0.id == id }) ?? summaries.first(where: { $0.acpSessionId == id }) else {
      return id
    }
    return session.title.isEmpty ? t("session.fallbackTitle") : session.title
  }

  /// Match folding.ts sessionReadLabel without allocating a long list of turns.
  static func readLabel(_ range: String?) -> String {
    guard let match = (range ?? "").wholeMatch(of: /(\d*)-(\d*)/),
          !match.1.isEmpty || !match.2.isEmpty,
          let from = match.1.isEmpty ? 1 : Int(match.1) else { return t("tool.sessionOutline") }
    if match.2.isEmpty { return t("tool.sessionTurnsFrom", ["n": "\(from)"]) }
    guard let end = Int(match.2) else { return t("tool.sessionOutline") }
    let lo = min(from, end), hi = max(from, end)
    let label = hi - lo < 5
      ? (lo...hi).map(String.init).joined(separator: t("turns.nameSep"))
      : "\(lo)\u{2013}\(hi)"
    return t("tool.sessionTurns", ["range": label])
  }
}
