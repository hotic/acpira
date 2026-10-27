import AcpiraDesign
import AcpiraModels
import Foundation
import JavaScriptCore
import SwiftUI

// chat/codeLanguage.ts, codeSyntax.ts and codeDiffHighlight.ts: which grammar a file or a fence gets, and its tokens in
// the webview's colours. The tokens come from shiki itself, run in JavaScriptCore (`Resources/shiki`, written by
// scripts/export/ios-shiki.ts with the webview's grammars, themes and JavaScriptCore regex options), so a line is
// coloured exactly as the desktop colours it.

/// A run of a line in one colour; `color` nil is the theme's default foreground, drawn in the card's own ink
nonisolated public struct CodeToken: Hashable, Sendable {
  public var text: String
  public var color: String?
}

/// The scheme the tokens are coloured for (light-plus / dark-plus)
nonisolated public enum CodeTheme: String, Sendable {
  case dark = "dark-plus", light = "light-plus"

  public init(_ scheme: ColorScheme) { self = scheme == .dark ? .dark : .light }

  /// The theme's default foreground: tokens in it render in the card's ink, as the webview's plain text nodes do
  var defaultColor: String { self == .dark ? "#D4D4D4" : "#000000" }
}

/// codeLanguage.ts: the grammar a path's extension names; no guess from content or an extensionless name
nonisolated func codeLanguage(_ path: String) -> String? {
  let base = path.split(whereSeparator: { $0 == "/" || $0 == "\\" }).last.map(String.init) ?? ""
  // /[^.]\.([^.]+)$/: a suffix after the last dot, which neither starts the name nor follows another dot
  guard let dot = base.lastIndex(of: "."), dot != base.startIndex, base[base.index(before: dot)] != ".",
        base.index(after: dot) != base.endIndex else { return nil }
  return CodeLanguages.extensions[base[base.index(after: dot)...].lowercased()]
}

/// codeLanguage.ts fenceLanguage: a fence is coloured only when its info string names a known grammar
nonisolated func fenceLanguage(_ tag: String?) -> String? {
  guard let name = tag?.trimmingCharacters(in: .whitespaces).lowercased(), !name.isEmpty else { return nil }
  if CodeLanguages.all.contains(name) { return name }
  return CodeLanguages.extensions[name] ?? CodeLanguages.fenceAliases[name]
}

/// codeLanguage.ts's tables, exported beside the grammars (`shiki/languages.json`), so the app answers as the webview does
nonisolated enum CodeLanguages {
  private struct Tables: Decodable {
    var languages: [String]
    var extensions: [String: String]
    var fenceAliases: [String: String]
  }
  private static let tables: Tables? = Bundle.module.url(forResource: "shiki", withExtension: nil)
    .flatMap { try? Data(contentsOf: $0.appending(path: "languages.json")) }
    .flatMap { try? JSONDecoder().decode(Tables.self, from: $0) }
  static let all = Set(tables?.languages ?? [])
  static var extensions: [String: String] { tables?.extensions ?? [:] }
  static var fenceAliases: [String: String] { tables?.fenceAliases ?? [:] }
}

