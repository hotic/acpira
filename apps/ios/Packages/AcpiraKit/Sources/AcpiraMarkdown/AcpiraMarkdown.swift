import Foundation

// Markdown → block AST, the subset the webview's streamdown pipeline renders for agent prose: GFM blocks (headings,
// paragraphs, nested lists, quotes, fenced code, tables, rules), `$$` display math and the inline set (code, strong,
// emphasis, strike, links, images, autolinks, breaks). It is safe against half-streamed syntax: an unclosed fence runs
// to the end, an unclosed delimiter stays literal text. Single-dollar math stays off, like the webview ("$5 and $10").

public indirect enum MDInline: Hashable, Sendable {
  case text(String)
  case code(String)
  case strong([MDInline])
  case emphasis([MDInline])
  case strike([MDInline])
  case link(url: String, [MDInline])
  /// `![alt](src)`: the source as written (angle brackets dropped), the alt text plain
  case image(url: String, alt: String)
  case lineBreak
}

public enum MDAlign: Hashable, Sendable { case none, left, center, right }

public indirect enum MDBlock: Hashable, Sendable {
  case heading(level: Int, [MDInline])
  case paragraph([MDInline])
  /// Items are block lists; `start` is the first number of an ordered list
  case list(ordered: Bool, start: Int, items: [[MDBlock]])
  case quote([MDBlock])
  case code(lang: String?, text: String)
  case table(align: [MDAlign], header: [[MDInline]], rows: [[[MDInline]]])
  case rule
  case math(String)
}

public enum Markdown {
  public static func parse(_ source: String) -> [MDBlock] {
    let lines = source.replacingOccurrences(of: "\r\n", with: "\n").components(separatedBy: "\n")
    var parser = BlockParser(lines: lines)
    return parser.blocks()
  }

  /// Plain text of inline content (copy, accessibility)
  public static func plain(_ inlines: [MDInline]) -> String {
    inlines.map { inline in
      switch inline {
      case .text(let s), .code(let s): s
      case .strong(let c), .emphasis(let c), .strike(let c), .link(_, let c): plain(c)
      case .image(_, let alt): alt
      case .lineBreak: "\n"
      }
    }.joined()
  }
}

// MARK: - Blocks

struct BlockParser {
  let lines: [String]
  var i = 0

  init(lines: [String]) { self.lines = lines }

  static func indentWidth(_ line: Substring) -> Int {
    var n = 0
    for c in line { if c == " " { n += 1 } else if c == "\t" { n += 4 } else { break } }
    return n
  }

  static func isBlank(_ line: String) -> Bool { line.allSatisfy { $0 == " " || $0 == "\t" } }

  static func fence(_ line: String) -> (marker: String, lang: String?)? {
    let trimmed = line.drop { $0 == " " }
    guard BlockParser.indentWidth(Substring(line)) < 4 else { return nil }
    for mark in ["```", "~~~"] where trimmed.hasPrefix(mark) {
      let run = trimmed.prefix { $0 == mark.first! }
      let info = trimmed.dropFirst(run.count).trimmingCharacters(in: .whitespaces)
      if mark == "```" && info.contains("`") { return nil }
      return (String(run), info.isEmpty ? nil : String(info.split(separator: " ").first ?? ""))
    }
    return nil
  }

