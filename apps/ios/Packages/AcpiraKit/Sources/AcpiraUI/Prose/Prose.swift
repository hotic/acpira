import AcpiraDesign
import AcpiraMarkdown
import AcpiraModels
import SwiftUI

// chat/Prose.tsx + styles/prose.css: agent markdown. Blocks stack with a --gap; every typographic rule (heading
// scale, list markers, quote rail, table frame, inline code chip) follows prose.css. `tone: .thought` is the quieter
// reasoning scale: text-2 in fg-2, headings at body size, strong one shade up instead of fg-strong.

public enum ProseTone: Sendable { case reply, thought }

public struct Prose: View {
  let blocks: [MDBlock]
  var tone: ProseTone
  @Environment(\.agentAccent) private var accent

  /// `streaming`: the text is still arriving, so its unfinished tail is repaired first (streamdown's streaming mode)
  public init(markdown: String, tone: ProseTone = .reply, streaming: Bool = false) {
    let source = streaming ? Remend.repair(markdown) : markdown
    blocks = Markdown.parse(source.trimmingCharacters(in: .newlines))
    self.tone = tone
  }

  public init(block: TextBlock, tone: ProseTone = .reply) {
    self.init(markdown: block.markdown, tone: tone, streaming: block.streaming == true)
  }

  public var body: some View {
    if !blocks.isEmpty {
      ProseBlocks(blocks: blocks, style: ProseStyle(tone: tone, accent: accent))
    }
  }
}

extension EnvironmentValues {
  /// --accent under accent = agent
  @Entry public var agentAccent: Color = Palette.brand
  /// The streamed tail's fade; set only along the path to the last text of a live Prose
  @Entry var streamFade: StreamFade?
}

// MARK: - Streaming

/// The fading glyphs of a streamed text at one frame: a glyph `d` places before the end of the visible text is
/// character `count - d`, and it fades in from the time the reveal step that brought that character in ran
/// (motion.css `sd-acpGlyph`: opacity only, --stream-char-duration, ease)
struct StreamFade: Equatable {
  var count: Int
  var reveals: [StreamReveal]

  /// Opacity of the glyph `distance` places before the end; 1 once its fade has run
  func opacity(distance: Int, now: Double) -> Double {
    let position = count - distance
    guard let step = reveals.last(where: { $0.from < position && position <= $0.to }) else { return 1 }
    let t = min(max((now - step.time) / Motion.streamChar, 0), 1)
    // CSS `ease`: cubic-bezier(.25, .1, .25, 1)
    return UnitCurve.bezier(startControlPoint: UnitPoint(x: 0.25, y: 0.1), endControlPoint: UnitPoint(x: 0.25, y: 1)).value(at: t)
  }

  /// How many glyphs from the end can still be fading
  var reach: Int { reveals.first.map { count - $0.from } ?? 0 }
}

/// chat/Prose.tsx with streamMotion.ts: a streamed text is paced out at the agent's measured speed and every newly
/// shown glyph fades in. A text that mounts already complete renders at once; once the stream ends, the backlog
/// flushes and the last fades finish, the view settles into a static Prose. Motion off shows the received text.
public struct StreamProse: View {
  let markdown: String
  let streaming: Bool
  var tone: ProseTone
  /// The local images the engine read with the text, for its Markdown images
  var images: [ImageRef]?
  @Environment(\.motionEnabled) private var motion
  @State private var pacer: StreamPacer?

  public init(markdown: String, tone: ProseTone = .reply, streaming: Bool) {
    self.markdown = markdown; self.streaming = streaming; self.tone = tone
    var start: StreamPacer?
    if streaming {
      start = StreamPacer(streaming: true)
      start?.receive(markdown, streaming: true, now: Date.timeIntervalSinceReferenceDate)
    }
    _pacer = State(initialValue: start)
  }

  public init(block: TextBlock, tone: ProseTone = .reply) {
    self.init(markdown: block.markdown, tone: tone, streaming: block.streaming == true)
    images = block.images
  }

