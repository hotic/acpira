import AcpiraDesign
import AcpiraMarkdown
import SwiftUI

// Inline markdown as one `Text`, so it wraps and selects like a paragraph. Inline code is SF Mono on a rounded
// --hover chip with `.1em .4em` padding (base.css `.acp-shell code`): the padding is laid out as narrow spaces that
// carry the chip attribute too, and a TextRenderer paints one rounded rect behind each run of chip glyphs per line.

/// Marks the glyphs that sit on an inline code chip
struct InlineCodeChip: TextAttribute {}

/// Marks the figure-space padding runs flanking an inline code chip so glyph-based streaming fades
/// ignore synthetic padding when indexing rendered glyphs
struct InlineCodePad: TextAttribute {}

/// Paints the inline code chips behind their glyphs, then the text. A line holding a chip is taller in CSS (the
/// mono inline box reaches past the strut), so each line moves down by the growth of the chip lines above it and
/// by its own top growth; the view pads its bottom by the total (`InlineText`)
struct InlineCodeRenderer: TextRenderer {
  var fill: Color
  var radius: CGFloat
  /// The chip's reach above and below the run's line box (`.1em` of the code size)
  var outsetY: CGFloat
  /// The paragraph's line box and the baseline's depth in it (`CSSLineRenderer`)
  var lineHeight: CGFloat
  var depth: CGFloat
  /// A chip line's growth above and below the strut
  var growth: (top: CGFloat, bottom: CGFloat)
  /// The total growth, known after the first layout
  var extra: CGFloat
  /// Per-paragraph rendered glyph fade tracker while `StreamProse` is live; nil for static text
  var tracker: GlyphFadeTracker?
  var now: Double = 0

  var displayPadding: EdgeInsets { EdgeInsets(top: 0, leading: 0, bottom: extra, trailing: 0) }

  func draw(layout: Text.Layout, in ctx: inout GraphicsContext) {
    var shift: CGFloat = 0
    if let tracker {
      let total = layout.reduce(0) { sum, line in
        sum + line.reduce(0) { $0 + ($1[InlineCodePad.self] == nil ? $1.count : 0) }
      }
      tracker.sync(glyphCount: total, now: now)
    }
    var cursor = 0
    for line in layout {
      let chipped = hasChip(line)
      var c = ctx
      c.translateBy(x: 0, y: baselineShift(line, depth: depth) + shift + (chipped ? growth.top : 0))
      var chip: CGRect?
      var chipFirst: Int?
      func flush() {
        if let rect = chip {
          var bg = c
          // Fade the inline code backdrop with its leading glyph so an empty grey box never pops ahead of the text
          if let tracker, now > 0, let first = chipFirst {
            bg.opacity = tracker.opacity(at: first, now: now)
          }
          bg.fill(RoundedRectangle(cornerRadius: radius).path(in: rect.insetBy(dx: 0, dy: -outsetY)), with: .color(fill))
        }
        chip = nil
        chipFirst = nil
      }
      var scan = cursor
      for run in line {
        let isPad = run[InlineCodePad.self] != nil
        if run[InlineCodeChip.self] != nil {
          let r = run.typographicBounds.rect
          chip = chip.map { $0.union(r) } ?? r
          if !isPad, chipFirst == nil { chipFirst = scan }
        } else { flush() }
        if !isPad { scan += run.count }
      }
      flush()
      let lineEnd = scan
      if let tracker, now > 0, tracker.hasFading(in: cursor..<lineEnd) {
        // The line holds fading glyphs: draw non-pad glyphs one by one at their own opacity
        var index = cursor
        for run in line {
          if run[InlineCodePad.self] != nil { continue }
          for glyph in run {
            var g = c
            g.opacity = tracker.opacity(at: index, now: now)
            g.draw(glyph)
            index += 1
          }
        }
      } else {
        c.draw(line)
      }
      cursor = lineEnd
      if chipped { shift += growth.top + growth.bottom }
    }
  }
}

private func hasChip(_ line: Text.Layout.Line) -> Bool {
  line.contains { $0[InlineCodeChip.self] != nil }
}