/// shiki in one JavaScriptCore context, created on first use. Calls are synchronous and serialised by a lock, so a
/// view calls it off the main actor (a first grammar costs a pass over its regexes) and the full-length render calls
/// it in place. Answers are cached by source, so a card that scrolls back in colours at once
nonisolated final class SyntaxHighlighter: @unchecked Sendable {
  static let shared = SyntaxHighlighter()

  private let lock = NSLock()
  private var api: JSValue?
  private var context: JSContext?
  private var failed = false
  private var cache: [String: [[CodeToken]]] = [:]
  private var order: [String] = []
  private static let cacheSize = 200

  /// One token array per source line (codeSyntax.ts tokenizeCode); `nil` without a language or a working highlighter
  func tokenize(_ code: String, language: String?, theme: CodeTheme) -> [[CodeToken]]? {
    guard let language else { return nil }
    let key = "\(theme.rawValue)\u{0}\(language)\u{0}\(code)"
    lock.lock(); defer { lock.unlock() }
    if let hit = cache[key] { return hit }
    guard let api = ready(language) else { return nil }
    guard let json = api.invokeMethod("tokenize", withArguments: [code.replacingOccurrences(of: "\r\n", with: "\n"), language, theme.rawValue])?
      .toString(), let raw = try? JSONSerialization.jsonObject(with: Data(json.utf8)) as? [[[String]]] else { return nil }
    let lines = raw.map { Self.merge($0, theme: theme) }
    cache[key] = lines
    order.append(key)
    if order.count > Self.cacheSize { cache.removeValue(forKey: order.removeFirst()) }
    return lines
  }

  /// A cached answer only, never tokenizing: what a view can draw at once
  func cached(_ code: String, language: String?, theme: CodeTheme) -> [[CodeToken]]? {
    guard let language else { return nil }
    lock.lock(); defer { lock.unlock() }
    return cache["\(theme.rawValue)\u{0}\(language)\u{0}\(code)"]
  }

  /// The context with `language` loaded (lock held)
  private func ready(_ language: String) -> JSValue? {
    if failed { return nil }
    if api == nil {
      guard let dir = Bundle.module.url(forResource: "shiki", withExtension: nil),
            let script = try? String(contentsOf: dir.appending(path: "shiki.js"), encoding: .utf8),
            let dark = try? String(contentsOf: dir.appending(path: "themes/dark-plus.json"), encoding: .utf8),
            let light = try? String(contentsOf: dir.appending(path: "themes/light-plus.json"), encoding: .utf8),
            let context = JSContext() else { failed = true; return nil }
      var error = false
      context.exceptionHandler = { _, _ in error = true }
      context.evaluateScript(script)
      let api = context.objectForKeyedSubscript("acpiraShiki")
      api?.invokeMethod("init", withArguments: ["[\(dark),\(light)]"])
      if error || api == nil || api!.isUndefined { failed = true; return nil }
      self.context = context
      self.api = api
    }
    guard let api else { return nil }
    if api.invokeMethod("has", withArguments: [language])?.toBool() != true {
      guard let url = Bundle.module.url(forResource: "shiki", withExtension: nil)?.appending(path: "langs/\(language).json"),
            let grammars = try? String(contentsOf: url, encoding: .utf8) else { return nil }
      api.invokeMethod("load", withArguments: [language, grammars])
    }
    return api
  }

  /// codeSyntax.ts mergeTokens: adjacent tokens of one colour become one run, whitespace joins the run before it, and
  /// the theme's default foreground is no colour
  static func merge(_ line: [[String]], theme: CodeTheme) -> [CodeToken] {
    var out: [CodeToken] = []
    for pair in line {
      guard pair.count == 2, !pair[0].isEmpty else { continue }
      let text = pair[0], raw = pair[1]
      if !out.isEmpty, !text.contains(where: { !$0.isWhitespace }) {
        out[out.count - 1].text += text
        continue
      }
      let color = !raw.isEmpty && raw.uppercased() != theme.defaultColor ? raw : nil
      if let last = out.last, last.color == color {
        out[out.count - 1].text += text
      } else {
        out.append(CodeToken(text: text, color: color))
      }
    }
    return out
  }

  /// codeDiffHighlight.ts without a source file: each run of rows between hunk headers is tokenized side by side, the
  /// old side for deleted rows and the new side for the rest, so deleted lines never shift the new side's grammar.
  /// One entry per row, `nil` for a hunk header or when nothing could be coloured
  func diff(_ lines: [DiffLine], path: String, theme: CodeTheme) -> [[CodeToken]?]? {
    guard let language = codeLanguage(path) else { return nil }
    var out = [[CodeToken]?](repeating: nil, count: lines.count)
    var start = 0
    for end in 0...lines.count where end == lines.count || lines[end].kind == .hunk {
      let segment = Array(start..<end)
      let old = segment.filter { lines[$0].kind != .add }
      let new = segment.filter { lines[$0].kind != .del }
      if !segment.isEmpty,
         let oldTokens = tokenize(old.map { lines[$0].code }.joined(separator: "\n"), language: language, theme: theme),
         let newTokens = tokenize(new.map { lines[$0].code }.joined(separator: "\n"), language: language, theme: theme) {
        for (i, row) in old.enumerated() where lines[row].kind == .del && i < oldTokens.count { out[row] = oldTokens[i] }
        for (i, row) in new.enumerated() where i < newTokens.count { out[row] = newTokens[i] }
      }
      start = end + 1
    }
    return out
  }

  /// The diff's answer from the cache alone (both sides of every segment), or nil when any side is still to tokenize
  func cachedDiff(_ lines: [DiffLine], path: String, theme: CodeTheme) -> [[CodeToken]?]? {
    guard let language = codeLanguage(path) else { return nil }
    var start = 0
    for end in 0...lines.count where end == lines.count || lines[end].kind == .hunk {
      let segment = Array(start..<end)
      let old = segment.filter { lines[$0].kind != .add }.map { lines[$0].code }.joined(separator: "\n")
      let new = segment.filter { lines[$0].kind != .del }.map { lines[$0].code }.joined(separator: "\n")
      if !segment.isEmpty && (cached(old, language: language, theme: theme) == nil || cached(new, language: language, theme: theme) == nil) {
        return nil
      }
      start = end + 1
    }
    return diff(lines, path: path, theme: theme)
  }
}

