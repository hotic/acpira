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
    if n.hasSuffix(Tokens.incompleteLinkSuffix) { return U.string(n) }
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

private enum Tokens {
  static let incompleteLinkSuffix = U.units("](streamdown:incomplete-link)")
  static let incompleteLinkAppend = U.units("](\(Remend.incompleteLink))")
  static let linkMid = U.units("](")
  static let tripleStar = U.units("***")
  static let doubleStar = U.units("**")
  static let doubleUnder = U.units("__")
  static let doubleTilde = U.units("~~")
  static let doubleDollar = U.units("$$")
  static let tripleTick = U.units("```")
  static let doubleTick = U.units("``")
  static let escapedTilde = U.units("\\~")
  static let escapedGt = U.units("\\>")
  static let zwsp = U.units("\u{200B}")
  static let underBold = U.units("_**")
  static let nlDoubleDollar = U.units("\n$$")
  static let inlineMathOpen = U.units("\\(")
  static let blockMathOpen = U.units("\\[")
}

/// Fast UTF-16 character-class and suffix matchers replacing per-call NSString + NSRegularExpression scans.
/// Every matcher mirrors its original JS/ICU pattern over UTF-16 code units.
private enum Scan {
  @inline(__always) static func isSpace(_ c: UInt16) -> Bool {
    c == 0x20 || (0x09...0x0D).contains(c) || c == 0x85 || c == 0xA0 || c == 0x1680
      || (0x2000...0x200A).contains(c) || c == 0x2028 || c == 0x2029 || c == 0x202F || c == 0x205F || c == 0x3000
  }

  @inline(__always) static func isDigit(_ c: UInt16) -> Bool { (0x30...0x39).contains(c) }
  @inline(__always) static func isASCIIAlpha(_ c: UInt16) -> Bool { (0x41...0x5A).contains(c) || (0x61...0x7A).contains(c) }

  /// `\A[\s_~*`]*\z`
  static func isMarkersOnly(_ s: S, from start: Int = 0) -> Bool {
    var i = start
    while i < s.count {
      let c = s[i]
      if c != U.under && c != U.tilde && c != U.star && c != U.tick && !isSpace(c) { return false }
      i += 1
    }
    return true
  }

  /// `\A[\s]*[-*+][\s]+\z`
  static func isBareListItem(_ s: S, from start: Int, to end: Int) -> Bool {
    var i = start
    while i < end && isSpace(s[i]) { i += 1 }
    guard i < end else { return false }
    let m = s[i]
    guard m == 0x2D || m == U.star || m == 0x2B else { return false }
    i += 1
    guard i < end && isSpace(s[i]) else { return false }
    while i < end && isSpace(s[i]) { i += 1 }
    return i == end
  }

  /// `\A```[^`\n]*```?\z`
  static func isOneLineFence(_ s: S) -> Bool {
    guard s.count >= 5, s[0] == U.tick, s[1] == U.tick, s[2] == U.tick else { return false }
    // Close is 2 or 3 trailing backticks
    let closeLen: Int
    if s.count >= 6 && s[s.count - 1] == U.tick && s[s.count - 2] == U.tick && s[s.count - 3] == U.tick {
      closeLen = 3
    } else if s[s.count - 1] == U.tick && s[s.count - 2] == U.tick {
      closeLen = 2
    } else {
      return false
    }
    let bodyEnd = s.count - closeLen
    guard bodyEnd >= 3 else { return false }
    for i in 3..<bodyEnd {
      if s[i] == U.tick || s[i] == U.newline { return false }
    }
    return true
  }

  /// `\A\*{4,}\z`
  static func isStarRule(_ s: S) -> Bool {
    guard s.count >= 4 else { return false }
    for c in s { if c != U.star { return false } }
    return true
  }

  /// `\A-{1,2}\z` or `\A={1,2}\z`
  static func isOneOrTwo(_ s: S, of ch: UInt16) -> Bool {
    (s.count == 1 && s[0] == ch) || (s.count == 2 && s[0] == ch && s[1] == ch)
  }