/// The height chip lines add to a laid-out paragraph
private func chipGrowth(_ layouts: [Text.LayoutKey.AnchoredLayout], _ growth: (top: CGFloat, bottom: CGFloat)) -> CGFloat {
  let extra = growth.top + growth.bottom
  // Most paragraphs have no code. Walking their glyph runs on every hosted layout cannot add any height.
  guard extra > 0 else { return 0 }
  let lines = layouts.reduce(0) { n, anchored in n + anchored.layout.filter(hasChip).count }
  return CGFloat(lines) * extra
}

struct InlineText: View {
  let inlines: [MDInline]
  let style: TextStyle
  let ink: Color
  let strongInk: Color
  let accent: Color
  var weight: FontWeight = .regular
  /// When true (the default for paragraphs), expands horizontally to fill the reading column; table cells pass false
  /// so columns stay as tight as their longest wrapped line.
  var expand: Bool = true
  /// Streaming fade signal while `StreamProse` is live. Kept as a view input rather than an environment value so
  /// selection text views never compare the full environment as a trait change on streamed updates
  var fade: StreamFadeSignal?
  @Environment(\.snapshot) private var snapshot
  @Environment(\.deferSelectionSurface) private var deferSelection
  @Environment(\.selectionSurfaceActive) private var selectionActive

  var body: some View {
    let liveFade = snapshot ? nil : fade
    let needsSelection = !snapshot && (!deferSelection || selectionActive)
    if let liveFade {
      LiveInlineText(inlines: inlines, style: style, ink: ink, strongInk: strongInk, accent: accent,
                     weight: weight, expand: expand, fade: liveFade, needsSelection: needsSelection)
    } else {
      StaticInlineText(inlines: inlines, style: style, ink: ink, strongInk: strongInk, accent: accent,
                       weight: weight, expand: expand, needsSelection: needsSelection)
    }
  }
}

/// Settled paragraph with no live streaming fade: skips `TimelineView`, `GlyphFadeTracker`, and `.task`, and skips
/// `Text.LayoutKey` preference measurement when no inline code chips are present.
private struct StaticInlineText: View {
  let inlines: [MDInline]
  let style: TextStyle
  let ink: Color
  let strongInk: Color
  let accent: Color
  let weight: FontWeight
  let expand: Bool
  let needsSelection: Bool

  var body: some View {
    let builder = InlineBuilder(style: style, ink: ink, strongInk: strongInk, accent: accent)
    let chipped = hasCode(inlines)
    let paragraph = builder.paragraph(inlines, weight: weight, chipped: chipped)
    Group {
      if chipped {
        ChippedInlineParagraph(paragraph: paragraph, style: style, ink: ink, expand: expand)
      } else {
        // Plain paragraphs need no chip-height preference pass or `InlineCodeRenderer`; `CSSLineRenderer` alone aligns
        // every line to Blink's baseline depth
        let base = paragraph
          .lineBox(style)
          .foregroundStyle(ink)
          .textRenderer(CSSLineRenderer(lineHeight: style.lineHeight, depth: style.renderDepth))
          .fixedSize(horizontal: false, vertical: true)
        if expand {
          base.frame(maxWidth: .infinity, alignment: .leading)
        } else {
          base
        }
      }
    }
    #if canImport(UIKit)
    .modifier(OptionalSelectionOverlay(active: needsSelection) {
      TextSelectionSurface(text: builder.selectionParagraph(inlines, weight: weight))
    })
    #else
    .textSelection(.enabled)
    #endif
  }
}

/// Settled paragraph holding inline code chips: measures chip-line height growth once via `Text.LayoutKey`
private struct ChippedInlineParagraph: View {
  let paragraph: Text
  let style: TextStyle
  let ink: Color
  let expand: Bool
  @State private var extra: CGFloat = 0

  var body: some View {
    let growth = style.inlineGrowth(WebType.mono)
    let needsMeasure = growth.top + growth.bottom > 0
    let base = paragraph
      .lineBox(style)
      .foregroundStyle(ink)
      .textRenderer(InlineCodeRenderer(fill: Palette.hover, radius: Metrics.rSm,
                                       outsetY: (WebType.mono.size * 0.1).rounded(.up),
                                       lineHeight: style.lineHeight, depth: style.renderDepth,
                                       growth: growth, extra: needsMeasure ? extra : 0, tracker: nil, now: 0))
      .fixedSize(horizontal: false, vertical: true)
    let measured = Group {
      if needsMeasure {
        base
          .onPreferenceChange(Text.LayoutKey.self) { layouts in
            let total = chipGrowth(layouts, growth)
            MainActor.assumeIsolated { if extra != total { extra = total } }
          }
          .padding(.bottom, extra)
      } else {
        base
      }
    }
    if expand {
      measured.frame(maxWidth: .infinity, alignment: .leading)
    } else {
      measured
    }
  }
}