  public var body: some View {
    Group {
      if let pacer, motion {
        // The pacing ticks come from the task below; the tail paragraph runs its own frames while glyphs fade
        let text = pacer.visible
        // The turn heading already indicates waiting before the first visible words
        if !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
          Prose(markdown: text, tone: tone, streaming: true)
            .environment(\.streamFade, StreamFade(count: pacer.count, reveals: pacer.reveals))
        } else {
          // An empty Group is no view at all and would never start the task below; this holds it for the first tick
          Color.clear.frame(height: 0)
        }
      } else {
        Prose(markdown: markdown, tone: tone, streaming: streaming)
      }
    }
    .onChange(of: markdown) { receive() }
    .onChange(of: streaming) { receive() }
    .task(id: pacer != nil && motion) { await pace() }
    .environment(\.markdownImages, images)
  }

  private func receive() {
    guard var next = pacer else { return }
    next.receive(markdown, streaming: streaming, now: Date.timeIntervalSinceReferenceDate)
    pacer = next
  }

  /// Ticks at STREAM_PACE.tick until the text has caught up, the stream ended and the last glyph faded in
  private func pace() async {
    guard motion, pacer != nil else { return }
    var last = Date.timeIntervalSinceReferenceDate
    while !Task.isCancelled {
      try? await Task.sleep(for: .seconds(StreamPace.standard.tick))
      // Read after the sleep: text received meanwhile must not be overwritten by a stale copy
      guard !Task.isCancelled, var next = pacer else { return }
      let now = Date.timeIntervalSinceReferenceDate
      next.tick(now: now, dt: now - last)
      last = now
      if !next.streaming && !next.pending && next.reveals.isEmpty {
        pacer = nil
      } else if next != pacer {
        pacer = next
      }
    }
  }
}

/// Resolved type and ink for a prose tone
struct ProseStyle {
  var tone: ProseTone
  var accent: Color
  /// A quote reads its whole content in fg-2
  var inkOverride: Color?
  /// Both tones read at text-1: the webview's thought class asks for text-2, but streamdown's class merge drops
  /// `text-2` next to `text-fg-2`, so thoughts render at the inherited text-1 size and leading (LAB, Chrome)
  var body: TextStyle { WebType.text1 }
  var ink: Color { inkOverride ?? (tone == .reply ? Palette.fg1 : Palette.fg2) }
  var strongInk: Color { tone == .reply ? Palette.fgStrong : Palette.fg1 }
  /// List marker column: `padding-left: 1.2em`
  var listIndent: CGFloat { (body.size * 1.2).rounded() }

  func heading(_ level: Int) -> TextStyle {
    guard tone == .reply else { return body }
    let factor: CGFloat = level == 1 ? 1.35 : level == 2 ? 1.25 : level == 3 ? 1.15 : 1
    // line-height stays --text-1-lh
    return TextStyle(size: (WebType.text1.size * factor).rounded(), lineHeight: WebType.text1.lineHeight)
  }
}

struct ProseBlocks: View {
  let blocks: [MDBlock]
  let style: ProseStyle
  @Environment(\.quoteProse) private var quote

  var body: some View {
    VStack(alignment: .leading, spacing: Metrics.gap) {
      ForEach(Array(blocks.enumerated()), id: \.offset) { index, block in
        ProseBlock(block: block, style: style, first: index == 0)
          // Quoted whole: the blocks nested in a list or a quote take no menu of their own
          .environment(\.quoteProse, nil)
          .quoteMenu(block, quote)
          .tailOnly(index == blocks.count - 1)
      }
    }
    .frame(maxWidth: .infinity, alignment: .leading)
  }
}

struct ProseBlock: View {
  let block: MDBlock
  let style: ProseStyle
  var first: Bool

  var body: some View {
    switch block {
    case .paragraph(let inlines) where holdsImage(inlines):
      ImageParagraph(inlines: inlines, style: style)
    case .paragraph(let inlines):
      InlineText(inlines: inlines, style: style.body, ink: style.ink, strongInk: style.strongInk, accent: style.accent)
    case .heading(let level, let inlines):
      InlineText(inlines: inlines, style: style.heading(level), ink: style.tone == .reply ? Palette.fgStrong : style.strongInk,
                 strongInk: style.tone == .reply ? Palette.fgStrong : style.strongInk, accent: style.accent, weight: .semibold)
        // :not(:first-child) gets margin-top: var(--gap) on top of the block gap
        .padding(.top, first ? 0 : Metrics.gap)
    case .list(let ordered, let start, let items):
      ProseList(ordered: ordered, start: start, items: items, style: style)
    case .quote(let inner):
      ProseQuote(blocks: inner, style: style)
    case .code(let lang, let text):
      Group {
      // A mermaid block the flowchart parser reads is drawn natively; any other diagram goes to mermaid itself in a web
      // view, and stays source if mermaid cannot read it either, as a failed render does in the webview
      if lang == "mermaid", let diagram = MermaidBlock(source: text) {
        diagram
      } else if lang == "mermaid" {
        MermaidWebBlock(source: text)
      } else {
        CodeSurface(text: text, lang: lang)
      }
      }.tailOnly(false)
    case .table(let align, let header, let rows):
      // Tables, code and math appear whole: a fade by distance from the end does not map onto their cells
      ProseTable(align: align, header: header, rows: rows, style: style).tailOnly(false)
    case .rule:
      Rectangle().fill(Palette.conversationLine).frame(height: 1).padding(.vertical, Metrics.gap)
    case .math(let tex):
      MathBlock(tex: tex, style: style).tailOnly(false)
    }
  }
}

