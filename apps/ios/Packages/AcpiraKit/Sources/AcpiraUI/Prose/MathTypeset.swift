import AcpiraDesign
import CoreText
import SwiftUI

// `$$` display math, typeset the way KaTeX does it for the webview: the same faces (bundled KaTeX fonts), the same TFM
// metrics (KaTeXMetrics.generated.swift) and the same placement rules for the TeX subset agents write — symbols with
// TeX inter-atom spacing, sub- and superscripts, big operators with limits, fractions, roots, \left…\right
// delimiters, accents, \text and operator names. Everything is laid out in points, y up from the baseline, and drawn as
// glyph outlines, so each glyph lands where KaTeX's CSS boxes put it.

// MARK: - Atoms

enum MathClass { case ord, op, bin, rel, open, close, punct, inner }

indirect enum MathNode {
  /// One symbol in a face
  case symbol(Character, KaTeXFace, MathClass)
  /// Upright text in a face (\text, \mathrm, operator names)
  case word(String, KaTeXFace, MathClass)
  case row([MathNode])
  case scripts(base: MathNode?, sup: MathNode?, sub: MathNode?)
  /// A large operator (∑ ∏ ∫ …): Size2 in display style, Size1 otherwise; `limits` puts scripts above and below
  case bigOp(Character, limits: Bool)
  /// A named operator (\lim, \sin …) in the upright face
  case namedOp(String, limits: Bool)
  case frac(MathNode, MathNode)
  case sqrt(MathNode)
  /// An explicit space in em
  case space(Double)
  case delimited(Character?, MathNode, Character?)
  case accent(Character, MathNode)
  case overline(MathNode)
}

// MARK: - Parser

/// Recursive descent over the TeX source; anything it does not know falls back to its literal characters
struct TeXParser {
  private var chars: [Character]
  private var at = 0

  init(_ source: String) { chars = Array(source) }

  static func parse(_ source: String) -> MathNode {
    var p = TeXParser(source)
    return .row(p.row())
  }

  private var current: Character? { at < chars.count ? chars[at] : nil }

  private mutating func skipSpaces() {
    while let c = current, c.isWhitespace { at += 1 }
  }

  /// Atoms until a closing brace, `\right` or the end
  mutating func row() -> [MathNode] {
    var out: [MathNode] = []
    while true {
      skipSpaces()
      guard let c = current, c != "}" else { break }
      if peekCommand() == "right" { break }
      if c == "^" || c == "_" {
        // A script with no base attaches to an empty one
        at += 1
        let arg = argument()
        out.append(c == "^" ? .scripts(base: nil, sup: arg, sub: nil) : .scripts(base: nil, sup: nil, sub: arg))
        continue
      }
      guard let base = primary() else { continue }
      out.append(scripted(base))
    }
    return out
  }

  /// A base followed by any `^` / `_` / `'` scripts
  private mutating func scripted(_ base: MathNode) -> MathNode {
    var sup: [MathNode] = []
    var sub: MathNode?
    var hasSup = false
    while true {
      skipSpaces()
      if current == "'" {
        at += 1
        sup.append(.symbol("′", .main, .ord))
        hasSup = true
      } else if current == "^" {
        at += 1
        if let arg = argument() { sup.append(arg) }
        hasSup = true
      } else if current == "_" {
        at += 1
        sub = argument()
      } else { break }
    }
    guard hasSup || sub != nil else { return base }
    return .scripts(base: base, sup: hasSup ? .row(sup) : nil, sub: sub)
  }

  /// A braced group or a single token
  private mutating func argument() -> MathNode? {
    skipSpaces()
    if current == "{" {
      at += 1
      let inner = row()
      if current == "}" { at += 1 }
      return .row(inner)
    }
    return primary()
  }

  /// The raw text of a braced argument (\text{…}, \operatorname{…})
  private mutating func rawArgument() -> String {
    skipSpaces()
    guard current == "{" else { return current.map { at += 1; return String($0) } ?? "" }
    at += 1
    var depth = 1
    var out = ""
    while let c = current {
      at += 1
      if c == "{" { depth += 1 } else if c == "}" { depth -= 1; if depth == 0 { break } }
      out.append(c)
    }
    return out
  }

  private func peekCommand() -> String? {
    guard current == "\\" else { return nil }
    var j = at + 1
    var name = ""
    while j < chars.count, chars[j].isLetter { name.append(chars[j]); j += 1 }
    return name
  }

  private mutating func command() -> String {
    at += 1  // backslash
    guard let c = current else { return "" }
    if !c.isLetter { at += 1; return String(c) }
    var name = ""
    while let c = current, c.isLetter { name.append(c); at += 1 }
    return name
  }