/// Live streaming paragraph with rendered-glyph fade (`StreamProse`)
private struct LiveInlineText: View {
  let inlines: [MDInline]
  let style: TextStyle
  let ink: Color
  let strongInk: Color
  let accent: Color
  let weight: FontWeight
  let expand: Bool
  let fade: StreamFadeSignal
  let needsSelection: Bool
  @State private var extra: CGFloat = 0
  @State private var tracker = GlyphFadeTracker()
  @State private var fading = false

  private struct FadePass: Equatable {
    var content: String
    var seedEpoch: Int
    var seeded: Bool
  }

  var body: some View {
    let builder = InlineBuilder(style: style, ink: ink, strongInk: strongInk, accent: accent)
    let chipped = hasCode(inlines)
    let paragraph = builder.paragraph(inlines, weight: weight, chipped: chipped)
    let growth = chipped ? style.inlineGrowth(WebType.mono) : (top: CGFloat(0), bottom: CGFloat(0))
    let content = Markdown.plain(inlines)
    let _ = tracker.prepare(content: content, signal: fade)
    let animating = fading || tracker.isActive
    let pass = FadePass(content: content, seedEpoch: fade.seedEpoch, seeded: fade.seeded)
    // Pause in place instead of swapping a TimelineView for Text: tearing down the text whenever a fade settled
    // discarded its layout while the hosted row was still resizing or the process above it was closing.
    TimelineView(.animation(paused: !animating)) { timeline in
      // A settled paragraph has no fade clock. A new timestamp would invalidate its renderer during cell sizing.
      text(paragraph, chipped: chipped, growth: growth,
           now: animating ? timeline.date.timeIntervalSinceReferenceDate : 0)
    }
    .task(id: pass) {
      if pass.seeded || tracker.needsSeed || !tracker.isActive {
        if fading { fading = false }
        return
      }
      if !fading { fading = true }
      try? await Task.sleep(for: .seconds(Motion.streamChar + 0.04))
      guard !Task.isCancelled else { return }
      tracker.expire(now: Date.timeIntervalSinceReferenceDate)
      fading = false
    }
    #if canImport(UIKit)
    .modifier(OptionalSelectionOverlay(active: needsSelection) {
      TextSelectionSurface(text: builder.selectionParagraph(inlines, weight: weight))
    })
    #else
    .textSelection(.enabled)
    #endif
  }

  @ViewBuilder
  private func text(_ paragraph: Text, chipped: Bool, growth: (top: CGFloat, bottom: CGFloat), now: Double) -> some View {
    let needsMeasure = chipped && (growth.top + growth.bottom > 0)
    let base = paragraph
      .lineBox(style)
      .foregroundStyle(ink)
      .textRenderer(InlineCodeRenderer(fill: Palette.hover, radius: Metrics.rSm,
                                       outsetY: (WebType.mono.size * 0.1).rounded(.up),
                                       lineHeight: style.lineHeight, depth: style.renderDepth, growth: growth,
                                       extra: needsMeasure ? extra : 0, tracker: tracker, now: now))
      .fixedSize(horizontal: false, vertical: true)
    let measured = Group {
      if needsMeasure {
        base
          .onPreferenceChange(Text.LayoutKey.self) { layouts in
            let total = chipGrowth(layouts, growth)
            // In this pass, not a task's hop later: the paragraph then grew after its row was measured and placed, and
            // a chat just opened slid by the chip lines' growth
            MainActor.assumeIsolated { if extra != total { extra = total } }
          }
          .padding(.bottom, extra)
      } else {
        base
      }
    }
    if expand {
      measured.frame(maxWidth: .infinity, alignment: .leading)
    } else {
      measured
    }
  }
}

#if canImport(UIKit)
/// Mounts the transparent native selection `UITextView` only when `active` is true, leaving SwiftUI's native text
/// accessible and `UITextView`-free during idle scrolling and streaming.
private struct OptionalSelectionOverlay<Surface: View>: ViewModifier {
  let active: Bool
  @ViewBuilder let surface: () -> Surface

