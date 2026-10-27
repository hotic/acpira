import Foundation

// The part of mermaid's flowchart grammar agents write: `graph` / `flowchart` with a direction, node shapes, links
// with their stroke, heads, length and labels, chains and `&` groups. Styling statements are skipped; anything else
// (subgraphs, other diagram types) fails the parse and the block stays a code block.

struct FlowChart: Equatable {
  enum Shape: Equatable { case rect, round, stadium, subroutine, circle, diamond, hexagon }
  enum Stroke: Equatable { case normal, thick, dotted, invisible }
  enum Head: Equatable { case none, point, cross, circle }

  struct Node: Equatable {
    var id: String
    var label: String
    var shape: Shape
  }

  struct Edge: Equatable {
    var from: String
    var to: String
    var label: String
    /// Extra dashes lengthen a link: its dagre minlen
    var length: Int
    var stroke: Stroke
    var start: Head
    var end: Head
  }

  var direction: RankDir
  var nodes: [Node] = []
  var edges: [Edge] = []

  static func parse(_ source: String) -> FlowChart? {
    var lines = source.components(separatedBy: .newlines)
      .map { $0.trimmingCharacters(in: .whitespaces) }
      .filter { !$0.isEmpty && !$0.hasPrefix("%%") }
    guard let header = lines.first else { return nil }
    lines.removeFirst()
    let words = header.split(separator: " ", omittingEmptySubsequences: true)
    guard let kind = words.first, kind == "graph" || kind == "flowchart" else { return nil }
    var chart = FlowChart(direction: .tb)
    if words.count > 1 {
      switch words[1].uppercased().trimmingCharacters(in: CharacterSet(charactersIn: ";")) {
      case "LR": chart.direction = .lr
      case "RL": chart.direction = .rl
      case "BT": chart.direction = .bt
      case "TB", "TD": chart.direction = .tb
      default: return nil
      }
    }
    for line in lines {
      for statement in line.split(separator: ";", omittingEmptySubsequences: true) {
        let s = statement.trimmingCharacters(in: .whitespaces)
        guard !s.isEmpty else { continue }
        let keyword = s.prefix { $0.isLetter }
        if ["classDef", "class", "style", "linkStyle", "click", "direction", "accTitle", "accDescr"].contains(String(keyword)) {
          continue
        }
        if keyword == "subgraph" || s == "end" { return nil }
        var scanner = FlowScanner(Array(s))
        guard scanner.statement(into: &chart) else { return nil }
      }
    }
    return chart.nodes.isEmpty ? nil : chart
  }

  mutating func vertex(_ id: String, label: String?, shape: Shape?) {
    if let i = nodes.firstIndex(where: { $0.id == id }) {
      if let label { nodes[i].label = label }
      if let shape { nodes[i].shape = shape }
    } else {
      nodes.append(Node(id: id, label: label ?? id, shape: shape ?? .rect))
    }
  }
}

/// One statement: `a[..] & b --> |label| c -- text --> d`
private struct FlowScanner {
  let chars: [Character]
  var at = 0

  init(_ chars: [Character]) { self.chars = chars }

  var current: Character? { at < chars.count ? chars[at] : nil }

  mutating func skipSpaces() {
    while let c = current, c == " " || c == "\t" { at += 1 }
  }

  func lookahead(_ s: String) -> Bool {
    let a = Array(s)
    guard at + a.count <= chars.count else { return false }
    return Array(chars[at..<(at + a.count)]) == a
  }

  mutating func statement(into chart: inout FlowChart) -> Bool {
    guard var left = group(&chart) else { return false }
    while true {
      skipSpaces()
      guard current != nil else { return true }
      guard let link = link() else { return false }
      guard let right = group(&chart) else { return false }
      for a in left {
        for b in right {
          chart.edges.append(FlowChart.Edge(from: a, to: b, label: link.label, length: link.length, stroke: link.stroke,
                                            start: link.start, end: link.end))
        }
      }
      left = right
    }
  }

  /// `a & b[...]`
  mutating func group(_ chart: inout FlowChart) -> [String]? {
    var ids: [String] = []
    while true {
      skipSpaces()
      guard let id = identifier() else { return nil }
      let (label, shape) = shapeText()
      // `:::class` is styling only
      if lookahead(":::") {
        at += 3
        while let c = current, c.isLetter || c.isNumber || c == "_" || c == "-" { at += 1 }
      }
      chart.vertex(id, label: label, shape: shape)
      ids.append(id)
      skipSpaces()
      guard current == "&" else { return ids }
      at += 1
    }
  }

  mutating func identifier() -> String? {
    var id = ""
    while let c = current, c.isLetter || c.isNumber || c == "_" || c == "$" {
      id.append(c)
      at += 1
    }
    return id.isEmpty ? nil : id
  }

