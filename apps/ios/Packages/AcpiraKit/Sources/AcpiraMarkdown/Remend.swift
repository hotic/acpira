import Foundation

// A port of remend 1.3.1, the incomplete-markdown repair streamdown 2.6 runs over a streaming reply before parsing
// (`mode="streaming"`, its default options: links in protocol mode, block KaTeX on, inline KaTeX off). An unclosed
// `**`, `` ` ``, `~~`, `$$` or link at the tail is closed so the partial text renders styled instead of showing its
// markers. Indices are UTF-16 offsets, as in JavaScript, so every rule matches the original character for character.

public enum Remend {
  /// The tail-repaired markdown a streaming block renders
  public static func repair(_ text: String) -> String {
    guard !text.isEmpty else { return text }
    var run = Run()
    var n = Array(text.utf16)
    // A single trailing space is dropped; two are a hard break
    if n.last == U.space && !(n.count >= 2 && n[n.count - 2] == U.space) { n.removeLast() }
    n = run.singleTilde(n)
    n = run.comparisonOperators(n)
    n = run.htmlTags(n)
    n = run.setextHeadings(n)
    n = run.links(n)
    if n.hasSuffix(U.units("](streamdown:incomplete-link)")) { return U.string(n) }
    n = run.boldItalic(n)
    n = run.bold(n)
    n = run.italicDoubleUnderscore(n)
    n = run.italicSingleAsterisk(n)
    n = run.italicSingleUnderscore(n)
    n = run.inlineCode(n)
    n = run.strikethrough(n)
    n = run.katex(n)
    return U.string(n)
  }

  /// The href remend gives a link whose URL has not arrived yet
  public static let incompleteLink = "streamdown:incomplete-link"
}

// MARK: - UTF-16 helpers

private typealias S = [UInt16]

private enum U {
  static let tick: UInt16 = 0x60, star: UInt16 = 0x2A, under: UInt16 = 0x5F, tilde: UInt16 = 0x7E
  static let backslash: UInt16 = 0x5C, dollar: UInt16 = 0x24, lbracket: UInt16 = 0x5B, rbracket: UInt16 = 0x5D
  static let lparen: UInt16 = 0x28, rparen: UInt16 = 0x29, lt: UInt16 = 0x3C, gt: UInt16 = 0x3E
  static let newline: UInt16 = 0x0A, space: UInt16 = 0x20, tab: UInt16 = 0x09, bang: UInt16 = 0x21
  static let slash: UInt16 = 0x2F

  static func units(_ s: String) -> S { Array(s.utf16) }
  static func string(_ s: S) -> String { String(decoding: s, as: UTF16.self) }

  /// JavaScript `substring`: clamped, swapped when reversed
  static func sub(_ s: S, _ a: Int, _ b: Int? = nil) -> S {
    var lo = max(0, min(a, s.count)), hi = max(0, min(b ?? s.count, s.count))
    if lo > hi { swap(&lo, &hi) }
    return Array(s[lo..<hi])
  }

  static func indexOf(_ s: S, _ needle: S, from: Int = 0) -> Int {
    guard needle.count <= s.count else { return -1 }
    var i = max(0, from)
    while i + needle.count <= s.count {
      if s[i..<(i + needle.count)].elementsEqual(needle) { return i }
      i += 1
    }
    return -1
  }

  static func lastIndexOf(_ s: S, _ needle: S) -> Int {
    guard needle.count <= s.count else { return -1 }
    var i = s.count - needle.count
    while i >= 0 {
      if s[i..<(i + needle.count)].elementsEqual(needle) { return i }
      i -= 1
    }
    return -1
  }

  static func includes(_ s: S, _ needle: S) -> Bool { indexOf(s, needle) != -1 }
  static func includes(_ s: S, _ c: UInt16) -> Bool { s.contains(c) }

  /// Non-overlapping occurrences, as `match(/x/g).length`
  static func count(_ s: S, _ needle: S) -> Int {
    var n = 0, i = 0
    while true {
      let j = indexOf(s, needle, from: i)
      if j == -1 { return n }
      n += 1; i = j + needle.count
    }
  }

  /// JavaScript's whitespace for `trim` / `trimEnd`
  static func jsSpace(_ c: UInt16) -> Bool {
    switch c {
    case 0x09...0x0D, 0x20, 0xA0, 0x1680, 0x2000...0x200A, 0x2028, 0x2029, 0x202F, 0x205F, 0x3000, 0xFEFF: true
    default: false
    }
  }