  func body(content: Content) -> some View {
    if active {
      content
        .accessibilityHidden(true)
        .overlay { LiveOnly { surface() } }
    } else {
      content
    }
  }
}
#endif

private func hasCode(_ inlines: [MDInline]) -> Bool {
  inlines.contains {
    switch $0 {
    case .code: true
    case .strong(let c), .emphasis(let c), .strike(let c), .link(_, let c): hasCode(c)
    case .text, .lineBreak, .image: false
    }
  }
}

/// Builds the styled `Text` for an inline tree
struct InlineBuilder {
  let style: TextStyle
  let ink: Color
  let strongInk: Color
  let accent: Color

  struct Marks { var weight: FontWeight; var italic = false; var strike = false; var strong = false }

  func text(_ inlines: [MDInline], weight: FontWeight = .regular) -> Text {
    list(inlines, Marks(weight: weight))
  }

  /// A whole paragraph: builds one flat `AttributedString` (or bare `Text(verbatim:)` for pure ASCII) whenever no
  /// inline code chip needs a `TextAttribute`, avoiding SwiftUI's `LocalizedStringKey` interpolation resolution tree.
  func paragraph(_ inlines: [MDInline], weight: FontWeight = .regular, chipped: Bool? = nil) -> Text {
    guard !inlines.isEmpty else { return Text(verbatim: "") }
    if inlines.count == 1, case .text(let s) = inlines[0], s.utf8.allSatisfy({ $0 < 0x80 }) {
      return Text(verbatim: s).font(style.font(weight))
    }
    let hasChips = chipped ?? hasCode(inlines)
    if !hasChips {
      var out = AttributedString()
      appendList(inlines, Marks(weight: weight), to: &out)
      if Self.hasBreakable(inlines) {
        var end = AttributedString(CJKWidth.paragraphEnd)
        end.font = style.font()
        out.append(end)
      }
      return Text(out)
    }
    let body = list(inlines, Marks(weight: weight))
    return Self.hasBreakable(inlines) ? Text("\(body)\(invisible(CJKWidth.paragraphEnd))") : body
  }

  /// Appends non-code inlines into one flat `AttributedString` with identical fonts, CJK widening, inter-run CJK break
  /// opportunities, bold ink, strikethrough, and link attributes as the `Text`-joined path.
  private func appendList(_ inlines: [MDInline], _ marks: Marks, to out: inout AttributedString) {
    for i in inlines.indices {
      let cur = inlines[i]
      if i > 0, !isCode(cur), !isCode(inlines[i - 1]),
         let prev = Self.lastScalar(of: inlines[i - 1]), CJKWidth.breakable(prev),
         let next = Self.firstScalar(of: cur), CJKWidth.breakable(next) {
        var br = AttributedString("\u{200B}")
        br.font = style.font()
        out.append(br)
      }
      appendPiece(cur, marks, to: &out)
    }
  }

  private func appendPiece(_ inline: MDInline, _ marks: Marks, to out: inout AttributedString) {
    switch inline {
    case .text(let s):
      appendPlain(s, marks, to: &out)
    case .lineBreak:
      var br = AttributedString("\n")
      br.font = style.font(marks.weight, italic: marks.italic)
      out.append(br)
    case .code(let s):
      appendPlain(s, marks, to: &out)
    case .strong(let children):
      var m = marks; m.weight = .semibold; m.strong = true
      appendList(children, m, to: &out)
    case .emphasis(let children):
      var m = marks; m.italic = true
      appendList(children, m, to: &out)
    case .strike(let children):
      var m = marks; m.strike = true
      appendList(children, m, to: &out)
    case .image(_, let alt):
      appendPlain(alt, marks, to: &out)
    case .link(let url, let children):
      var a = CJKWidth.attributed(Markdown.plain(children), size: style.size, paragraph: false)
      a.font = style.font(marks.weight, italic: marks.italic)
      a.foregroundColor = accent
      a.underlineStyle = .single
      a.link = fileTarget(url) ?? URL(string: url)
      out.append(a)
    }
  }