  /// A delimiter after \left / \right: a character or a command naming one; "." is none
  private mutating func delimiter() -> Character? {
    skipSpaces()
    guard let c = current else { return nil }
    if c == "\\" {
      let name = command()
      return TeX.delimiters[name] ?? name.first
    }
    at += 1
    return c == "." ? nil : c
  }

  private mutating func primary() -> MathNode? {
    skipSpaces()
    guard let c = current else { return nil }
    if c == "{" {
      at += 1
      let inner = row()
      if current == "}" { at += 1 }
      return .row(inner)
    }
    if c == "\\" { return commandNode(command()) }
    at += 1
    return TeX.symbol(c)
  }

  private mutating func commandNode(_ name: String) -> MathNode? {
    if let space = TeX.spaces[name] { return .space(space) }
    if let s = TeX.commands[name] { return .symbol(s.0, s.1, s.2) }
    if let op = TeX.bigOps[name] { return .bigOp(op.0, limits: op.1) }
    if let limits = TeX.namedOps[name] { return .namedOp(name, limits: limits) }
    if let mark = TeX.accents[name] { return argument().map { .accent(mark, $0) } }
    switch name {
    case "frac", "dfrac", "tfrac", "cfrac":
      let num = argument() ?? .row([])
      return .frac(num, argument() ?? .row([]))
    case "sqrt":
      skipSpaces()
      // The optional index is dropped
      if current == "[" { while let c = current, c != "]" { at += 1; _ = c }; at += 1 }
      return argument().map { .sqrt($0) }
    case "text", "textrm", "mathrm", "textnormal", "mbox":
      return .word(rawArgument(), .main, .ord)
    case "textbf", "mathbf", "boldsymbol", "bm":
      return .word(rawArgument(), .mainBold, .ord)
    case "mathit", "textit":
      return .row(rawArgument().map { .symbol($0, .math, .ord) })
    case "operatorname":
      return .namedOp(rawArgument(), limits: false)
    case "overline":
      return argument().map { .overline($0) }
    case "left":
      let open = delimiter()
      let inner = row()
      var close: Character?
      if peekCommand() == "right" { _ = command(); close = delimiter() }
      return .delimited(open, .row(inner), close)
    case "mathbb", "mathcal", "mathfrak", "mathscr", "mathsf", "mathtt", "displaystyle", "textstyle", "limits", "nolimits":
      // Faces this subset does not carry render the plain letters; style switches are ignored
      return name.hasPrefix("math") ? argument() : nil
    default:
      // Unknown: the command's name, upright, so nothing silently disappears
      return .word(name, .main, .ord)
    }
  }
}

// MARK: - Symbol tables (KaTeX's symbols.js, the common part)

enum TeX {
  static func symbol(_ c: Character) -> MathNode {
    if c.isLetter && c.isASCII { return .symbol(c, .math, .ord) }
    if c.isNumber && c.isASCII { return .symbol(c, .main, .ord) }
    switch c {
    case "+": return .symbol("+", .main, .bin)
    case "-": return .symbol("−", .main, .bin)
    case "*": return .symbol("∗", .main, .bin)
    case "=", "<", ">", ":": return .symbol(c, .main, .rel)
    case "(", "[": return .symbol(c, .main, .open)
    case ")", "]": return .symbol(c, .main, .close)
    case ",", ";": return .symbol(c, .main, .punct)
    case "|": return .symbol("∣", .main, .ord)
    case "~": return .space(0.25)
    default: return .symbol(c, .main, .ord)
    }
  }

  /// em for \, \: \; \! \quad …
  static let spaces: [String: Double] = [
    ",": 3.0 / 18, ":": 4.0 / 18, ">": 4.0 / 18, ";": 5.0 / 18, "!": -3.0 / 18, " ": 0.25,
    "quad": 1, "qquad": 2, "thinspace": 3.0 / 18, "medspace": 4.0 / 18, "thickspace": 5.0 / 18,
  ]

  static let delimiters: [String: Character] = [
    "{": "{", "}": "}", "langle": "⟨", "rangle": "⟩", "lfloor": "⌊", "rfloor": "⌋", "lceil": "⌈", "rceil": "⌉",
    "|": "∥", "vert": "∣", "Vert": "∥", "lvert": "∣", "rvert": "∣", "lbrace": "{", "rbrace": "}",
  ]