  static func trimEnd(_ s: S) -> S {
    var end = s.count
    while end > 0 && jsSpace(s[end - 1]) { end -= 1 }
    return Array(s[..<end])
  }

  static func trim(_ s: S) -> S {
    var start = 0
    while start < s.count && jsSpace(s[start]) { start += 1 }
    return trimEnd(Array(s[start...]))
  }

  /// `k`: a space, tab or newline
  static func blank(_ c: UInt16?) -> Bool { c == space || c == tab || c == newline }

  /// `f`: a word character (`[\p{L}\p{N}_]` over one UTF-16 unit; a lone surrogate is not one)
  static func word(_ c: UInt16?) -> Bool {
    guard let c else { return false }
    if (0x30...0x39).contains(c) || (0x41...0x5A).contains(c) || (0x61...0x7A).contains(c) || c == under { return true }
    guard let scalar = Unicode.Scalar(c) else { return false }
    switch scalar.properties.generalCategory {
    case .uppercaseLetter, .lowercaseLetter, .titlecaseLetter, .modifierLetter, .otherLetter,
         .decimalNumber, .letterNumber, .otherNumber:
      return true
    default:
      return false
    }
  }
}

extension Array where Element == UInt16 {
  fileprivate subscript(js i: Int) -> UInt16? { i >= 0 && i < count ? self[i] : nil }
  fileprivate func hasSuffix(_ s: [UInt16]) -> Bool { count >= s.count && self[(count - s.count)...].elementsEqual(s) }
  fileprivate func hasPrefix(_ s: [UInt16]) -> Bool { count >= s.count && self[..<s.count].elementsEqual(s) }
}

/// A compiled pattern matched over UTF-16 units; `$` in the source patterns is JavaScript's end of input (`\z`)
private struct Pattern {
  let regex: NSRegularExpression
  init(_ pattern: String, lines: Bool = false) {
    regex = try! NSRegularExpression(pattern: pattern, options: lines ? [.anchorsMatchLines] : [])
  }

  /// The first match's capture ranges (`nil` for a group that did not take part)
  func match(_ s: S) -> [NSRange?]? {
    let ns = U.string(s) as NSString
    guard let m = regex.firstMatch(in: ns as String, range: NSRange(location: 0, length: ns.length)) else { return nil }
    return (0..<m.numberOfRanges).map { m.range(at: $0).location == NSNotFound ? nil : m.range(at: $0) }
  }

  func test(_ s: S) -> Bool { match(s) != nil }

  /// Every match, for `replace(/…/g, fn)`
  func matches(_ s: S) -> [NSTextCheckingResult] {
    let ns = U.string(s) as NSString
    return regex.matches(in: ns as String, range: NSRange(location: 0, length: ns.length))
  }

  static func group(_ s: S, _ r: NSRange?) -> S { r.map { Array(s[$0.location..<($0.location + $0.length)]) } ?? [] }
}