  private func appendPlain(_ s: String, _ marks: Marks, to out: inout AttributedString) {
    guard !s.isEmpty else { return }
    var a = CJKWidth.attributed(s, size: style.size, paragraph: false)
    var attrs = AttributeContainer()
    attrs.font = style.font(marks.weight, italic: marks.italic)
    if marks.strong { attrs.foregroundColor = strongInk }
    if marks.strike { attrs.strikethroughStyle = .single }
    a.mergeAttributes(attrs, mergePolicy: .keepCurrent)
    out.append(a)
  }

  /// Siblings in order; a code span looks at its neighbours to place its line-break opportunities, and two runs
  /// meeting between ideographs ("中文**粗体**") get the zero-width space `CJKWidth` puts inside a run. Consecutive
  /// non-code siblings coalesce into one `AttributedString` to keep `Text` interpolation trees shallow.
  func list(_ inlines: [MDInline], _ marks: Marks) -> Text {
    var parts: [Text] = []
    parts.reserveCapacity(inlines.count)
    var pending = AttributedString()
    func flushPending() {
      guard !pending.characters.isEmpty else { return }
      parts.append(Text(pending))
      pending = AttributedString()
    }
    for i in inlines.indices {
      let cur = inlines[i]
      if i > 0, !isCode(cur), !isCode(inlines[i - 1]),
         let prev = Self.lastScalar(of: inlines[i - 1]), CJKWidth.breakable(prev),
         let next = Self.firstScalar(of: cur), CJKWidth.breakable(next) {
        var br = AttributedString("\u{200B}")
        br.font = style.font()
        pending.append(br)
      }
      if case .code(let s) = cur {
        flushPending()
        parts.append(code(s, after: i > 0 ? Self.lastChar(of: inlines[i - 1]) : nil,
                          before: i + 1 < inlines.count ? Self.firstChar(of: inlines[i + 1]) : nil))
      } else if !hasCode([cur]) {
        appendPiece(cur, marks, to: &pending)
      } else {
        flushPending()
        parts.append(piece(cur, marks))
      }
    }
    flushPending()
    return join(parts)
  }

  private static func hasBreakable(_ inlines: [MDInline]) -> Bool {
    for inline in inlines {
      switch inline {
      case .text(let s), .code(let s):
        if CJKWidth.needsParagraphEnd(s) { return true }
      case .strong(let c), .emphasis(let c), .strike(let c), .link(_, let c):
        if hasBreakable(c) { return true }
      case .image(_, let alt):
        if CJKWidth.needsParagraphEnd(alt) { return true }
      case .lineBreak:
        break
      }
    }
    return false
  }

  private static func firstScalar(of inline: MDInline) -> Unicode.Scalar? {
    switch inline {
    case .text(let s), .code(let s): s.unicodeScalars.first
    case .strong(let c), .emphasis(let c), .strike(let c), .link(_, let c): c.lazy.compactMap(firstScalar).first
    case .image(_, let alt): alt.unicodeScalars.first
    case .lineBreak: "\n"
    }
  }

  private static func lastScalar(of inline: MDInline) -> Unicode.Scalar? {
    switch inline {
    case .text(let s), .code(let s): s.unicodeScalars.last
    case .strong(let c), .emphasis(let c), .strike(let c), .link(_, let c): c.reversed().lazy.compactMap(lastScalar).first
    case .image(_, let alt): alt.unicodeScalars.last
    case .lineBreak: "\n"
    }
  }

  private static func firstChar(of inline: MDInline) -> Character? {
    switch inline {
    case .text(let s), .code(let s): s.first
    case .strong(let c), .emphasis(let c), .strike(let c), .link(_, let c): c.lazy.compactMap(firstChar).first
    case .image(_, let alt): alt.first
    case .lineBreak: "\n"
    }
  }

  private static func lastChar(of inline: MDInline) -> Character? {
    switch inline {
    case .text(let s), .code(let s): s.last
    case .strong(let c), .emphasis(let c), .strike(let c), .link(_, let c): c.reversed().lazy.compactMap(lastChar).first
    case .image(_, let alt): alt.last
    case .lineBreak: "\n"
    }
  }

  /// A zero-width character in the paragraph's face: a run with no font of its own would take the environment's
  /// and stretch its line
  private func invisible(_ s: String) -> Text {
    Text(verbatim: s).font(style.font())
  }

  private func isCode(_ inline: MDInline) -> Bool {
    if case .code = inline { true } else { false }
  }

  func join(_ parts: [Text]) -> Text {
    Text.joined(parts)
  }