  static let bigOps: [String: (Character, Bool)] = [
    "sum": ("∑", true), "prod": ("∏", true), "coprod": ("∐", true), "bigcup": ("⋃", true), "bigcap": ("⋂", true),
    "bigoplus": ("⨁", true), "bigotimes": ("⨂", true), "int": ("∫", false), "iint": ("∬", false), "oint": ("∮", false),
  ]

  /// Operator names and whether they take limits in display style
  static let namedOps: [String: Bool] = [
    "lim": true, "max": true, "min": true, "sup": true, "inf": true, "det": true, "gcd": true, "Pr": true,
    "sin": false, "cos": false, "tan": false, "cot": false, "sec": false, "csc": false, "sinh": false, "cosh": false,
    "tanh": false, "log": false, "ln": false, "lg": false, "exp": false, "arg": false, "deg": false, "dim": false,
    "ker": false, "hom": false, "arcsin": false, "arccos": false, "arctan": false,
  ]

  static let accents: [String: Character] = [
    "hat": "ˆ", "widehat": "ˆ", "bar": "ˉ", "tilde": "˜", "widetilde": "˜", "dot": "˙", "ddot": "¨", "vec": "⃗",
    "acute": "ˊ", "grave": "ˋ", "check": "ˇ", "breve": "˘",
  ]

  static let commands: [String: (Character, KaTeXFace, MathClass)] = {
    var out: [String: (Character, KaTeXFace, MathClass)] = [:]
    let lowerGreek: [(String, Character)] = [
      ("alpha", "α"), ("beta", "β"), ("gamma", "γ"), ("delta", "δ"), ("epsilon", "ϵ"), ("varepsilon", "ε"),
      ("zeta", "ζ"), ("eta", "η"), ("theta", "θ"), ("vartheta", "ϑ"), ("iota", "ι"), ("kappa", "κ"), ("lambda", "λ"),
      ("mu", "μ"), ("nu", "ν"), ("xi", "ξ"), ("pi", "π"), ("varpi", "ϖ"), ("rho", "ρ"), ("varrho", "ϱ"), ("sigma", "σ"),
      ("varsigma", "ς"), ("tau", "τ"), ("upsilon", "υ"), ("phi", "ϕ"), ("varphi", "φ"), ("chi", "χ"), ("psi", "ψ"),
      ("omega", "ω"),
    ]
    for (name, c) in lowerGreek { out[name] = (c, .math, .ord) }
    let upperGreek: [(String, Character)] = [
      ("Gamma", "Γ"), ("Delta", "Δ"), ("Theta", "Θ"), ("Lambda", "Λ"), ("Xi", "Ξ"), ("Pi", "Π"), ("Sigma", "Σ"),
      ("Upsilon", "Υ"), ("Phi", "Φ"), ("Psi", "Ψ"), ("Omega", "Ω"),
    ]
    for (name, c) in upperGreek { out[name] = (c, .main, .ord) }
    let rel: [(String, Character)] = [
      ("le", "≤"), ("leq", "≤"), ("ge", "≥"), ("geq", "≥"), ("ne", "≠"), ("neq", "≠"), ("approx", "≈"),
      ("equiv", "≡"), ("sim", "∼"), ("simeq", "≃"), ("cong", "≅"), ("to", "→"), ("rightarrow", "→"),
      ("leftarrow", "←"), ("gets", "←"), ("Rightarrow", "⇒"), ("Leftarrow", "⇐"), ("leftrightarrow", "↔"),
      ("Leftrightarrow", "⇔"), ("iff", "⟺"), ("implies", "⟹"), ("mapsto", "↦"), ("in", "∈"), ("notin", "∉"),
      ("ni", "∋"), ("subset", "⊂"), ("supset", "⊃"), ("subseteq", "⊆"), ("supseteq", "⊇"), ("propto", "∝"),
      ("ll", "≪"), ("gg", "≫"), ("perp", "⊥"), ("mid", "∣"), ("parallel", "∥"), ("prec", "≺"), ("succ", "≻"),
      ("longrightarrow", "⟶"), ("longleftarrow", "⟵"),
    ]
    for (name, c) in rel { out[name] = (c, .main, .rel) }
    let bin: [(String, Character)] = [
      ("cdot", "⋅"), ("times", "×"), ("pm", "±"), ("mp", "∓"), ("div", "÷"), ("cup", "∪"), ("cap", "∩"),
      ("circ", "∘"), ("ast", "∗"), ("oplus", "⊕"), ("otimes", "⊗"), ("setminus", "∖"), ("wedge", "∧"),
      ("land", "∧"), ("vee", "∨"), ("lor", "∨"), ("bullet", "∙"), ("star", "⋆"),
    ]
    for (name, c) in bin { out[name] = (c, .main, .bin) }
    let ord: [(String, Character)] = [
      ("infty", "∞"), ("partial", "∂"), ("nabla", "∇"), ("forall", "∀"), ("exists", "∃"), ("emptyset", "∅"),
      ("varnothing", "∅"), ("prime", "′"), ("hbar", "ℏ"), ("ell", "ℓ"), ("angle", "∠"), ("triangle", "△"),
      ("neg", "¬"), ("lnot", "¬"), ("top", "⊤"), ("bot", "⊥"), ("aleph", "ℵ"), ("Re", "ℜ"), ("Im", "ℑ"),
      ("wp", "℘"), ("surd", "√"), ("%", "%"), ("$", "$"), ("#", "#"), ("&", "&"), ("_", "_"),
    ]
    for (name, c) in ord { out[name] = (c, .main, .ord) }
    for (name, c) in [("ldots", "…"), ("dots", "…"), ("cdots", "⋯"), ("vdots", "⋮"), ("ddots", "⋱")] as [(String, Character)] {
      out[name] = (c, .main, .inner)
    }
    out["{"] = ("{", .main, .open)
    out["}"] = ("}", .main, .close)
    out["langle"] = ("⟨", .main, .open)
    out["rangle"] = ("⟩", .main, .close)
    out["lfloor"] = ("⌊", .main, .open)
    out["rfloor"] = ("⌋", .main, .close)
    out["lceil"] = ("⌈", .main, .open)
    out["rceil"] = ("⌉", .main, .close)
    out["|"] = ("∥", .main, .ord)
    out["vert"] = ("∣", .main, .ord)
    out["colon"] = (":", .main, .punct)
    return out
  }()
}