  static func heading(_ line: String) -> (Int, String)? {
    guard let m = line.firstMatch(of: /^ {0,3}(#{1,6})(?:[ \t]+(.*?))?[ \t]*#*[ \t]*$/) else { return nil }
    return (m.1.count, String(m.2 ?? ""))
  }

  static func isRule(_ line: String) -> Bool {
    line.wholeMatch(of: /^ {0,3}(?:(?:\*[ \t]*){3,}|(?:-[ \t]*){3,}|(?:_[ \t]*){3,})$/) != nil
  }

  /// A list marker: indent, ordered, number, content column
  static func listMarker(_ line: String) -> (indent: Int, ordered: Bool, number: Int, content: Int)? {
    guard let m = line.firstMatch(of: /^( *)([-*+]|\d{1,9}[.)])( +|$)/) else { return nil }
    let indent = m.1.count
    let marker = String(m.2)
    let ordered = marker.first!.isNumber
    let spaces = min(max(m.3.count, 1), 4)
    return (indent, ordered, ordered ? Int(marker.dropLast()) ?? 1 : 1, indent + marker.count + spaces)
  }

  static func isTableDelimiter(_ line: String) -> Bool {
    line.wholeMatch(of: /^ {0,3}\|?[ \t]*:?-+:?[ \t]*(?:\|[ \t]*:?-+:?[ \t]*)*\|?[ \t]*$/) != nil && line.contains("-")
  }

  static func cells(_ line: String) -> [String] {
    var s = line.trimmingCharacters(in: .whitespaces)
    if s.hasPrefix("|") { s.removeFirst() }
    if s.hasSuffix("|") && !s.hasSuffix("\\|") { s.removeLast() }
    // Split on unescaped pipes outside inline code
    var out: [String] = []
    var cell = ""
    var inCode = false
    var prev: Character = " "
    for c in s {
      if c == "`" { inCode.toggle() }
      if c == "|" && !inCode && prev != "\\" { out.append(cell); cell = "" } else { cell.append(c) }
      prev = c
    }
    out.append(cell)
    return out.map { $0.trimmingCharacters(in: .whitespaces).replacingOccurrences(of: "\\|", with: "|") }
  }

  /// Whether a line can interrupt a paragraph
  func startsBlock(_ line: String) -> Bool {
    BlockParser.fence(line) != nil || BlockParser.heading(line) != nil || BlockParser.isRule(line)
      || line.drop { $0 == " " }.hasPrefix(">") || line.trimmingCharacters(in: .whitespaces).hasPrefix("$$")
      || (BlockParser.listMarker(line).map { m in
        // An ordered list interrupts a paragraph only from 1; an empty item never does
        (!m.ordered || m.number == 1) && !BlockParser.isBlank(String(line.dropFirst(min(m.content, line.count))))
      } ?? false)
  }

  mutating func blocks() -> [MDBlock] {
    var out: [MDBlock] = []
    while i < lines.count {
      let line = lines[i]
      if BlockParser.isBlank(line) { i += 1; continue }
      if let f = BlockParser.fence(line) { out.append(fencedCode(f)); continue }
      if line.trimmingCharacters(in: .whitespaces).hasPrefix("$$") { out.append(math()); continue }
      if let (level, text) = BlockParser.heading(line) { out.append(.heading(level: level, Inline.parse(text))); i += 1; continue }
      if BlockParser.isRule(line) { out.append(.rule); i += 1; continue }
      if line.drop(while: { $0 == " " }).hasPrefix(">") { out.append(quote()); continue }
      if BlockParser.listMarker(line) != nil { out.append(list()); continue }
      if i + 1 < lines.count, line.contains("|"), BlockParser.isTableDelimiter(lines[i + 1]) { out.append(table()); continue }
      out.append(paragraph())
    }
    return out
  }

  mutating func fencedCode(_ f: (marker: String, lang: String?)) -> MDBlock {
    let indent = BlockParser.indentWidth(Substring(lines[i]))
    i += 1
    var body: [String] = []
    while i < lines.count {
      let line = lines[i]
      let trimmed = line.trimmingCharacters(in: .whitespaces)
      if trimmed.hasPrefix(f.marker) && trimmed.allSatisfy({ $0 == f.marker.first! }) { i += 1; break }
      // Strip up to the fence's own indent
      var drop = 0
      for c in line where c == " " && drop < indent { drop += 1 }
      body.append(String(line.dropFirst(drop)))
      i += 1
    }
    return .code(lang: f.lang, text: body.joined(separator: "\n"))
  }

  mutating func math() -> MDBlock {
    var first = lines[i].trimmingCharacters(in: .whitespaces).dropFirst(2)
    // `$$ x $$` on one line
    if first.hasSuffix("$$") && first.count >= 2 {
      first = first.dropLast(2)
      i += 1
      return .math(first.trimmingCharacters(in: .whitespaces))
    }
    var body = first.isEmpty ? [] : [String(first)]
    i += 1
    while i < lines.count {
      let t = lines[i].trimmingCharacters(in: .whitespaces)
      if t.hasSuffix("$$") { let rest = t.dropLast(2); if !rest.isEmpty { body.append(String(rest)) }; i += 1; break }
      body.append(lines[i]); i += 1
    }
    return .math(body.joined(separator: "\n").trimmingCharacters(in: .whitespacesAndNewlines))
  }

  mutating func quote() -> MDBlock {
    var inner: [String] = []
    while i < lines.count {
      let line = lines[i]
      let stripped = line.drop { $0 == " " }
      if stripped.hasPrefix(">") {
        var rest = stripped.dropFirst()
        if rest.first == " " { rest = rest.dropFirst() }
        inner.append(String(rest)); i += 1
      } else if !BlockParser.isBlank(line), let last = inner.last, !BlockParser.isBlank(last), !startsBlock(line) {
        // Lazy continuation of the quoted paragraph
        inner.append(line); i += 1
      } else { break }
    }
    var sub = BlockParser(lines: inner)
    return .quote(sub.blocks())
  }

  mutating func list() -> MDBlock {
    let first = BlockParser.listMarker(lines[i])!
    var items: [[MDBlock]] = []
    while i < lines.count, let m = BlockParser.listMarker(lines[i]), m.ordered == first.ordered,
          m.indent < first.content, m.indent <= first.indent + 3 {
      // The item's first line, dedented to its content column
      var body = [String(lines[i].dropFirst(min(m.content, lines[i].count)))]
      i += 1
      var blankRun = false
      while i < lines.count {
        let line = lines[i]
        if BlockParser.isBlank(line) { body.append(""); blankRun = true; i += 1; continue }
        let width = BlockParser.indentWidth(Substring(line))
        if width >= m.content {
          body.append(String(line.dropFirst(min(m.content, line.count)))); blankRun = false; i += 1; continue
        }
        // A sibling or an outer block ends the item; a lazy line continues its paragraph
        if blankRun || BlockParser.listMarker(line) != nil || startsBlock(line) { break }
        body.append(line.trimmingCharacters(in: .whitespaces)); i += 1
      }
      while body.last.map(BlockParser.isBlank) == true { body.removeLast() }
      var sub = BlockParser(lines: body)
      items.append(sub.blocks())
      // A blank line between items keeps the list going; anything else stops it
      var j = i
      while j < lines.count && BlockParser.isBlank(lines[j]) { j += 1 }
      if j < lines.count, let next = BlockParser.listMarker(lines[j]), next.ordered == first.ordered, next.indent < first.content { i = j } else { break }
    }
    return .list(ordered: first.ordered, start: first.number, items: items)
  }

  mutating func table() -> MDBlock {
    let header = BlockParser.cells(lines[i]).map(Inline.parse)
    let align: [MDAlign] = BlockParser.cells(lines[i + 1]).map { spec in
      switch (spec.hasPrefix(":"), spec.hasSuffix(":")) {
      case (true, true): .center
      case (true, false): .left
      case (false, true): .right
      default: .none
      }
    }
    i += 2
    var rows: [[[MDInline]]] = []
    while i < lines.count, !BlockParser.isBlank(lines[i]), lines[i].contains("|") || !startsBlock(lines[i]) {
      if startsBlock(lines[i]) { break }
      var row = BlockParser.cells(lines[i]).map(Inline.parse)
      // Rows take the header's width
      if row.count < header.count { row += Array(repeating: [], count: header.count - row.count) }
      rows.append(Array(row.prefix(header.count)))
      i += 1
    }
    return .table(align: align, header: header, rows: rows)
  }

  mutating func paragraph() -> MDBlock {
    var text: [String] = [lines[i]]
    i += 1
    while i < lines.count, !BlockParser.isBlank(lines[i]), !startsBlock(lines[i]) {
      if i + 1 < lines.count, lines[i].contains("|"), BlockParser.isTableDelimiter(lines[i + 1]) { break }
      text.append(lines[i]); i += 1
    }
    return .paragraph(Inline.parse(text.map { $0.drop { $0 == " " } }.joined(separator: "\n")))
  }
}

// MARK: - Inline

enum Inline {
  static func parse(_ text: String) -> [MDInline] {
    var p = InlineParser(chars: Array(text))
    return merge(p.parse(until: nil))
  }