  /// `\A[\s]*<ch>{1,2}[\s]+\z`
  static func isRepeatedItem(_ s: S, of ch: UInt16) -> Bool {
    var i = 0
    while i < s.count && isSpace(s[i]) { i += 1 }
    var n = 0
    while i < s.count && s[i] == ch { i += 1; n += 1 }
    guard n == 1 || n == 2 else { return false }
    guard i < s.count && isSpace(s[i]) else { return false }
    while i < s.count && isSpace(s[i]) { i += 1 }
    return i == s.count
  }

  /// `<[a-zA-Z/][^>]*\z` -> returns `<` index
  static func htmlTailStart(_ s: S) -> Int? {
    var i = s.count - 1
    while i >= 0 {
      let c = s[i]
      if c == U.gt { return nil }
      if c == U.lt {
        if i + 1 < s.count, let next = s[js: i + 1], isASCIIAlpha(next) || next == U.slash { return i }
        // A `<` not followed by [a-zA-Z/] cannot match, and an outer `<` can still match across it (not `>`).
      }
      i -= 1
    }
    return nil
  }

  /// `(\*\*)([^*]*\*?)\z` -> returns start of group 2 (right after `**`), or nil
  static func boldTail(_ s: S) -> Int? {
    guard s.count >= 2 else { return nil }
    var i = s.count
    if s[i - 1] == U.star { i -= 1 }
    while i > 0 && s[i - 1] != U.star { i -= 1 }
    guard i >= 2 && s[i - 1] == U.star && s[i - 2] == U.star else { return nil }
    return i
  }

  /// `(\*\*\*)([^*]*?)\z` -> returns start of group 2 (right after `***`), or nil
  static func boldItalicTail(_ s: S) -> Int? {
    guard s.count >= 3 else { return nil }
    var i = s.count
    while i > 0 && s[i - 1] != U.star { i -= 1 }
    guard i >= 3 && s[i - 1] == U.star && s[i - 2] == U.star && s[i - 3] == U.star else { return nil }
    return i
  }

  /// `(__)([^_]*?)\z` -> returns start of group 2 (right after `__`), or nil
  static func doubleUnderscoreTail(_ s: S) -> Int? {
    guard s.count >= 2 else { return nil }
    var i = s.count
    while i > 0 && s[i - 1] != U.under { i -= 1 }
    guard i >= 2 && s[i - 1] == U.under && s[i - 2] == U.under else { return nil }
    return i
  }

  /// `(__)([^_]+)_\z`
  static func hasUnderscoreTail(_ s: S) -> Bool {
    guard s.count >= 4, s[s.count - 1] == U.under else { return false }
    var i = s.count - 1
    while i > 0 && s[i - 1] != U.under { i -= 1 }
    return i < s.count - 1 && i >= 2 && s[i - 1] == U.under && s[i - 2] == U.under
  }

  /// `` (`)([^`]*?)\z `` -> returns start of group 2 (right after `` ` ``), or nil
  static func inlineCodeTail(_ s: S) -> Int? {
    var i = s.count
    while i > 0 && s[i - 1] != U.tick { i -= 1 }
    guard i >= 1 && s[i - 1] == U.tick else { return nil }
    return i
  }

  /// `(~~)([^~]*?)\z` -> returns start of group 2 (right after `~~`), or nil
  static func strikeTail(_ s: S) -> Int? {
    guard s.count >= 2 else { return nil }
    var i = s.count
    while i > 0 && s[i - 1] != U.tilde { i -= 1 }
    guard i >= 2 && s[i - 1] == U.tilde && s[i - 2] == U.tilde else { return nil }
    return i
  }