// MARK: - Lists

struct ProseList: View {
  let ordered: Bool
  let start: Int
  let items: [[MDBlock]]
  let style: ProseStyle

  var body: some View {
    // li + li: margin-top 2px
    VStack(alignment: .leading, spacing: Metrics.s0_5) {
      ForEach(Array(items.enumerated()), id: \.offset) { index, item in
        HStack(alignment: .firstTextBaseline, spacing: 0) {
          marker(index)
            .frame(width: style.listIndent, alignment: .trailing)
          VStack(alignment: .leading, spacing: Metrics.s0_5) {
            ForEach(Array(item.enumerated()), id: \.offset) { i, block in
              ProseBlock(block: block, style: style, first: i == 0)
                .tailOnly(i == item.count - 1)
            }
          }
          .frame(maxWidth: .infinity, alignment: .leading)
        }
        // streamdown's li carries py-1
        .padding(.vertical, Metrics.s1)
        .tailOnly(index == items.count - 1)
      }
    }
  }

  /// `li::marker` in fg-3, outside the content box: "1." or a disc, then a space's width before the text
  @ViewBuilder func marker(_ index: Int) -> some View {
    let space = (style.body.size * 0.28).rounded()
    if ordered {
      Text(web: "\(start + index).")
        .textStyle(style.body)
        .foregroundStyle(Palette.fg3)
        .padding(.trailing, space)
        .fixedSize()
    } else {
      // Chrome's disc: a filled circle about a third of an em, centred on the x-height
      Text(web: "\u{2022}")
        .textStyle(style.body)
        .foregroundStyle(.clear)
        .overlay {
          Circle().fill(Palette.fg3)
            .frame(width: (style.body.size * 0.32).rounded(), height: (style.body.size * 0.32).rounded())
        }
        .padding(.trailing, space + (style.body.size * 0.18).rounded())
        .fixedSize()
    }
  }
}

// MARK: - Quote

struct ProseQuote: View {
  let blocks: [MDBlock]
  let style: ProseStyle

  var body: some View {
    var quiet = style
    quiet.inkOverride = Palette.fg2
    return ProseBlocks(blocks: blocks, style: quiet)
      .padding(.vertical, Metrics.gap)
      .padding(.horizontal, Metrics.pad)
      .background(alignment: .leading) {
        RoundedRectangle(cornerRadius: Metrics.rSm).fill(Palette.lineStrong).frame(width: 2)
      }
  }
}

// MARK: - Code

/// CodeSurface: fenced code on the code background inside the conversation ring, scrolling sideways; coloured when the
/// fence names a known grammar (CodeBlock.tsx)
struct CodeSurface: View {
  let text: String
  var lang: String?
  @Environment(\.snapshot) private var snapshot
  @Environment(\.colorScheme) private var scheme
  @State private var colored: Colored?
  @State private var running = Running()

  /// A pass's tokens and the text they were made for
  struct Colored { var text: String; var language: String; var theme: CodeTheme; var lines: [[CodeToken]]; var at: ContinuousClock.Instant }
  struct Pass: Equatable { var text: String; var language: String?; var theme: CodeTheme; var landed: ContinuousClock.Instant? }
  /// Whether a pass is on: one at a time, its result kept even when the text has grown since
  final class Running { var on = false }
  /// CodeBlock.tsx STREAM_SETTLE_MS: a fence that streams asks again this long after its last pass landed
  private static let settle = Duration.milliseconds(120)