  /// Adjacent text runs joined into one
  static func merge(_ items: [MDInline]) -> [MDInline] {
    var out: [MDInline] = []
    for item in items {
      if case .text(let s) = item, case .text(let prev)? = out.last { out[out.count - 1] = .text(prev + s) }
      else if case .text(let s) = item, s.isEmpty { continue }
      else { out.append(item) }
    }
    return out
  }
}

struct InlineParser {
  let chars: [Character]
  var i = 0

  static let punctuation = Set("!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~")

  /// Parses until the closing delimiter `close` (consumed) or the end; returns nil content when `close` never came
  mutating func parse(until close: String?) -> [MDInline] {
    var out: [MDInline] = []
    var text = ""
    func flush() { if !text.isEmpty { out.append(.text(text)); text = "" } }
    while i < chars.count {
      if let close, matches(close, at: i) { flush(); return out }
      let c = chars[i]
      // Escapes
      if c == "\\", i + 1 < chars.count {
        let n = chars[i + 1]
        if n == "\n" { flush(); out.append(.lineBreak); i += 2; continue }
        if InlineParser.punctuation.contains(n) { text.append(n); i += 2; continue }
      }
      // Hard break: two trailing spaces; a soft break renders as a space
      if c == "\n" {
        if text.hasSuffix("  ") { text = String(text.dropLast(2)); flush(); out.append(.lineBreak) }
        else { text = String(text.reversed().drop { $0 == " " }.reversed()); text.append(" ") }
        i += 1
        while i < chars.count && chars[i] == " " { i += 1 }
        continue
      }
      if c == "`", let code = codeSpan() { flush(); out.append(.code(code)); continue }
      if c == "!", i + 1 < chars.count, chars[i + 1] == "[", let image = imageAt() { flush(); out.append(image); continue }
      if c == "[", let link = linkAt() { flush(); out.append(link); continue }
      if c == "<", let auto = autolink() { flush(); out.append(auto); continue }
      if (c == "h" || c == "w"), let bare = bareURL() { flush(); out.append(bare); continue }
      if c == "*" || c == "_" || c == "~" {
        if let styled = delimited() { flush(); out.append(styled); continue }
        // A run that opens nothing stays literal
        let run = runLength(at: i, of: c)
        text.append(String(repeating: c, count: run)); i += run; continue
      }
      text.append(c); i += 1
    }
    flush()
    return out
  }