  /// `(~~)([^~]+)~\z`
  static func hasSingleStrikeTail(_ s: S) -> Bool {
    guard s.count >= 4, s[s.count - 1] == U.tilde else { return false }
    var i = s.count - 1
    while i > 0 && s[i - 1] != U.tilde { i -= 1 }
    return i < s.count - 1 && i >= 2 && s[i - 1] == U.tilde && s[i - 2] == U.tilde
  }
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
      // `fenceAt` is the substring test without copying three units out for every offset
      if fenceAt(n, o) {
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
    fenceAt(n, r) || (r > 0 && fenceAt(n, r - 1)) || (r > 1 && fenceAt(n, r - 2))
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
      if fenceAt(n, s) { fence.toggle(); s += 3; continue }
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
    n.contains(U.dollar) || U.includes(n, Tokens.inlineMathOpen) || U.includes(n, Tokens.blockMathOpen)
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
    // Once per text rather than once per asterisk
    let math = hasMath(n)
    while s < n.count {
      if fenceAt(n, s) { fence.toggle(); s += 3; continue }
      if fence { s += 1; continue }
      if n[s] != U.star {
        if !U.word(n[s]) { chain = false }
        s += 1
        continue
      }
      let t = n[js: s - 1], a = n[js: s + 1]
      if skipStar(n, s, t, a, math: math) { s += 1; continue }
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
  static func skipStar(_ n: S, _ r: Int, _ e: UInt16?, _ i: UInt16?, math: Bool) -> Bool {
    if e == U.backslash || (math && inMath(n, r)) { return true }
    if e != U.star && i == U.star { return n[js: r + 2] != U.star }
    return e == U.star || ((e == nil || U.blank(e)) && (i == nil || U.blank(i)))
  }

  /// `Pn`: underscores that open or close emphasis
  static func singleUnderscores(_ n: S) -> Int {
    var r = 0, fence = false, o = 0
    let math = hasMath(n)
    while o < n.count {
      if fenceAt(n, o) { fence.toggle(); o += 3; continue }
      if fence || n[o] != U.under { o += 1; continue }
      let s = n[js: o - 1], t = n[js: o + 1]
      let skip = s == U.backslash || (math && inMath(n, o)) || inLinkURL(n, o) || inHTMLTag(n, o)
        || s == U.under || t == U.under || (U.word(s) && U.word(t))
      if !skip { r += 1 }
      o += 1
    }
    return r
  }

  /// `En` / `yn`: an emphasis tail that needs no closing
  static func closedEnough(_ n: S, _ tailStart: Int, _ at: Int, _ c: UInt16) -> Bool {
    if tailStart >= n.count || Scan.isMarkersOnly(n, from: tailStart) { return true }
    var nl = at - 1
    while nl >= 0 && n[nl] != U.newline { nl -= 1 }
    if Scan.isBareListItem(n, from: nl + 1, to: at) && n[tailStart...].contains(U.newline) { return true }
    return ruleLine(n, at, c)
  }

  // MARK: Handlers (in remend's priority order)

  /// `un`: `a~b` is not a strikethrough opener
  mutating func singleTilde(_ n: S) -> S {
    guard n.contains(U.tilde) else { return n }
    var out: S?
    var last = 0
    var i = 1
    while i + 1 < n.count {
      if n[i] == U.tilde && n[i + 1] != U.tilde && U.word(n[i - 1]) && U.word(n[i + 1]) {
        if !inCode(n, i) {
          if out == nil { out = S(); out!.reserveCapacity(n.count + 8) }
          out! += n[last..<i]
          out! += Tokens.escapedTilde
          last = i + 1
        }
        // Equivalent to regex consuming `([\p{L}\p{N}_])~` so scanning continues at `i + 1`
        i += 2
        continue
      }
      i += 1
    }
    guard var result = out else { return n }
    result += n[last...]
    return result
  }

  /// `y`: `- >5` in a list item is not a quote (`^(\s*(?:[-*+]|\d+[.)]) +)>(=?\s*[$]?\d)` multiline)
  mutating func comparisonOperators(_ n: S) -> S {
    guard n.contains(U.gt) else { return n }
    var out: S?
    var last = 0
    var lineStart = 0
    while lineStart < n.count {
      var lineEnd = lineStart
      while lineEnd < n.count && n[lineEnd] != U.newline { lineEnd += 1 }
      // Match lineStart..<lineEnd
      var i = lineStart
      while i < lineEnd && Scan.isSpace(n[i]) { i += 1 }
      var markerOK = false
      if i < lineEnd {
        let ch = n[i]
        if ch == 0x2D || ch == U.star || ch == 0x2B {
          i += 1
          markerOK = true
        } else if Scan.isDigit(ch) {
          while i < lineEnd && Scan.isDigit(n[i]) { i += 1 }
          if i < lineEnd && (n[i] == 0x2E || n[i] == U.rparen) {
            i += 1
            markerOK = true
          }
        }
      }
      if markerOK && i < lineEnd && n[i] == U.space {
        while i < lineEnd && n[i] == U.space { i += 1 }
        if i < lineEnd && n[i] == U.gt {
          let gt = i
          var k = gt + 1
          if k < lineEnd && n[k] == 0x3D { k += 1 }
          while k < lineEnd && Scan.isSpace(n[k]) { k += 1 }
          if k < lineEnd && n[k] == U.dollar { k += 1 }
          if k < lineEnd && Scan.isDigit(n[k]) && !inCode(n, lineStart) {
            if out == nil { out = S(); out!.reserveCapacity(n.count + 8) }
            out! += n[last..<gt]
            out! += Tokens.escapedGt
            last = gt + 1
          }
        }
      }
      lineStart = lineEnd + 1
    }
    guard var result = out else { return n }
    result += n[last...]
    return result
  }

  /// `rn`: a tag still being written is held back
  mutating func htmlTags(_ n: S) -> S {
    guard let at = Scan.htmlTailStart(n), !inCode(n, at) else { return n }
    return U.trimEnd(U.sub(n, 0, at))
  }

  /// `cn`: a lone `-` / `=` under a line must not turn it into a setext heading yet
  mutating func setextHeadings(_ n: S) -> S {
    let r = U.lastIndexOf(n, [U.newline])
    guard r != -1 else { return n }
    let e = U.sub(n, r + 1), o = U.trim(e)
    guard Scan.isOneOrTwo(o, of: 0x2D) || Scan.isOneOrTwo(o, of: 0x3D) else { return n }
    var prevStart = r - 1
    while prevStart >= 0 && n[prevStart] != U.newline { prevStart -= 1 }
    let prevHasNonSpace = ((prevStart + 1)..<r).contains { !U.jsSpace(n[$0]) }
    guard prevHasNonSpace else { return n }
    if Scan.isOneOrTwo(o, of: 0x2D) && !Scan.isRepeatedItem(e, of: 0x2D) { return n + Tokens.zwsp }
    if Scan.isOneOrTwo(o, of: 0x3D) && !Scan.isRepeatedItem(e, of: 0x3D) { return n + Tokens.zwsp }
    return n
  }

  /// `B` (protocol mode): an unfinished link points at the incomplete-link href; an unfinished image is dropped
  mutating func links(_ n: S) -> S {
    let e = U.lastIndexOf(n, Tokens.linkMid)
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
    if n[(r + 2)...].contains(U.rparen) { return nil }
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
    return a + [U.lbracket] + U.sub(n, o + 1, r) + Tokens.incompleteLinkAppend
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
    if close == -1 {
      return image ? U.sub(n, 0, o) : n + Tokens.incompleteLinkAppend
    }
    return nil
  }

  /// `en`
  mutating func boldItalic(_ n: S) -> S {
    if Scan.isStarRule(n) { return n }
    guard let eStart = Scan.boldItalicTail(n) else { return n }
    let i = U.lastIndexOf(n, Tokens.tripleStar)
    if eStart >= n.count || Scan.isMarkersOnly(n, from: eStart) || inCode(n, i) || Run.insideInlineCode(n, i)
      || Run.ruleLine(n, i, U.star) {
      return n
    }
    guard Run.tripleStars(n) % 2 == 1 else { return n }
    return Run.doubleStars(n) % 2 == 0 && Run.singleStars(n) % 2 == 0 ? n : n + Tokens.tripleStar
  }

  /// `q`
  mutating func bold(_ n: S) -> S {
    guard let eStart = Scan.boldTail(n) else { return n }
    let i = U.lastIndexOf(n, Tokens.doubleStar)
    if inCode(n, i) || Run.insideInlineCode(n, i) || Run.closedEnough(n, eStart, i, U.star) { return n }
    guard Run.doubleStars(n) % 2 == 1 else { return n }
    return n.last == U.star ? n + [U.star] : n + Tokens.doubleStar
  }

  /// `J`
  mutating func italicDoubleUnderscore(_ n: S) -> S {
    guard let eStart = Scan.doubleUnderscoreTail(n) else {
      if Scan.hasUnderscoreTail(n) {
        let t = U.lastIndexOf(n, Tokens.doubleUnder)
        if !(inCode(n, t) || Run.insideInlineCode(n, t)) && Run.doubleUnderscores(n) % 2 == 1 { return n + [U.under] }
      }
      return n
    }
    let i = U.lastIndexOf(n, Tokens.doubleUnder)
    if inCode(n, i) || Run.insideInlineCode(n, i) || Run.closedEnough(n, eStart, i, U.under) { return n }
    return Run.doubleUnderscores(n) % 2 == 1 ? n + Tokens.doubleUnder : n
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
    guard n.contains(U.star) else { return n }
    let e = Run.singleStarOpener(n)
    if e == -1 || inCode(n, e) || Run.insideInlineCode(n, e) { return n }
    if e + 1 >= n.count || Scan.isMarkersOnly(n, from: e + 1) { return n }
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
    guard n.contains(U.under) else { return n }
    let e = Run.singleUnderscoreOpener(n)
    if e == -1 { return n }
    if e + 1 >= n.count || Scan.isMarkersOnly(n, from: e + 1) || inCode(n, e) || Run.insideInlineCode(n, e) { return n }
    guard Run.singleUnderscores(n) % 2 == 1 else { return n }
    // `Un`: `**bold _italic**` closes the italic inside the bold
    if n.hasSuffix(Tokens.doubleStar) {
      let r = Array(n.dropLast(2))
      if Run.doubleStars(r) % 2 == 1 {
        let b = U.indexOf(r, Tokens.doubleStar), o = Run.singleUnderscoreOpener(r)
        if b != -1 && o != -1 && b < o { return r + Tokens.underBold }
      }
    }
    // `Rn`: before any trailing newlines
    var end = n.count
    while end > 0 && n[end - 1] == U.newline { end -= 1 }
    return Array(n[..<end]) + [U.under] + Array(n[end...])
  }

  /// `on`
  mutating func inlineCode(_ n: S) -> S {
    guard n.contains(U.tick) else { return n }
    if Scan.isOneLineFence(n) {
      return n.hasSuffix(Tokens.doubleTick) && !n.hasSuffix(Tokens.tripleTick) ? n + [U.tick] : n
    }
    if let iStart = Scan.inlineCodeTail(n), U.count(n, Tokens.tripleTick) % 2 == 0 {
      if iStart >= n.count || Scan.isMarkersOnly(n, from: iStart) { return n }
      if Run.singleBackticks(n) % 2 == 1 { return n + [U.tick] }
    }
    return n
  }

  /// `fn`
  mutating func strikethrough(_ n: S) -> S {
    guard n.contains(U.tilde) else { return n }
    if let oStart = Scan.strikeTail(n) {
      if oStart >= n.count || Scan.isMarkersOnly(n, from: oStart) { return n }
      let s = U.lastIndexOf(n, Tokens.doubleTilde)
      if inCode(n, s) || Run.insideInlineCode(n, s) { return n }
      if U.count(n, Tokens.doubleTilde) % 2 == 1 { return n + Tokens.doubleTilde }
    } else if Scan.hasSingleStrikeTail(n) {
      let s = U.lastIndexOf(n, Tokens.doubleTilde)
      if inCode(n, s) || Run.insideInlineCode(n, s) { return n }
      if U.count(n, Tokens.doubleTilde) % 2 == 1 { return n + [U.tilde] }
    }
    return n
  }

  /// `sn`
  static func fenceTick(_ n: S, _ r: Int) -> Bool { partOfFence(n, r) }

  /// `tn`: an open `$$` block is closed
  mutating func katex(_ n: S) -> S {
    guard n.contains(U.dollar) else { return n }
    // `Gn`: `$$` outside inline code
    var r = 0, code = false, i = 0
    while i < n.count - 1 {
      if n[i] == U.tick && !Run.fenceTick(n, i) { code.toggle() }
      if !code && n[i] == U.dollar && n[i + 1] == U.dollar { r += 1; i += 1 }
      i += 1
    }
    guard r % 2 == 1 else { return n }
    // `Xn`
    if n.last == U.dollar && !n.hasSuffix(Tokens.doubleDollar) { return n + [U.dollar] }
    let open = U.indexOf(n, Tokens.doubleDollar)
    if open != -1 && U.indexOf(n, [U.newline], from: open) != -1 && n.last != U.newline {
      return n + Tokens.nlDoubleDollar
    }
    return n + Tokens.doubleDollar
  }
}