// MARK: - Boxes

/// Something drawn: a glyph outline at a baseline point, or a filled rule (both y up from the box's baseline)
enum MathInk {
  case glyph(CTFont, CGGlyph, CGPoint)
  case rule(CGRect)
}

struct MathBox {
  var width: CGFloat = 0
  var height: CGFloat = 0
  var depth: CGFloat = 0
  /// The italic correction already counted in `width` (KaTeX's margin-right on a slanted glyph)
  var italic: CGFloat = 0
  var cls: MathClass = .ord
  /// A single symbol: scripts sit at fixed shifts from it instead of hanging off its box
  var isSymbol = false
  var ink: [MathInk] = []

  /// Adds another box's ink with its baseline origin at (x, y)
  mutating func place(_ box: MathBox, x: CGFloat, y: CGFloat) {
    for piece in box.ink {
      switch piece {
      case .glyph(let f, let g, let p): ink.append(.glyph(f, g, CGPoint(x: p.x + x, y: p.y + y)))
      case .rule(let r): ink.append(.rule(r.offsetBy(dx: x, dy: y)))
      }
    }
  }
}

/// KaTeX's styles: display, text, script, scriptscript, each possibly cramped
struct MathStyle {
  var level: Int
  var cramped = false

  static let display = MathStyle(level: 0)
  var isDisplay: Bool { level == 0 }
  /// Script levels get tight spacing
  var tight: Bool { level >= 2 }
  var multiplier: CGFloat { [1, 1, 0.7, 0.5][level] }
  /// The column of the font parameter tables
  var sizeIndex: Int { [0, 0, 1, 2][level] }

  func sup() -> MathStyle { MathStyle(level: level < 2 ? 2 : 3, cramped: cramped) }
  func sub() -> MathStyle { MathStyle(level: level < 2 ? 2 : 3, cramped: true) }
  func fracNum() -> MathStyle { MathStyle(level: level == 0 ? 1 : level == 1 ? 2 : 3, cramped: cramped) }
  func fracDen() -> MathStyle { MathStyle(level: level == 0 ? 1 : level == 1 ? 2 : 3, cramped: true) }
  func crampedStyle() -> MathStyle { MathStyle(level: level, cramped: true) }
}

// MARK: - Layout

struct MathLayout {
  /// The em of display and text style, in points
  let base: CGFloat

  func size(_ s: MathStyle) -> CGFloat { base * s.multiplier }
  /// A font parameter in points for a style
  func param(_ name: String, _ s: MathStyle) -> CGFloat { CGFloat(KaTeXFont.param(name, s.sizeIndex)) * size(s) }