  func matches(_ s: String, at index: Int) -> Bool {
    let needle = Array(s)
    guard index + needle.count <= chars.count else { return false }
    return Array(chars[index..<index + needle.count]) == needle
  }

  func runLength(at index: Int, of c: Character) -> Int {
    var n = 0
    while index + n < chars.count && chars[index + n] == c { n += 1 }
    return n
  }

  mutating func codeSpan() -> String? {
    let run = runLength(at: i, of: "`")
    var j = i + run
    while j < chars.count {
      if chars[j] == "`" {
        let close = runLength(at: j, of: "`")
        if close == run {
          var body = String(chars[(i + run)..<j]).replacingOccurrences(of: "\n", with: " ")
          if body.count >= 2, body.first == " ", body.last == " ", body.contains(where: { $0 != " " }) {
            body = String(body.dropFirst().dropLast())
          }
          i = j + close
          return body
        }
        j += close
      } else { j += 1 }
    }
    return nil
  }

  /// `**strong**`, `*em*`, `_em_`, `__strong__`, `~~strike~~`, `***both***`
  mutating func delimited() -> MDInline? {
    let c = chars[i]
    let run = runLength(at: i, of: c)
    // The opener must be followed by non-space
    guard i + run < chars.count, !chars[i + run].isWhitespace else { return nil }
    // Intraword underscores stay literal
    if c == "_", i > 0, chars[i - 1].isLetter || chars[i - 1].isNumber { return nil }
    if c == "~" && run != 2 { return nil }
    let width = c == "~" ? 2 : min(run, 3)
    let delimiter = String(repeating: c, count: width)
    // Find a closer that is preceded by non-space
    var j = i + width
    var depth = 0
    while j < chars.count {
      if chars[j] == "`" {
        // Skip code spans
        let r = runLength(at: j, of: "`")
        var k = j + r
        var closed = false
        while k < chars.count { if chars[k] == "`" && runLength(at: k, of: "`") == r { closed = true; break }; k += 1 }
        j = closed ? k + r : j + r
        continue
      }
      if chars[j] == "\\" { j += 2; continue }
      if chars[j] == "[" { depth += 1 } else if chars[j] == "]" { depth = max(0, depth - 1) }
      if chars[j] == c, matches(delimiter, at: j), !chars[j - 1].isWhitespace {
        let after = j + width
        let next = runLength(at: j, of: c)
        // For a single delimiter, a longer run is a different (strong) closer unless it is the outer one
        if width == 1 && next == 2 { j += 2; continue }
        if c == "_" && after < chars.count && (chars[after].isLetter || chars[after].isNumber) { j += 1; continue }
        let inner = String(chars[(i + width)..<j])
        i = after
        let content = Inline.parse(inner)
        switch (c, width) {
        case ("~", _): return .strike(content)
        case (_, 3): return .strong([.emphasis(content)])
        case (_, 2): return .strong(content)
        default: return .emphasis(content)
        }
      }
      j += 1
    }
    return nil
  }