  var body: some View {
    let theme = CodeTheme(scheme)
    let language = fenceLanguage(lang)
    let usable = colored.flatMap { $0.language == language && $0.theme == theme ? $0 : nil }
      // The full-length render tokenizes in place, as its tasks may not run before the image is taken
      ?? (snapshot ? language.flatMap { l in
        SyntaxHighlighter.shared.tokenize(text, language: l, theme: theme).map { Colored(text: text, language: l, theme: theme, lines: $0, at: .now) }
      } : nil)
    Scroller(.horizontal) {
      Text(usable == nil ? CJKWidth.attributed(text, size: WebType.mono.size, paragraph: false)
           : codeSource(text, tokenized: usable?.text, lines: usable?.lines, size: WebType.mono.size))
        .textStyle(WebType.mono)
        .foregroundStyle(Palette.fg1)
        .fixedSize()
        .padding(.horizontal, Metrics.pad)
        .padding(.vertical, Metrics.pad - 2)
    }
    .frame(maxWidth: .infinity, alignment: .leading)
    // The CSS border takes room inside the card; the stroke below does not
    .padding(Metrics.hairline)
    .background(Palette.bgCode, in: RoundedRectangle(cornerRadius: Metrics.rLg))
    .overlay(RoundedRectangle(cornerRadius: Metrics.rLg).strokeBorder(Palette.conversationLine))
    // One pass at a time, off the main actor. A pass for a text the fence has since extended still lands (its lines
    // colour the text it covers), and its landing asks again for the text then: dropped by every chunk, a pass of a
    // streaming fence never landed
    .task(id: Pass(text: text, language: language, theme: theme, landed: colored?.at)) {
      guard !snapshot, let language, !running.on,
            !(colored?.text == text && colored?.language == language && colored?.theme == theme) else { return }
      if let last = colored, last.language == language, last.theme == theme, text.utf8.starts(with: last.text.utf8) {
        try? await Task.sleep(until: last.at + Self.settle)
        if Task.isCancelled { return }
      }
      running.on = true
      let source = text
      let lines = await Task.detached(priority: .utility) { SyntaxHighlighter.shared.tokenize(source, language: language, theme: theme) }.value
      running.on = false
      if let lines { colored = Colored(text: source, language: language, theme: theme, lines: lines, at: .now) }
    }
  }
}

// MARK: - Table

struct ProseTable: View {
  let align: [MDAlign]
  let header: [[MDInline]]
  let rows: [[[MDInline]]]
  let style: ProseStyle
  @State private var width: CGFloat = 0

  var body: some View {
    Scroller(.horizontal) {
      Grid(alignment: .leading, horizontalSpacing: 0, verticalSpacing: 0) {
        GridRow {
          ForEach(Array(header.enumerated()), id: \.offset) { i, cell in
            cellView(cell, column: i, head: true, last: rows.isEmpty)
          }
        }
        .background(Palette.hover)
        ForEach(Array(rows.enumerated()), id: \.offset) { r, row in
          GridRow {
            ForEach(Array(row.enumerated()), id: \.offset) { i, cell in
              cellView(cell, column: i, head: false, last: r == rows.count - 1)
            }
          }
        }
      }
      // min-width: 100%
      .frame(minWidth: width, alignment: .leading)
    }
    .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { width = $0 }
    // The frame's 1 px border takes room around the scrollport
    .padding(Metrics.hairline)
    .clipShape(RoundedRectangle(cornerRadius: Metrics.rLg))
    .overlay(RoundedRectangle(cornerRadius: Metrics.rLg).strokeBorder(Palette.conversationLine))
  }

  func cellView(_ cell: [MDInline], column: Int, head: Bool, last: Bool) -> some View {
    InlineText(inlines: cell, style: WebType.text2, ink: head ? Palette.fg2 : style.ink, strongInk: style.strongInk,
               accent: style.accent, weight: head ? .medium : .regular)
      .fixedSize()
      .padding(.vertical, Metrics.gap)
      .padding(.horizontal, Metrics.pad)
      // A table row's cells share its height (vertical-align: top), so every cell's rule sits on the row's bottom;
      // sized to their own text, a shorter cell drew its rule a point higher than its neighbours'
      .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: Alignment(horizontal: alignment(column).horizontal, vertical: .top))
      // border-bottom: 1px, inside the cell's box
      .padding(.bottom, last ? 0 : Metrics.hairline)
      .overlay(alignment: .bottom) { if !last { Rectangle().fill(Palette.line).frame(height: Metrics.hairline) } }
  }

  func alignment(_ column: Int) -> Alignment {
    switch column < align.count ? align[column] : .none {
    case .center: .center
    case .right: .trailing
    default: .leading
    }
  }
}

// MARK: - Math

// MathBlock (KaTeX-faithful `$$` display math) lives in MathTypeset.swift

extension View {
  /// Keeps the stream fade on the last text only; earlier blocks have finished fading by construction
  @ViewBuilder func tailOnly(_ tail: Bool) -> some View {
    if tail { self } else { environment(\.streamFade, nil) }
  }
}