  func layout(_ node: MathNode, _ s: MathStyle) -> MathBox {
    switch node {
    case .symbol(let c, let face, let cls):
      var box = symbol(c, face, s)
      box.cls = cls
      return box
    case .word(let text, let face, let cls):
      var box = hlist(text.map { symbol($0, face, s, text: true) })
      box.cls = cls
      return box
    case .row(let nodes):
      return row(nodes, s)
    case .scripts(let base, let sup, let sub):
      return scripts(base, sup, sub, s)
    case .bigOp(let c, _):
      return bigOp(c, s).box
    case .namedOp(let name, _):
      var box = hlist(name.map { symbol($0, .main, s, text: true) })
      box.cls = .op
      return box
    case .frac(let num, let den):
      return frac(num, den, s)
    case .sqrt(let inner):
      return sqrt(inner, s)
    case .space(let em):
      return MathBox(width: CGFloat(em) * size(s))
    case .delimited(let open, let inner, let close):
      return delimited(open, inner, close, s)
    case .accent(let mark, let inner):
      return accent(mark, inner, s)
    case .overline(let inner):
      return overline(inner, s)
    }
  }

  /// A glyph with its TeX metrics; a character the face lacks falls back to Main, then to the system's fallback face
  func symbol(_ c: Character, _ face: KaTeXFace, _ s: MathStyle, text: Bool = false) -> MathBox {
    let pt = size(s)
    guard let scalar = c.unicodeScalars.first else { return MathBox() }
    if c == " " { return MathBox(width: 0.25 * pt) }
    for f in [face, .main, .ams] {
      guard let m = KaTeXFont.metrics(f, scalar) else { continue }
      let font = KaTeXFont.ctFont(f, size: pt)
      // KaTeX keeps a slanted glyph's italic correction as a right margin, except inside \text
      let italic = text ? 0 : CGFloat(m.italic) * pt
      var box = MathBox(width: CGFloat(m.width) * pt + italic, height: CGFloat(m.height) * pt,
                        depth: CGFloat(m.depth) * pt, italic: italic, isSymbol: true)
      if let g = glyph(font, c) { box.ink = [.glyph(font, g, .zero)] }
      return box
    }
    // Not in KaTeX's faces (CJK inside \text …): the system fallback, measured from the font
    let mainFont = KaTeXFont.ctFont(.main, size: pt)
    let string = String(c) as CFString
    let font = CTFontCreateForString(mainFont, string, CFRange(location: 0, length: CFStringGetLength(string)))
    guard let g = glyph(font, c) else { return MathBox() }
    var glyphs = [g]
    var advance = CGSize.zero
    CTFontGetAdvancesForGlyphs(font, .horizontal, &glyphs, &advance, 1)
    return MathBox(width: advance.width, height: CTFontGetAscent(font), depth: CTFontGetDescent(font), isSymbol: true,
                   ink: [.glyph(font, g, .zero)])
  }

  private func glyph(_ font: CTFont, _ c: Character) -> CGGlyph? {
    let units = Array(String(c).utf16)
    var glyphs = [CGGlyph](repeating: 0, count: units.count)
    guard CTFontGetGlyphsForCharacters(font, units, &glyphs, units.count), glyphs[0] != 0 else { return nil }
    return glyphs[0]
  }

  /// Boxes side by side on one baseline, no spacing
  func hlist(_ boxes: [MathBox]) -> MathBox {
    var out = MathBox()
    for b in boxes {
      out.place(b, x: out.width, y: 0)
      out.width += b.width
      out.height = max(out.height, b.height)
      out.depth = max(out.depth, b.depth)
    }
    if boxes.count == 1 { out.italic = boxes[0].italic; out.isSymbol = boxes[0].isSymbol }
    return out
  }

  /// A math list with TeX's inter-atom spacing (KaTeX spacingData)
  func row(_ nodes: [MathNode], _ s: MathStyle) -> MathBox {
    var boxes: [(box: MathBox, space: Bool)] = nodes.map { node in
      if case .space = node { return (layout(node, s), true) }
      return (layout(node, s), false)
    }
    // A binary operator with nothing to bind on one side is an ordinary symbol
    var previous: MathClass?
    for i in boxes.indices where !boxes[i].space {
      if boxes[i].box.cls == .bin {
        let next = boxes[(i + 1)...].first { !$0.space }?.box.cls
        if previous == nil || [.bin, .op, .rel, .open, .punct].contains(previous!) || next == nil
          || [.rel, .close, .punct].contains(next!) {
          boxes[i].box.cls = .ord
        }
      }
      previous = boxes[i].box.cls
    }
    var out = MathBox()
    var last: MathClass?
    for item in boxes {
      if !item.space, let l = last {
        out.width += spacing(l, item.box.cls, s)
      }
      out.place(item.box, x: out.width, y: 0)
      out.width += item.box.width
      out.height = max(out.height, item.box.height)
      out.depth = max(out.depth, item.box.depth)
      if !item.space { last = item.box.cls }
    }
    if boxes.count == 1 {
      out.italic = boxes[0].box.italic
      out.isSymbol = boxes[0].box.isSymbol
      out.cls = boxes[0].box.cls
    }
    return out
  }