  /// A node's shape and text, if any; the openers are tried longest first
  mutating func shapeText() -> (String?, FlowChart.Shape?) {
    let forms: [(String, String, FlowChart.Shape)] = [
      ("(((", ")))", .circle), ("((", "))", .circle), ("([", "])", .stadium), ("[[", "]]", .subroutine),
      ("[(", ")]", .rect), ("[/", "/]", .rect), ("[\\", "\\]", .rect), ("[/", "\\]", .rect), ("[\\", "/]", .rect),
      ("{{", "}}", .hexagon), ("[", "]", .rect), ("(", ")", .round), ("{", "}", .diamond), (">", "]", .rect),
    ]
    for (open, close, shape) in forms where lookahead(open) {
      let start = at
      at += open.count
      if let text = text(until: close) { return (text, shape) }
      at = start
    }
    return (nil, nil)
  }

  /// Text up to a closing sequence; a quoted string may contain the closer
  mutating func text(until close: String) -> String? {
    skipSpaces()
    var out = ""
    if current == "\"" {
      at += 1
      while let c = current, c != "\"" { out.append(c); at += 1 }
      guard current == "\"" else { return nil }
      at += 1
      skipSpaces()
      guard lookahead(close) else { return nil }
      at += close.count
      return FlowScanner.clean(out)
    }
    while current != nil {
      if lookahead(close) {
        at += close.count
        return FlowScanner.clean(out)
      }
      out.append(current!)
      at += 1
    }
    return nil
  }

  /// `<br>` breaks a line; entities mermaid decodes; surrounding spaces trimmed
  static func clean(_ s: String) -> String {
    var t = s.replacingOccurrences(of: "<br\\s*/?>", with: "\n", options: [.regularExpression, .caseInsensitive])
    for (entity, char) in [("#quot;", "\""), ("&quot;", "\""), ("&amp;", "&"), ("&lt;", "<"), ("&gt;", ">"), ("#35;", "#")] {
      t = t.replacingOccurrences(of: entity, with: char)
    }
    return t.split(separator: "\n", omittingEmptySubsequences: false)
      .map { $0.trimmingCharacters(in: .whitespaces) }
      .joined(separator: "\n")
  }

  struct Link {
    var label = ""
    var length = 1
    var stroke = FlowChart.Stroke.normal
    var start = FlowChart.Head.none
    var end = FlowChart.Head.point
  }

  /// mermaid's LINK / START_LINK tokens and destructLink
  mutating func link() -> Link? {
    skipSpaces()
    let rest = String(chars[at...])
    let endLink = #"^([xo<]?)(--+[-xo>]|==+[=xo>]|-?\.+-[xo>]?|~~~+)"#
    let startLink = #"^([xo<]?)(--|==|-\.)\s"#
    if let m = rest.firstMatch(of: try! Regex(endLink)), let token = m.output[0].substring {
      at += token.count
      var link = destruct(String(token))
      skipSpaces()
      // `-->|label|`
      if current == "|" {
        at += 1
        var label = ""
        while let c = current, c != "|" { label.append(c); at += 1 }
        guard current == "|" else { return nil }
        at += 1
        link.label = FlowScanner.clean(label)
      }
      return link
    }
    if let m = rest.firstMatch(of: try! Regex(startLink)), let token = m.output[0].substring {
      // `-- text -->`: the text runs to the first closing link token of the same stroke
      at += token.count
      let opener = String(token).trimmingCharacters(in: .whitespaces)
      let closers = opener.hasSuffix("==") ? #"==+[=xo>]"# : opener.hasSuffix("-.") ? #"\.+-[xo>]?"# : #"--+[-xo>]"#
      let tail = String(chars[at...])
      guard let close = tail.firstMatch(of: try! Regex("\\s*(" + closers + ")")),
            let closeToken = close.output[1].substring,
            let whole = close.output[0].substring else { return nil }
      let label = String(tail[tail.startIndex..<whole.startIndex])
      at += tail.distance(from: tail.startIndex, to: whole.endIndex)
      // The closing token alone carries the head and length; the opener's prefix adds a start head
      var link = destruct(opener.hasSuffix("-.") ? "-" + String(closeToken) : String(closeToken))
      if let first = opener.first, "xo<".contains(first) { link.start = head(first) }
      link.label = FlowScanner.clean(label)
      return link
    }
    return nil
  }

  func head(_ c: Character) -> FlowChart.Head {
    switch c {
    case ">", "<": .point
    case "x": .cross
    case "o": .circle
    default: .none
    }
  }

  /// flowDb.destructEndLink
  func destruct(_ token: String) -> Link {
    var link = Link()
    var str = token
    var line = String(str.dropLast())
    let last = str.last!
    link.end = "x>o".contains(last) ? head(last) : .none
    if "x>o".contains(last), let first = str.first, (first == "<" && last == ">") || (first == last && first != "-") {
      link.start = head(first)
      line = String(line.dropFirst())
      str = String(str.dropFirst())
    }
    link.length = max(1, line.count - 1)
    if line.hasPrefix("=") { link.stroke = .thick }
    if line.hasPrefix("~") { link.stroke = .invisible; link.end = .none }
    let dots = line.filter { $0 == "." }.count
    if dots > 0 {
      link.stroke = .dotted
      link.length = dots
    }
    return link
  }
}