  func piece(_ inline: MDInline, _ marks: Marks) -> Text {
    switch inline {
    case .text(let s):
      return plain(s, marks)
    case .lineBreak:
      return Text(web: "\n")
    case .code(let s):
      return code(s, after: nil, before: nil)
    case .strong(let children):
      var m = marks; m.weight = .semibold; m.strong = true
      return list(children, m)
    case .emphasis(let children):
      var m = marks; m.italic = true
      return list(children, m)
    case .strike(let children):
      var m = marks; m.strike = true
      return list(children, m)
    case .image(_, let alt):
      // An image inside a run (a badge in a link): its alt text; a paragraph's own images stand as views (Prose)
      return plain(alt, marks)
    case .link(let url, let children):
      var a = CJKWidth.attributed(Markdown.plain(children), size: style.size, paragraph: false)
      a.font = style.font(marks.weight, italic: marks.italic)
      a.foregroundColor = accent
      a.underlineStyle = .single
      // A workspace path opens the file page (chat/Link.tsx); session links and the rest go to openLink as they are
      a.link = fileTarget(url) ?? URL(string: url)
      return Text(a)
    }
  }

  func plain(_ s: String, _ marks: Marks) -> Text {
    var t = Text(web: s, size: style.size, paragraph: false).font(style.font(marks.weight, italic: marks.italic))
    if marks.strong { t = t.foregroundStyle(strongInk) }
    if marks.strike { t = t.strikethrough() }
    return t
  }

  /// Code text between two narrow pads, all three on the chip. The pads are no-break spaces so they never stay
  /// behind at a line end, which also hides the break CSS finds at the chip's edges (UAX #14 forbids one before a
  /// no-break space after an ideograph or a space in CoreText); a zero-width space outside the chip gives it back,
  /// except after opening punctuation or before closing punctuation, where CSS has none either.
  /// The pad is a figure space, not U+00A0: when the code is wider than a whole line, TextKit's fallback break still
  /// splits after a U+00A0 and leaves the leading pad as an empty chip on the line before (iOS 27 / macOS 27)
  func code(_ s: String, after previous: Character?, before next: Character?) -> Text {
    let mono = WebType.mono
    // A mono figure space is .6em; the pad is .4em of the code size
    let pad = Text(web: Self.codePad)
      .font(Font(MonoFont.ctFont(size: mono.size * 0.4 / 0.6, weight: .regular)))
      .customAttribute(InlineCodePad())
    // A word joiner after each slash: Chrome finds no break opportunity after `/` before a letter, CoreText does
    var text = CJKWidth.attributed(s.replacingOccurrences(of: "/", with: "/\u{2060}"), size: WebType.mono.size, paragraph: false)
    // A path in backticks opens the file page (chat/Link.tsx InlineFileCode), still looking like code
    if let target = fileTarget(s) {
      text.link = target
      text.foregroundColor = Palette.fg1
    }
    let body = Text(text).styled(mono).foregroundStyle(Palette.fg1)
    let chip = Text("\(pad)\(body)\(pad)").customAttribute(InlineCodeChip())
    let lead = previous.map { !$0.isWhitespace && opens($0) } ?? true ? "" : "\u{200B}"
    let trail = next.map { !$0.isWhitespace && !$0.isPunctuation && !$0.isSymbol } ?? false ? "\u{200B}" : ""
    switch (lead.isEmpty, trail.isEmpty) {
    case (true, true): return chip
    case (false, true): return Text("\(Text(verbatim: lead))\(chip)")
    case (true, false): return Text("\(chip)\(Text(verbatim: trail))")
    case (false, false): return Text("\(Text(verbatim: lead))\(chip)\(Text(verbatim: trail))")
    }
  }

  /// The no-break space on each side of a code chip, shared with the selection text (TextSelection.swift)
  static let codePad = "\u{2007}"

  /// The app's link for a workspace path (FileLinks.swift); a session link stays itself
  private func fileTarget(_ text: String) -> URL? {
    guard SessionLinks.id(ofLink: text) == nil, let file = FileLinks.parse(text) else { return nil }
    return FileLinks.url(file)
  }

  private func opens(_ c: Character) -> Bool {
    c.unicodeScalars.first.map { [.openPunctuation, .initialPunctuation].contains($0.properties.generalCategory) } ?? false
  }
}