  /// thin 3mu, medium 4mu, thick 5mu between two atom classes; script styles keep only the thin ones around operators
  func spacing(_ a: MathClass, _ b: MathClass, _ s: MathStyle) -> CGFloat {
    let mu = param("quad", s) / 18
    let thin = 3 * mu, medium = 4 * mu, thick = 5 * mu
    if s.tight {
      switch (a, b) {
      case (.ord, .op), (.op, .ord), (.op, .op), (.close, .op), (.inner, .op): return thin
      default: return 0
      }
    }
    switch (a, b) {
    case (.ord, .op), (.ord, .inner), (.op, .ord), (.op, .op), (.op, .inner), (.close, .op), (.close, .inner),
         (.inner, .ord), (.inner, .op), (.inner, .open), (.inner, .punct), (.inner, .inner),
         (.punct, .ord), (.punct, .op), (.punct, .open), (.punct, .close), (.punct, .punct), (.punct, .inner):
      return thin
    case (.ord, .bin), (.bin, .ord), (.bin, .op), (.bin, .open), (.bin, .inner), (.close, .bin), (.inner, .bin):
      return medium
    case (.ord, .rel), (.op, .rel), (.rel, .ord), (.rel, .op), (.rel, .open), (.rel, .inner), (.close, .rel),
         (.punct, .rel), (.inner, .rel):
      return thick
    default:
      return 0
    }
  }

  /// A big operator's glyph, centred on the axis (KaTeX op.js); returns the shift it was moved down by
  func bigOp(_ c: Character, _ s: MathStyle) -> (box: MathBox, shift: CGFloat) {
    var box = symbol(c, s.isDisplay ? .size2 : .size1, s)
    let shift = (box.height - box.depth) / 2 - param("axisHeight", s)
    box.height -= shift
    box.depth += shift
    box.ink = box.ink.map { if case .glyph(let f, let g, let p) = $0 { .glyph(f, g, CGPoint(x: p.x, y: p.y - shift)) } else { $0 } }
    box.cls = .op
    box.isSymbol = false
    return (box, shift)
  }

  func scripts(_ baseNode: MathNode?, _ supNode: MathNode?, _ subNode: MathNode?, _ s: MathStyle) -> MathBox {
    // Limits above and below: a display-style operator that takes them
    if let baseNode, s.isDisplay {
      switch baseNode {
      case .bigOp(let c, true):
        let op = bigOp(c, s)
        return limits(op.box, shift: op.shift, slant: op.box.italic, supNode, subNode, s)
      case .namedOp(_, true):
        return limits(layout(baseNode, s), shift: 0, slant: 0, supNode, subNode, s)
      default: break
      }
    }
    var base = baseNode.map { layout($0, s) } ?? MathBox()
    let supStyle = s.sup(), subStyle = s.sub()
    let sup = supNode.map { layout($0, supStyle) }
    let sub = subNode.map { layout($0, subStyle) }
    // KaTeX supsub.js
    var supShift: CGFloat = 0, subShift: CGFloat = 0
    if !base.isSymbol {
      supShift = base.height - param("supDrop", supStyle)
      subShift = base.depth + param("subDrop", subStyle)
    }
    let xHeight = param("xHeight", s)
    let minSup = s.isDisplay ? param("sup1", s) : s.cramped ? param("sup3", s) : param("sup2", s)
    if let sup, let sub {
      supShift = max(supShift, minSup, sup.depth + 0.25 * xHeight)
      subShift = max(subShift, param("sub2", s))
      let rule = param("defaultRuleThickness", s)
      if (supShift - sup.depth) - (sub.height - subShift) < 4 * rule {
        subShift = 4 * rule - (supShift - sup.depth) + sub.height
        let psi = 0.8 * xHeight - (supShift - sup.depth)
        if psi > 0 { supShift += psi; subShift -= psi }
      }
    } else if let sub {
      subShift = max(subShift, param("sub1", s), sub.height - 0.8 * xHeight)
    } else if let sup {
      supShift = max(supShift, minSup, sup.depth + 0.25 * xHeight)
    }
    // \scriptspace after the scripts
    let scriptSpace = 0.05 * self.base
    let x = base.width
    var out = base
    out.isSymbol = false
    out.italic = 0
    var right = x
    if let sup {
      out.place(sup, x: x, y: supShift)
      right = max(right, x + sup.width)
      out.height = max(out.height, supShift + sup.height)
      out.depth = max(out.depth, sup.depth - supShift)
    }
    if let sub {
      // A slanted symbol's subscript tucks under it
      let subX = x - (base.isSymbol ? base.italic : 0)
      out.place(sub, x: subX, y: -subShift)
      right = max(right, subX + sub.width)
      out.depth = max(out.depth, subShift + sub.depth)
      out.height = max(out.height, sub.height - subShift)
    }
    out.width = right + scriptSpace
    base.ink = []
    return out
  }

