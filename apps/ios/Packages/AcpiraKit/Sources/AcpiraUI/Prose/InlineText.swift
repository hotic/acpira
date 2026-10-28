import AcpiraDesign
import AcpiraMarkdown
import SwiftUI

// Inline markdown as one `Text`, so it wraps and selects like a paragraph. Inline code is SF Mono on a rounded
// --hover chip with `.1em .4em` padding (base.css `.acp-shell code`): the padding is laid out as narrow spaces that
// carry the chip attribute too, and a TextRenderer paints one rounded rect behind each run of chip glyphs per line.

/// Marks the glyphs that sit on an inline code chip
struct InlineCodeChip: TextAttribute {}

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
  /// A streamed tail's glyph fade (the last text of a live Prose)
  var fade: StreamFade?
  var now: Double = 0

  var displayPadding: EdgeInsets { EdgeInsets(top: 0, leading: 0, bottom: extra, trailing: 0) }

  func draw(layout: Text.Layout, in ctx: inout GraphicsContext) {
    var shift: CGFloat = 0
    // Glyphs counted from the end of the text: the fade reaches only the last `reach` of them
    var remaining = fade.map { _ in layout.reduce(0) { $0 + $1.reduce(0) { $0 + $1.count } } } ?? 0
    for line in layout {
      let chipped = hasChip(line)
      var c = ctx
      c.translateBy(x: 0, y: baselineShift(line, depth: depth) + shift + (chipped ? growth.top : 0))
      var chip: CGRect?
      func flush() {
        if let rect = chip {
          c.fill(RoundedRectangle(cornerRadius: radius).path(in: rect.insetBy(dx: 0, dy: -outsetY)), with: .color(fill))
        }
        chip = nil
      }
      for run in line {
        if run[InlineCodeChip.self] != nil {
          let r = run.typographicBounds.rect
          chip = chip.map { $0.union(r) } ?? r
        } else { flush() }
      }
      flush()
      let glyphs = line.reduce(0) { $0 + $1.count }
      if let fade, remaining - glyphs < fade.reach {
        // The line holds fading glyphs: draw it glyph by glyph at each one's opacity
        var left = remaining
        for run in line {
          for glyph in run {
            left -= 1
            var g = c
            g.opacity = fade.opacity(distance: left, now: now)
            g.draw(glyph)
          }
        }
      } else {
        c.draw(line)
      }
      remaining -= glyphs
      if chipped { shift += growth.top + growth.bottom }
    }
  }
}

private func hasChip(_ line: Text.Layout.Line) -> Bool {
  line.contains { $0[InlineCodeChip.self] != nil }
}

/// The height chip lines add to a laid-out paragraph
private func chipGrowth(_ layouts: [Text.LayoutKey.AnchoredLayout], _ growth: (top: CGFloat, bottom: CGFloat)) -> CGFloat {
  let lines = layouts.reduce(0) { n, anchored in n + anchored.layout.filter(hasChip).count }
  return CGFloat(lines) * (growth.top + growth.bottom)
}

struct InlineText: View {
  let inlines: [MDInline]
  let style: TextStyle
  let ink: Color
  let strongInk: Color
  let accent: Color
  var weight: FontWeight = .regular
  @State private var extra: CGFloat = 0
  @Environment(\.streamFade) private var fade

  var body: some View {
    if let fade, !fade.reveals.isEmpty {
      // Only the streamed tail redraws every frame, and only while its last glyphs fade
      TimelineView(.animation) { timeline in text(now: timeline.date.timeIntervalSinceReferenceDate) }
    } else {
      text(now: 0)
    }
  }

  private func text(now: Double) -> some View {
    let growth = hasCode(inlines) ? style.inlineGrowth(WebType.mono) : (top: CGFloat(0), bottom: CGFloat(0))
    return InlineBuilder(style: style, ink: ink, strongInk: strongInk, accent: accent).paragraph(inlines, weight: weight)
      .lineHeight(.exact(points: style.lineHeight))
      .foregroundStyle(ink)
      .textRenderer(InlineCodeRenderer(fill: Palette.hover, radius: Metrics.rSm, outsetY: (WebType.mono.size * 0.1).rounded(.up),
                                       lineHeight: style.lineHeight, depth: style.baselineDepth, growth: growth, extra: extra,
                                       fade: fade, now: now))
      .fixedSize(horizontal: false, vertical: true)
      .onPreferenceChange(Text.LayoutKey.self) { layouts in
        let total = chipGrowth(layouts, growth)
        Task { @MainActor in if extra != total { extra = total } }
      }
      .padding(.bottom, extra)
      .frame(maxWidth: .infinity, alignment: .leading)
  }
}

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

  /// A whole paragraph: the runs, closed by `CJKWidth.paragraphEnd` when it holds CJK
  func paragraph(_ inlines: [MDInline], weight: FontWeight = .regular) -> Text {
    let body = text(inlines, weight: weight)
    return CJKWidth.needsParagraphEnd(Markdown.plain(inlines)) ? Text("\(body)\(invisible(CJKWidth.paragraphEnd))") : body
  }

  /// Siblings in order; a code span looks at its neighbours to place its line-break opportunities, and two runs
  /// meeting between ideographs ("中文**粗体**") get the zero-width space `CJKWidth` puts inside a run
  func list(_ inlines: [MDInline], _ marks: Marks) -> Text {
    join(inlines.indices.flatMap { i -> [Text] in
      let previous = i > 0 ? Markdown.plain([inlines[i - 1]]).unicodeScalars.last : nil
      let current = Markdown.plain([inlines[i]]).unicodeScalars.first
      let seam = previous.map(CJKWidth.breakable) == true && current.map(CJKWidth.breakable) == true
        && !isCode(inlines[i]) && !isCode(inlines[i - 1])
      let gap = seam ? [invisible("\u{200B}")] : []
      if case .code(let s) = inlines[i] {
        return gap + [code(s, after: i > 0 ? Markdown.plain([inlines[i - 1]]).last : nil,
                           before: i + 1 < inlines.count ? Markdown.plain([inlines[i + 1]]).first : nil)]
      }
      return gap + [piece(inlines[i], marks)]
    })
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
    parts.reduce(Text(web: "")) { Text("\($0)\($1)") }
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
      a.link = URL(string: url)
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
  /// except after opening punctuation or before closing punctuation, where CSS has none either
  func code(_ s: String, after previous: Character?, before next: Character?) -> Text {
    let mono = WebType.mono
    // A mono space is .6em; the pad is .4em of the code size
    let pad = Text(web: "\u{00A0}").font(Font(MonoFont.ctFont(size: mono.size * 0.4 / 0.6, weight: .regular)))
    // A word joiner after each slash: Chrome finds no break opportunity after `/` before a letter, CoreText does
    let body = Text(web: s.replacingOccurrences(of: "/", with: "/\u{2060}"), size: WebType.mono.size, paragraph: false).styled(mono).foregroundStyle(Palette.fg1)
    let chip = Text("\(pad)\(body)\(pad)").customAttribute(InlineCodeChip())
    let lead = previous.map { !$0.isWhitespace && opens($0) } ?? true ? "" : "\u{200B}"
    let trail = next.map { !$0.isWhitespace && !$0.isPunctuation && !$0.isSymbol } ?? false ? "\u{200B}" : ""
    return Text("\(Text(web: lead))\(chip)\(Text(web: trail))")
  }

  private func opens(_ c: Character) -> Bool {
    c.unicodeScalars.first.map { [.openPunctuation, .initialPunctuation].contains($0.properties.generalCategory) } ?? false
  }
}