  mutating func linkAt() -> MDInline? {
    // [label](url "title")
    var depth = 0
    var j = i
    while j < chars.count {
      if chars[j] == "\\" { j += 2; continue }
      if chars[j] == "[" { depth += 1 }
      if chars[j] == "]" { depth -= 1; if depth == 0 { break } }
      j += 1
    }
    guard j + 1 < chars.count, chars[j] == "]", chars[j + 1] == "(" else { return nil }
    var k = j + 2
    var parens = 1
    while k < chars.count {
      if chars[k] == "(" { parens += 1 }
      if chars[k] == ")" { parens -= 1; if parens == 0 { break } }
      k += 1
    }
    guard k < chars.count else { return nil }
    let label = String(chars[(i + 1)..<j])
    var target = String(chars[(j + 2)..<k]).trimmingCharacters(in: .whitespaces)
    // `<a path with spaces>` keeps its spaces; otherwise a space starts the title
    if target.hasPrefix("<"), let close = target.firstIndex(of: ">") {
      target = String(target[target.index(after: target.startIndex)..<close])
    } else if let space = target.firstIndex(of: " ") {
      target = String(target[..<space])
    }
    i = k + 1
    return .link(url: target, Inline.parse(label))
  }

  /// `![alt](src "title")`: a link's syntax after the bang; the bang stays text when no link follows
  mutating func imageAt() -> MDInline? {
    let start = i
    i += 1
    guard case .link(let url, let label)? = linkAt() else { i = start; return nil }
    return .image(url: url, alt: Markdown.plain(label))
  }

  mutating func autolink() -> MDInline? {
    guard let close = chars[i...].firstIndex(of: ">") else { return nil }
    let body = String(chars[(i + 1)..<close])
    guard body.firstMatch(of: /^[a-zA-Z][a-zA-Z0-9+.-]{1,31}:[^\s<>]*$/) != nil else { return nil }
    i = close + 1
    return .link(url: body, [.text(body)])
  }

  /// GFM bare `https://…` / `www.…` links, trailing punctuation excluded
  mutating func bareURL() -> MDInline? {
    if i > 0, chars[i - 1].isLetter || chars[i - 1].isNumber { return nil }
    let rest = String(chars[i...])
    guard let m = rest.prefixMatch(of: /(?:https?:\/\/|www\.)[^\s<]+/) else { return nil }
    var url = String(m.0)
    while let last = url.last, ".,:;!?)\"'*_~".contains(last) {
      // Keep a closing paren that balances one inside the URL
      if last == ")" && url.filter({ $0 == "(" }).count >= url.filter({ $0 == ")" }).count { break }
      url.removeLast()
    }
    // CJK punctuation ends the URL as well
    if let cut = url.firstIndex(where: { "，。；：！？、）》」』".contains($0) }) { url = String(url[..<cut]) }
    guard !url.isEmpty else { return nil }
    i += url.count
    return .link(url: url.hasPrefix("www.") ? "http://\(url)" : url, [.text(url)])
  }
}