  /// Scripts centred above and below an operator (KaTeX assembleSupSub)
  func limits(_ op: MathBox, shift: CGFloat, slant: CGFloat, _ supNode: MathNode?, _ subNode: MathNode?, _ s: MathStyle) -> MathBox {
    let sup = supNode.map { layout($0, s.sup()) }
    let sub = subNode.map { layout($0, s.sub()) }
    let width = max(op.width, sup?.width ?? 0, sub?.width ?? 0)
    var out = MathBox(width: width, height: op.height, depth: op.depth, cls: .op)
    out.place(op, x: (width - op.width) / 2, y: 0)
    let spacing5 = param("bigOpSpacing5", s)
    if let sup {
      let kern = max(param("bigOpSpacing1", s), param("bigOpSpacing3", s) - sup.depth)
      let y = op.height + kern + sup.depth
      out.place(sup, x: (width - sup.width) / 2 + slant / 2, y: y)
      out.height = y + sup.height + spacing5
    }
    if let sub {
      let kern = max(param("bigOpSpacing2", s), param("bigOpSpacing4", s) - sub.height)
      let y = -(op.depth + kern + sub.height)
      out.place(sub, x: (width - sub.width) / 2 - slant / 2, y: y)
      out.depth = -y + sub.depth + spacing5
    }
    return out
  }

  /// KaTeX genfrac.js with a bar
  func frac(_ numNode: MathNode, _ denNode: MathNode, _ s: MathStyle) -> MathBox {
    let num = layout(numNode, s.fracNum())
    let den = layout(denNode, s.fracDen())
    let rule = param("defaultRuleThickness", s)
    let axis = param("axisHeight", s)
    var numShift = s.isDisplay ? param("num1", s) : param("num2", s)
    var denShift = s.isDisplay ? param("denom1", s) : param("denom2", s)
    let clearance = s.isDisplay ? 3 * rule : rule
    let numGap = (numShift - num.depth) - (axis + rule / 2)
    if numGap < clearance { numShift += clearance - numGap }
    let denGap = (axis - rule / 2) - (den.height - denShift)
    if denGap < clearance { denShift += clearance - denGap }
    // \nulldelimiterspace on each side
    let side = 0.12 * size(s)
    let inner = max(num.width, den.width)
    var out = MathBox(width: inner + 2 * side, height: numShift + num.height, depth: denShift + den.depth)
    out.place(num, x: side + (inner - num.width) / 2, y: numShift)
    out.place(den, x: side + (inner - den.width) / 2, y: -denShift)
    out.ink.append(.rule(CGRect(x: side, y: axis - rule / 2, width: inner, height: rule)))
    return out
  }

  /// A radical: the smallest surd glyph that covers the body, its rule running over the body (KaTeX sqrt.js, glyphs
  /// in place of its SVG paths)
  func sqrt(_ innerNode: MathNode, _ s: MathStyle) -> MathBox {
    let inner = layout(innerNode, s.crampedStyle())
    let theta = param("defaultRuleThickness", s)
    let phi = s.isDisplay ? param("xHeight", s) : theta
    var clearance = theta + phi / 4
    let needed = inner.height + inner.depth + clearance + theta
    var surd = symbol("√", .main, s)
    for face in [KaTeXFace.main, .size1, .size2] {
      surd = symbol("√", face, s)
      if surd.height + surd.depth >= needed { break }
    }
    let total = surd.height + surd.depth
    if total - theta > inner.height + inner.depth + clearance {
      clearance = (clearance + total - theta - inner.height - inner.depth) / 2
    }
    // The surd's top meets the rule's top
    let top = inner.height + clearance + theta
    var out = MathBox(width: surd.width + inner.width, height: top, depth: max(inner.depth, total - top))
    out.place(surd, x: 0, y: top - surd.height)
    out.place(inner, x: surd.width, y: 0)
    out.ink.append(.rule(CGRect(x: surd.width, y: top - theta, width: inner.width, height: theta)))
    return out
  }