extension DiffLine {
  /// The line's code: the wire text carries its sign column (' ', '-', '+'; transcript/diff.rs), which the gutter draws
  /// on its own (codeDiff.ts plainDiffRows); a hunk header stays whole
  nonisolated var code: String { kind == .hunk ? text : String(text.dropFirst()) }
}

extension Color {
  /// `#rrggbb` (or `#rrggbbaa`) from a theme
  init?(themeHex hex: String) {
    var s = hex
    if s.hasPrefix("#") { s.removeFirst() }
    guard s.count == 6 || s.count == 8, let v = UInt64(s, radix: 16) else { return nil }
    let rgb = s.count == 8 ? v >> 8 : v
    let alpha = s.count == 8 ? Double(v & 0xff) / 255 : 1
    self.init(.sRGB, red: Double((rgb >> 16) & 0xff) / 255, green: Double((rgb >> 8) & 0xff) / 255,
              blue: Double(rgb & 0xff) / 255, opacity: alpha)
  }
}

/// Tokens as one attributed run, each coloured run in its colour and the rest left to the inherited ink
func codeRuns(_ tokens: [CodeToken], size: CGFloat) -> AttributedString {
  var line = AttributedString()
  for token in tokens {
    var run = CJKWidth.attributed(token.text, size: size, paragraph: false)
    if let hex = token.color, let color = Color(themeHex: hex) { run.foregroundColor = color }
    line += run
  }
  return line
}

/// A fence's source as the webview shows it (CodeBlock.tsx syntaxChildren): coloured when its tokens are for this text;
/// while it streams on past them, the complete lines they cover keep their colours and the growing tail is plain
func codeSource(_ text: String, tokenized: String?, lines: [[CodeToken]]?, size: CGFloat) -> AttributedString {
  // Byte prefixes, as the webview's startsWith: the tail is cut at a byte offset of the tokenized text
  guard let tokenized, let lines, text.utf8.starts(with: tokenized.utf8) else { return CJKWidth.attributed(text, size: size, paragraph: false) }
  // The lines to colour: all of them for the same text, else those a newline closed
  let keep = text == tokenized ? lines.count : min(lines.count, tokenized.utf8.reduce(0) { $1 == UInt8(ascii: "\n") ? $0 + 1 : $0 })
  var out = AttributedString()
  for (i, line) in lines.prefix(keep).enumerated() {
    out += codeRuns(line, size: size)
    if i < keep - 1 || text != tokenized { out += AttributedString("\n") }
  }
  if text != tokenized {
    let tail = tokenized.utf8.lastIndex(of: UInt8(ascii: "\n")).map { tokenized.utf8.distance(from: tokenized.utf8.startIndex, to: $0) + 1 } ?? 0
    out += CJKWidth.attributed(String(decoding: text.utf8.dropFirst(tail), as: UTF8.self), size: size, paragraph: false)
  }
  return out
}
