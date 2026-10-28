import Foundation

// chat/fileLinks.ts parseFileLink: the workspace paths agent Markdown names (a link's target, a code span), with an
// optional line (`src/a.ts:12`, `#L12`, `file:///…`). The prose turns each into an `acpira-file:` link the app opens
// in its file page (AppModel+Links.swift), the desktop's open-in-panel.

nonisolated struct FileLink: Equatable, Sendable {
  var path: String
  var line: Int?
}

nonisolated enum FileLinks {
  static let scheme = "acpira-file"
  private static let webTLD: Set<String> = ["com", "org", "net", "io", "edu", "gov", "cn", "co", "dev", "info", "xyz", "me", "ai"]

  static func parse(_ text: String) -> FileLink? {
    let raw = text.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !raw.isEmpty, !raw.contains(where: \.isWhitespace) else { return nil }
    if let own = decode(URL(string: raw)) { return own }
    if raw.hasPrefix("#") || raw.prefixMatch(of: /(?:https?|mailto|javascript|data|vbscript|blob|tel):/.ignoresCase()) != nil { return nil }
    if raw.lowercased().hasPrefix("file:") { return parseFileURL(raw) }
    return pathAndLine(raw)
  }

  /// The app's own link for a file, as the prose sets it on a path
  static func url(_ link: FileLink) -> URL? {
    var c = URLComponents()
    c.scheme = scheme
    c.host = "open"
    c.queryItems = [URLQueryItem(name: "path", value: link.path)] + (link.line.map { [URLQueryItem(name: "line", value: String($0))] } ?? [])
    return c.url
  }

  static func decode(_ url: URL?) -> FileLink? {
    guard let url, url.scheme == scheme, let items = URLComponents(url: url, resolvingAgainstBaseURL: false)?.queryItems,
          let path = items.first(where: { $0.name == "path" })?.value, !path.isEmpty else { return nil }
    return FileLink(path: path, line: items.first { $0.name == "line" }?.value.flatMap(Int.init))
  }

  private static func parseFileURL(_ href: String) -> FileLink? {
    guard let url = URLComponents(string: href), url.scheme?.lowercased() == "file" else { return nil }
    // Line suffixes are read before percent decoding: a name ending in %23L12 is no line anchor
    let parsed = pathAndLine(url.percentEncodedPath)
    var path = (parsed?.path ?? url.percentEncodedPath).removingPercentEncoding ?? url.percentEncodedPath
    if let host = url.host, !host.isEmpty, host != "localhost" { path = "//\(host)\(path)" }
    else if path.wholeMatch(of: /\/[A-Za-z]:.*/) != nil { path.removeFirst() }
    guard !path.isEmpty else { return nil }
    let line = url.fragment.flatMap { $0.prefixMatch(of: /[Ll](\d+)/).flatMap { Int($0.1) } } ?? parsed?.line
    return FileLink(path: path, line: line)
  }

  private static func pathAndLine(_ text: String) -> FileLink? {
    if let m = text.wholeMatch(of: /(.*)#[Ll](\d+)(?:-\d+)?/) {
      let path = String(m.1)
      return isFilePath(path) ? FileLink(path: path, line: Int(m.2)) : nil
    }
    if let m = text.wholeMatch(of: /(.+?):(\d+)(?:[–-]\d+)?(?::\d+)?/), isFilePath(String(m.1)) {
      return FileLink(path: String(m.1), line: Int(m.2))
    }
    return isFilePath(text) ? FileLink(path: text) : nil
  }

  static func isFilePath(_ path: String) -> Bool {
    guard !path.isEmpty else { return false }
    if path.prefixMatch(of: /\\\\[^\\]+\\[^\\]+/) != nil { return true }
    if path.prefixMatch(of: /(?:\.{0,2}\/|[A-Za-z]:[\\\/]|\/)/) != nil { return true }
    let slashed = path.contains("/") || path.contains("\\")
    if path.hasPrefix(".") && slashed { return true }
    if slashed && path.firstMatch(of: /\.[A-Za-z][\w-]*$/) != nil { return true }
    guard let ext = path.wholeMatch(of: /[^\s\/\\]+\.([A-Za-z][\w-]{1,15})/) else { return false }
    return !webTLD.contains(ext.1.lowercased())
  }
}