  /// \left … \right: delimiters grown to the body around the axis (KaTeX delimiter.js, Main / Size1 / Size2)
  func delimited(_ open: Character?, _ innerNode: MathNode, _ close: Character?, _ s: MathStyle) -> MathBox {
    let inner = layout(innerNode, s)
    let axis = param("axisHeight", s)
    let reach = max(inner.height - axis, inner.depth + axis)
    let wanted = max(reach / 500 * 901 * 2, 2 * reach - 0.5 * size(s))
    func delim(_ c: Character?) -> MathBox {
      guard let c else { return MathBox(width: 0.12 * size(s)) }
      var box = symbol(c, .main, s)
      for face in [KaTeXFace.main, .size1, .size2] {
        guard let scalar = c.unicodeScalars.first, KaTeXFont.metrics(face, scalar) != nil else { continue }
        box = symbol(c, face, s)
        if box.height + box.depth >= wanted { break }
      }
      // Centred on the axis
      let shift = (box.height - box.depth) / 2 - axis
      box.height -= shift
      box.depth += shift
      box.ink = box.ink.map { if case .glyph(let f, let g, let p) = $0 { .glyph(f, g, CGPoint(x: p.x, y: p.y - shift)) } else { $0 } }
      return box
    }
    var out = hlist([delim(open), inner, delim(close)])
    out.cls = .inner
    out.isSymbol = false
    return out
  }

  /// An accent glyph centred over its base, shifted by the base's skew (KaTeX accent.js)
  func accent(_ mark: Character, _ innerNode: MathNode, _ s: MathStyle) -> MathBox {
    let inner = layout(innerNode, s.crampedStyle())
    let accent = symbol(mark, .main, s, text: true)
    var skew: CGFloat = 0
    if inner.isSymbol, case .symbol(let c, let face, _)? = symbolNode(innerNode), let scalar = c.unicodeScalars.first,
       let m = KaTeXFont.metrics(face, scalar) {
      skew = CGFloat(m.skew) * size(s)
    }
    let clearance = min(inner.height, param("xHeight", s))
    var out = inner
    out.isSymbol = false
    let y = inner.height - clearance
    out.place(accent, x: (inner.width - inner.italic - accent.width) / 2 + skew, y: y)
    out.height = max(inner.height, y + accent.height)
    return out
  }

  private func symbolNode(_ node: MathNode) -> MathNode? {
    if case .row(let nodes) = node, nodes.count == 1 { return symbolNode(nodes[0]) }
    if case .symbol = node { return node }
    return nil
  }

  func overline(_ innerNode: MathNode, _ s: MathStyle) -> MathBox {
    let inner = layout(innerNode, s.crampedStyle())
    let rule = param("defaultRuleThickness", s)
    var out = inner
    out.isSymbol = false
    out.ink.append(.rule(CGRect(x: 0, y: inner.height + 3 * rule, width: inner.width, height: rule)))
    out.height = inner.height + 5 * rule
    return out
  }
}

// MARK: - View

/// `$$` display math: KaTeX's display block at the reply's size × 1.21, centred, scrolling sideways when wider than
/// the column; `.katex-display` keeps a 2 px pad above and below
struct MathBlock: View {
  let tex: String
  let style: ProseStyle

  var body: some View {
    let box = MathLayout(base: style.body.size * 1.21).layout(TeXParser.parse(tex), .display)
    ViewThatFits(in: .horizontal) {
      MathCanvas(box: box).frame(maxWidth: .infinity)
      Scroller(.horizontal) { MathCanvas(box: box) }
    }
    .padding(.vertical, Metrics.s0_5)
  }
}

/// Draws a laid-out formula: every glyph outline and rule in one path, filled in the prose ink
struct MathCanvas: View {
  let box: MathBox

  var body: some View {
    Canvas { ctx, _ in
      ctx.fill(path(), with: .color(Palette.fg1))
    }
    .frame(width: box.width, height: box.height + box.depth)
    .accessibilityHidden(true)
  }

  private func path() -> Path {
    var path = Path()
    // Flip from TeX's y-up baseline space into the canvas, the baseline `height` below the top
    for piece in box.ink {
      switch piece {
      case .glyph(let font, let glyph, let p):
        guard let outline = CTFontCreatePathForGlyph(font, glyph, nil) else { continue }
        let transform = CGAffineTransform(translationX: p.x, y: box.height - p.y).scaledBy(x: 1, y: -1)
        path.addPath(Path(outline), transform: transform)
      case .rule(let r):
        path.addRect(CGRect(x: r.minX, y: box.height - r.maxY, width: r.width, height: r.height))
      }
    }
    return path
  }
}