private enum P {
  static let comparison = Pattern(#"^(\s*(?:[-*+]|\d+[.)]) +)>(=?\s*[$]?\d)"#, lines: true)
  static let bold = Pattern(#"(\*\*)([^*]*\*?)\z"#)
  static let doubleUnderscore = Pattern(#"(__)([^_]*?)\z"#)
  static let boldItalic = Pattern(#"(\*\*\*)([^*]*?)\z"#)
  static let singleAsterisk = Pattern(#"(\*)([^*]*?)\z"#)
  static let singleUnderscore = Pattern(#"(_)([^_]*?)\z"#)
  static let inlineCode = Pattern(#"(`)([^`]*?)\z"#)
  static let strike = Pattern(#"(~~)([^~]*?)\z"#)
  static let markersOnly = Pattern(#"\A[\s_~*`]*\z"#)
  static let bareListItem = Pattern(#"\A[\s]*[-*+][\s]+\z"#)
  static let oneLineFence = Pattern("\\A```[^`\\n]*```?\\z")
  static let starRule = Pattern(#"\A\*{4,}\z"#)
  static let underscoreTail = Pattern(#"(__)([^_]+)_\z"#)
  static let strikeTail = Pattern(#"(~~)([^~]+)~\z"#)
  static let htmlTail = Pattern(#"<[a-zA-Z/][^>]*\z"#)
  static let tilde = Pattern(#"([\p{L}\p{N}_])~(?!~)(?=[\p{L}\p{N}_])"#)
  static let dashes = Pattern(#"\A-{1,2}\z"#)
  static let dashItem = Pattern(#"\A[\s]*-{1,2}[\s]+\z"#)
  static let equals = Pattern(#"\A={1,2}\z"#)
  static let equalsItem = Pattern(#"\A[\s]*={1,2}[\s]+\z"#)
}

// MARK: - Code and math context

private struct Run {
  /// `c`'s lookup for the last text it saw
  private var cachedText: S?
  private var cachedLookup: [UInt8] = []

  /// `kn`: for each offset, whether it sits inside inline code or a fence
  private static func codeLookup(_ n: S) -> [UInt8] {
    var r = [UInt8](repeating: 0, count: n.count + 1)
    var inline = false, fence = false, o = 0
    while o < n.count {
      if n[o] == U.backslash && o + 1 < n.count && n[o + 1] == U.tick {
        let s: UInt8 = inline || fence ? 1 : 0
        r[o + 1] = s
        if o + 2 < r.count { r[o + 2] = s }
        o += 2
        continue
      }
      if U.sub(n, o, o + 3) == U.units("```") {
        fence.toggle()
        let s: UInt8 = inline || fence ? 1 : 0
        let t = min(o + 3, n.count)
        for a in (o + 1)...t { r[a] = s }
        o = t
        continue
      }
      if !fence && n[o] == U.tick { inline.toggle() }
      r[o + 1] = inline || fence ? 1 : 0
      o += 1
    }
    return r
  }

  /// `c`: is offset `r` inside code
  mutating func inCode(_ n: S, _ r: Int) -> Bool {
    if cachedText != n {
      cachedText = n
      cachedLookup = Run.codeLookup(n)
    }
    return cachedLookup[min(r, n.count)] == 1
  }

  /// `In`: part of a triple backtick
  static func partOfFence(_ n: S, _ r: Int) -> Bool {
    let f = U.units("```")
    return U.sub(n, r, r + 3) == f || (r > 0 && U.sub(n, r - 1, r + 2) == f) || (r > 1 && U.sub(n, r - 2, r + 1) == f)
  }

  /// `E`: unescaped single backticks
  static func singleBackticks(_ n: S) -> Int {
    var r = 0, e = 0
    while e < n.count {
      if n[e] == U.backslash && e + 1 < n.count && n[e + 1] == U.tick { e += 2; continue }
      if n[e] == U.tick && !partOfFence(n, e) { r += 1 }
      e += 1
    }
    return r
  }

  /// `d`: is offset `r` strictly inside a complete inline code span
  static func insideInlineCode(_ n: S, _ r: Int) -> Bool {
    var open = false, fence = false, start = -1, s = 0
    while s < n.count {
      if n[s] == U.backslash && s + 1 < n.count && n[s + 1] == U.tick { s += 2; continue }
      if U.sub(n, s, s + 3) == U.units("```") { fence.toggle(); s += 3; continue }
      if !fence && n[s] == U.tick {
        if open {
          if start < r && r < s { return true }
          open = false; start = -1
        } else {
          open = true; start = s
        }
      }
      s += 1
    }
    return false
  }

  enum MathState { case none, inlineLatex, blockLatex, inlineDollar, blockDollar }

  /// `h`: is offset `r` inside `$…$`, `$$…$$`, `\(…\)` or `\[…\]`
  static func inMath(_ n: S, _ r: Int) -> Bool {
    var e = MathState.none, i = 0
    while i < n.count && i < r {
      if n[i] == U.backslash && n[js: i + 1] == U.dollar { i += 2; continue }
      if n[i] == U.backslash {
        let next = n[js: i + 1]
        var o: MathState?
        if next == U.lbracket && e == .none { o = .blockLatex }
        else if next == U.rbracket && e == .blockLatex { o = MathState.none }
        else if next == U.lparen && e == .none { o = .inlineLatex }
        else if next == U.rparen && e == .inlineLatex { o = MathState.none }
        if let o { e = o; i += 2; continue }
      }
      if n[i] == U.dollar && !(e == .inlineLatex || e == .blockLatex) {
        let double = n[js: i + 1] == U.dollar
        if double { e = e == .blockDollar ? .none : .blockDollar }
        else if e != .blockDollar { e = e == .inlineDollar ? .none : .inlineDollar }
        if double { i += 1 }
      }
      i += 1
    }
    return e != .none
  }

  /// `Z`
  static func hasMath(_ n: S) -> Bool {
    n.contains(U.dollar) || U.includes(n, U.units("\\(")) || U.includes(n, U.units("\\["))
  }

  /// `m`: inside a link / image URL `](…)`
  static func inLinkURL(_ n: S, _ r: Int) -> Bool {
    var e = r - 1
    while e >= 0 {
      if n[e] == U.rparen { return false }
      if n[e] == U.lparen {
        guard e > 0 && n[e - 1] == U.rbracket else { return false }
        var k = r
        while k < n.count {
          if n[k] == U.rparen { return true }
          if n[k] == U.newline { return false }
          k += 1
        }
        return false
      }
      if n[e] == U.newline { return false }
      e -= 1
    }
    return false
  }

  /// `j`: inside an HTML tag
  static func inHTMLTag(_ n: S, _ r: Int) -> Bool {
    var e = r - 1
    while e >= 0 {
      if n[e] == U.gt { return false }
      if n[e] == U.lt {
        guard let i = n[js: e + 1] else { return false }
        return (0x61...0x7A).contains(i) || (0x41...0x5A).contains(i) || i == U.slash
      }
      if n[e] == U.newline { return false }
      e -= 1
    }
    return false
  }

  /// `p`: the line holding `r` is a thematic break of `c` (three or more, nothing else but spaces)
  static func ruleLine(_ n: S, _ r: Int, _ c: UInt16) -> Bool {
    var start = 0, end = n.count
    var l = r - 1
    while l >= 0 { if n[l] == U.newline { start = l + 1; break }; l -= 1 }
    l = r
    while l < n.count { if n[l] == U.newline { end = l; break }; l += 1 }
    var t = 0
    for ch in n[start..<end] {
      if ch == c { t += 1 } else if ch != U.space && ch != U.tab { return false }
    }
    return t >= 3
  }

  /// Toggles a fence at a triple backtick starting at `i`
  static func fenceAt(_ n: S, _ i: Int) -> Bool {
    n[i] == U.tick && i + 2 < n.count && n[i + 1] == U.tick && n[i + 2] == U.tick
  }

  // MARK: Counting

  /// `L`: `**` pairs outside fences
  static func doubleStars(_ n: S) -> Int { pairs(n, U.star) }
  /// `Q`: `__` pairs outside fences
  static func doubleUnderscores(_ n: S) -> Int { pairs(n, U.under) }

  private static func pairs(_ n: S, _ c: UInt16) -> Int {
    var r = 0, fence = false, i = 0
    while i < n.count {
      if fenceAt(n, i) { fence.toggle(); i += 3; continue }
      if !fence && n[i] == c && i + 1 < n.count && n[i + 1] == c { r += 1; i += 2; continue }
      i += 1
    }
    return r
  }

  /// `$n`: runs of three asterisks outside fences
  static func tripleStars(_ n: S) -> Int {
    var r = 0, e = 0, fence = false, o = 0
    while o < n.count {
      if fenceAt(n, o) {
        if e >= 3 { r += e / 3 }
        e = 0; fence.toggle(); o += 3
        continue
      }
      if !fence {
        if n[o] == U.star { e += 1 } else { if e >= 3 { r += e / 3 }; e = 0 }
      }
      o += 1
    }
    if e >= 3 { r += e / 3 }
    return r
  }

  /// `v`: single asterisks that open or close emphasis
  static func singleStars(_ n: S) -> Int {
    var r = 0, fence = false, chain = false, s = 0
    while s < n.count {
      if fenceAt(n, s) { fence.toggle(); s += 3; continue }
      if fence { s += 1; continue }
      if n[s] != U.star {
        if !U.word(n[s]) { chain = false }
        s += 1
        continue
      }
      let t = n[js: s - 1], a = n[js: s + 1]
      if skipStar(n, s, t, a) { s += 1; continue }
      // `_n`
      let both = U.word(t) && U.word(a)
      let after = a != nil && !U.blank(a), before = t != nil && !U.blank(t)
      if both && r % 2 == 0 && !chain {
        // not counted
      } else if (before && r % 2 == 1) || after {
        r += 1; chain = both
      }
      s += 1
    }
    return r
  }

  /// `Sn`
  static func skipStar(_ n: S, _ r: Int, _ e: UInt16?, _ i: UInt16?) -> Bool {
    if e == U.backslash || (hasMath(n) && inMath(n, r)) { return true }
    if e != U.star && i == U.star { return n[js: r + 2] != U.star }
    return e == U.star || ((e == nil || U.blank(e)) && (i == nil || U.blank(i)))
  }

  /// `Pn`: underscores that open or close emphasis
  static func singleUnderscores(_ n: S) -> Int {
    var r = 0, fence = false, o = 0
    while o < n.count {
      if fenceAt(n, o) { fence.toggle(); o += 3; continue }
      if fence || n[o] != U.under { o += 1; continue }
      let s = n[js: o - 1], t = n[js: o + 1]
      let skip = s == U.backslash || (hasMath(n) && inMath(n, o)) || inLinkURL(n, o) || inHTMLTag(n, o)
        || s == U.under || t == U.under || (U.word(s) && U.word(t))
      if !skip { r += 1 }
      o += 1
    }
    return r
  }

  /// `En` / `yn`: an emphasis tail that needs no closing
  static func closedEnough(_ n: S, _ tail: S, _ at: Int, _ c: UInt16) -> Bool {
    if tail.isEmpty || P.markersOnly.test(tail) { return true }
    let before = U.sub(n, 0, at)
    let nl = U.lastIndexOf(before, [U.newline])
    let line = U.sub(n, nl == -1 ? 0 : nl + 1, at)
    if P.bareListItem.test(line) && tail.contains(U.newline) { return true }
    return ruleLine(n, at, c)
  }

  // MARK: Handlers (in remend's priority order)

  /// `un`: `a~b` is not a strikethrough opener
  mutating func singleTilde(_ n: S) -> S {
    guard n.contains(U.tilde) else { return n }
    var out = S(), last = 0
    for m in P.tilde.matches(n) {
      let g1 = m.range(at: 1)
      let o = g1.location + g1.length
      out += n[last..<m.range.location]
      if inCode(n, o) {
        out += n[m.range.location..<(m.range.location + m.range.length)]
      } else {
        out += n[g1.location..<o] + U.units("\\~")
      }
      last = m.range.location + m.range.length
    }
    out += n[last...]
    return out
  }

  /// `y`: `- >5` in a list item is not a quote
  mutating func comparisonOperators(_ n: S) -> S {
    guard n.contains(U.gt) else { return n }
    var out = S(), last = 0
    for m in P.comparison.matches(n) {
      out += n[last..<m.range.location]
      if inCode(n, m.range.location) {
        out += n[m.range.location..<(m.range.location + m.range.length)]
      } else {
        out += Pattern.group(n, m.range(at: 1)) + U.units("\\>") + Pattern.group(n, m.range(at: 2))
      }
      last = m.range.location + m.range.length
    }
    out += n[last...]
    return out
  }

  /// `rn`: a tag still being written is held back
  mutating func htmlTags(_ n: S) -> S {
    guard let m = P.htmlTail.match(n), let r = m[0], !inCode(n, r.location) else { return n }
    return U.trimEnd(U.sub(n, 0, r.location))
  }

  /// `cn`: a lone `-` / `=` under a line must not turn it into a setext heading yet
  mutating func setextHeadings(_ n: S) -> S {
    let r = U.lastIndexOf(n, [U.newline])
    guard r != -1 else { return n }
    let e = U.sub(n, r + 1), before = U.sub(n, 0, r), o = U.trim(e)
    let prev = before.split(separator: U.newline, omittingEmptySubsequences: false).last.map(Array.init) ?? []
    if P.dashes.test(o) && !P.dashItem.test(e) && !U.trim(prev).isEmpty { return n + U.units("\u{200B}") }
    if P.equals.test(o) && !P.equalsItem.test(e) && !U.trim(prev).isEmpty { return n + U.units("\u{200B}") }
    return n
  }

  /// `B` (protocol mode): an unfinished link points at the incomplete-link href; an unfinished image is dropped
  mutating func links(_ n: S) -> S {
    let e = U.lastIndexOf(n, U.units("]("))
    if e != -1 && !inCode(n, e), let i = linkURLTail(n, e) { return i }
    var i = n.count - 1
    while i >= 0 {
      if n[i] == U.lbracket && !inCode(n, i), let o = linkTextTail(n, i) { return o }
      i -= 1
    }
    return n
  }

  /// `zn`
  private mutating func linkURLTail(_ n: S, _ r: Int) -> S? {
    if U.sub(n, r + 2).contains(U.rparen) { return nil }
    // `Y`: the `[` matching the `]` at r
    var depth = 1, o = -1, k = r - 1
    while k >= 0 {
      if n[k] == U.rbracket { depth += 1 } else if n[k] == U.lbracket { depth -= 1; if depth == 0 { o = k; break } }
      k -= 1
    }
    if o == -1 || inCode(n, o) { return nil }
    let image = o > 0 && n[o - 1] == U.bang
    let a = U.sub(n, 0, image ? o - 1 : o)
    if image { return a }
    return a + [U.lbracket] + U.sub(n, o + 1, r) + U.units("](\(Remend.incompleteLink))")
  }

  /// `Yn`
  private func linkTextTail(_ n: S, _ r: Int) -> S? {
    let image = r > 0 && n[r - 1] == U.bang
    let o = image ? r - 1 : r
    // `T`: the `]` matching the `[` at r
    var depth = 1, close = -1, k = r + 1
    while k < n.count {
      if n[k] == U.lbracket { depth += 1 } else if n[k] == U.rbracket { depth -= 1; if depth == 0 { close = k; break } }
      k += 1
    }
    if !U.sub(n, r + 1).contains(U.rbracket) || close == -1 {
      return image ? U.sub(n, 0, o) : n + U.units("](\(Remend.incompleteLink))")
    }
    return nil
  }

  /// `en`
  mutating func boldItalic(_ n: S) -> S {
    if P.starRule.test(n) { return n }
    guard let r = P.boldItalic.match(n) else { return n }
    let e = Pattern.group(n, r[2]), i = U.lastIndexOf(n, U.units("***"))
    if e.isEmpty || P.markersOnly.test(e) || inCode(n, i) || Run.insideInlineCode(n, i) || Run.ruleLine(n, i, U.star) {
      return n
    }
    guard Run.tripleStars(n) % 2 == 1 else { return n }
    return Run.doubleStars(n) % 2 == 0 && Run.singleStars(n) % 2 == 0 ? n : n + U.units("***")
  }

  /// `q`
  mutating func bold(_ n: S) -> S {
    guard let r = P.bold.match(n) else { return n }
    let e = Pattern.group(n, r[2]), i = U.lastIndexOf(n, U.units("**"))
    if inCode(n, i) || Run.insideInlineCode(n, i) || Run.closedEnough(n, e, i, U.star) { return n }
    guard Run.doubleStars(n) % 2 == 1 else { return n }
    return e.last == U.star ? n + [U.star] : n + U.units("**")
  }

  /// `J`
  mutating func italicDoubleUnderscore(_ n: S) -> S {
    guard let r = P.doubleUnderscore.match(n) else {
      if P.underscoreTail.test(n) {
        let t = U.lastIndexOf(n, U.units("__"))
        if !(inCode(n, t) || Run.insideInlineCode(n, t)) && Run.doubleUnderscores(n) % 2 == 1 { return n + [U.under] }
      }
      return n
    }
    let e = Pattern.group(n, r[2]), i = U.lastIndexOf(n, U.units("__"))
    if inCode(n, i) || Run.insideInlineCode(n, i) || Run.closedEnough(n, e, i, U.under) { return n }
    return Run.doubleUnderscores(n) % 2 == 1 ? n + U.units("__") : n
  }

  /// `Nn`: the first single `*` that opens emphasis
  static func singleStarOpener(_ n: S) -> Int {
    var fence = false, e = 0
    while e < n.count {
      if fenceAt(n, e) { fence.toggle(); e += 3; continue }
      if !fence && n[e] == U.star && n[js: e - 1] != U.star && n[js: e + 1] != U.star && n[js: e - 1] != U.backslash
        && !inMath(n, e) {
        let i = n[js: e - 1], o = n[js: e + 1]
        let s = i == nil || U.blank(i), t = o == nil || U.blank(o)
        if !((s && t) || (U.word(i) && U.word(o)) || t) { return e }
      }
      e += 1
    }
    return -1
  }

  /// `V`
  mutating func italicSingleAsterisk(_ n: S) -> S {
    guard P.singleAsterisk.test(n) else { return n }
    let e = Run.singleStarOpener(n)
    if e == -1 || inCode(n, e) || Run.insideInlineCode(n, e) { return n }
    let i = U.sub(n, e + 1)
    if i.isEmpty || P.markersOnly.test(i) { return n }
    return Run.singleStars(n) % 2 == 1 ? n + [U.star] : n
  }

  /// `x`: the first single `_` that opens emphasis
  static func singleUnderscoreOpener(_ n: S) -> Int {
    var fence = false, e = 0
    while e < n.count {
      if fenceAt(n, e) { fence.toggle(); e += 3; continue }
      if !fence && n[e] == U.under && n[js: e - 1] != U.under && n[js: e + 1] != U.under
        && n[js: e - 1] != U.backslash && !inMath(n, e) && !inLinkURL(n, e) {
        let i = n[js: e - 1], o = n[js: e + 1]
        if !(U.word(i) && U.word(o)) { return e }
      }
      e += 1
    }
    return -1
  }

  /// `nn`
  mutating func italicSingleUnderscore(_ n: S) -> S {
    guard P.singleUnderscore.test(n) else { return n }
    let e = Run.singleUnderscoreOpener(n)
    if e == -1 { return n }
    let i = U.sub(n, e + 1)
    if i.isEmpty || P.markersOnly.test(i) || inCode(n, e) || Run.insideInlineCode(n, e) { return n }
    guard Run.singleUnderscores(n) % 2 == 1 else { return n }
    // `Un`: `**bold _italic**` closes the italic inside the bold
    if n.hasSuffix(U.units("**")) {
      let r = Array(n.dropLast(2))
      if Run.doubleStars(r) % 2 == 1 {
        let b = U.indexOf(r, U.units("**")), o = Run.singleUnderscoreOpener(r)
        if b != -1 && o != -1 && b < o { return r + U.units("_**") }
      }
    }
    // `Rn`: before any trailing newlines
    var end = n.count
    while end > 0 && n[end - 1] == U.newline { end -= 1 }
    return Array(n[..<end]) + [U.under] + Array(n[end...])
  }

  /// `on`
  mutating func inlineCode(_ n: S) -> S {
    if P.oneLineFence.test(n) && !n.contains(U.newline) {
      return n.hasSuffix(U.units("``")) && !n.hasSuffix(U.units("```")) ? n + [U.tick] : n
    }
    if let e = P.inlineCode.match(n), U.count(n, U.units("```")) % 2 == 0 {
      let i = Pattern.group(n, e[2])
      if i.isEmpty || P.markersOnly.test(i) { return n }
      if Run.singleBackticks(n) % 2 == 1 { return n + [U.tick] }
    }
    return n
  }

  /// `fn`
  mutating func strikethrough(_ n: S) -> S {
    if let r = P.strike.match(n) {
      let o = Pattern.group(n, r[2])
      if o.isEmpty || P.markersOnly.test(o) { return n }
      let s = U.lastIndexOf(n, U.units("~~"))
      if inCode(n, s) || Run.insideInlineCode(n, s) { return n }
      if U.count(n, U.units("~~")) % 2 == 1 { return n + U.units("~~") }
    } else if P.strikeTail.test(n) {
      let s = U.lastIndexOf(n, U.units("~~"))
      if inCode(n, s) || Run.insideInlineCode(n, s) { return n }
      if U.count(n, U.units("~~")) % 2 == 1 { return n + [U.tilde] }
    }
    return n
  }

  /// `sn`
  static func fenceTick(_ n: S, _ r: Int) -> Bool { partOfFence(n, r) }

  /// `tn`: an open `$$` block is closed
  mutating func katex(_ n: S) -> S {
    // `Gn`: `$$` outside inline code
    var r = 0, code = false, i = 0
    while i < n.count - 1 {
      if n[i] == U.tick && !Run.fenceTick(n, i) { code.toggle() }
      if !code && n[i] == U.dollar && n[i + 1] == U.dollar { r += 1; i += 1 }
      i += 1
    }
    guard r % 2 == 1 else { return n }
    // `Xn`
    if n.last == U.dollar && !n.hasSuffix(U.units("$$")) { return n + [U.dollar] }
    let open = U.indexOf(n, U.units("$$"))
    if open != -1 && U.indexOf(n, [U.newline], from: open) != -1 && n.last != U.newline {
      return n + U.units("\n$$")
    }
    return n + U.units("$$")
  }
}
