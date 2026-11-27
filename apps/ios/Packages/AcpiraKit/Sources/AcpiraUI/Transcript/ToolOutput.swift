import AcpiraDesign
import AcpiraModels
import SwiftUI

// MARK: - Command card

/// The `code-output` frame: rounded-md, a hairline, the code background. The CSS border takes layout room and the
/// stroke overlay does not, so the content sits a hairline in on every side
struct CodeOutputFrame: ViewModifier {
  func body(content: Content) -> some View {
    content
      .padding(Metrics.hairline)
      .frame(maxWidth: .infinity, alignment: .leading)
      .background(Palette.bgCode, in: RoundedRectangle(cornerRadius: Metrics.rMd))
      .clipShape(RoundedRectangle(cornerRadius: Metrics.rMd))
      .overlay(RoundedRectangle(cornerRadius: Metrics.rMd).strokeBorder(Palette.line))
  }
}

/// Command output (Codex-style Shell card): the full command behind a muted `$`, then the output
struct TerminalOutput: View {
  let block: ToolCallBlock
  let command: String?
  @Environment(\.codeSheet) private var inSheet

  /// The lines the transcript's card can show. It is cut at --code-output-max and never scrolls there, so a running
  /// command's streamed output is built and laid out only down to the cut, not whole on every patch; one line more
  /// keeps the card past the cut so its fade still shows. The code sheet shows every line
  private static let previewLines = Int((WebMetrics.codeOutputMax / WebType.mono.lineHeight).rounded(.up)) + 1
  private static let sheetLines = 60

  var body: some View {
    let text = outputOf(block)
    if !text.isEmpty || command != nil {
      VStack(alignment: .leading, spacing: 0) {
        if let command { CommandPrompt(command: command, last: text.isEmpty) }
        if !text.isEmpty {
          OutputPane(top: command != nil ? Metrics.gapHalf : Metrics.commandY) {
            if inSheet {
              // One `Text` per piece, as `ToolOutput`'s sheet: a few hundred lines drawn as one came out empty. The
              // lines do not wrap here, so only their count sets a piece's height
              VStack(alignment: .leading, spacing: 0) {
                ForEach(Array(textPieces(text.trimmingTrailingWhitespace(), lines: Self.sheetLines, chars: .max).enumerated()),
                        id: \.offset) { terminalLines($0.element) }
              }
            } else {
              terminalLines(text, limit: Self.previewLines)
            }
          }
        }
      }
      .modifier(CodeOutputFrame())
      .codeViewer(title: command ?? toolVerb(block), copy: [command.map { "$ \($0)" }, text].compactMap { $0 }.joined(separator: "\n"),
                  copyLabel: t("code.copyOutput")) {
        TerminalOutput(block: block, command: command)
      }
    }
  }

  /// npm banners muted, a leading ✓ in the success colour; the first `limit` lines only when given
  func terminalLines(_ text: String, limit: Int? = nil) -> Text {
    var lines = text.trimmingTrailingWhitespace().components(separatedBy: "\n")
    if let limit, lines.count > limit { lines.removeSubrange(limit...) }
    var parts: [Text] = []
    for (i, line) in lines.enumerated() {
      let end = i < lines.count - 1 ? "\n" : ""
      if line.hasPrefix("> ") || line.hasPrefix("Done in ") {
        parts.append(Text(web: line + end, paragraph: false).foregroundStyle(Palette.fg3))
      } else if let check = line.firstIndex(of: "✓"), line[..<check].allSatisfy(\.isWhitespace) {
        parts.append(Text(web: String(line[..<check]), size: WebType.mono.size, paragraph: false))
        parts.append(Text(web: "✓", size: WebType.mono.size, paragraph: false).foregroundStyle(Palette.ok))
        parts.append(Text(web: String(line[line.index(after: check)...]) + end, paragraph: false))
      } else {
        parts.append(Text(web: line + end, paragraph: false))
      }
    }
    return Text.joined(parts)
  }
}

/// Text a non-command tool returned, wrapped (reports and prose, not column-aligned logs)
struct ToolOutput: View {
  let text: String
  @Environment(\.codeSheet) private var inSheet

  /// Lines of the transcript's card, which is cut at --code-output-max and never scrolls (as `TerminalOutput`): one
  /// line more keeps the card past the cut, so its fade still shows
  private static let previewLines = Int((WebMetrics.codeOutputMax / WebType.mono.lineHeight).rounded(.up)) + 1
  /// Characters a wrapped line can hold at most, in the widest column: the card's head keeps as many visual lines as
  /// the line count above even when the text is one long line (a JSON reply)
  private static let lineChars = 100
  /// A sheet piece stays near 2 000 pt at the largest code size: a `Text` around 2 800 pt tall on a 3x screen
  /// already drew nothing (likely the 8 192 px layer limit)
  private static let pieceLines = 30
  private static let pieceChars = 1200

  var body: some View {
    let body = text.trimmingTrailingWhitespace()
    if !body.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
      Group {
        if inSheet {
          // One `Text` per piece: drawn whole, a page of fetched text (20 000 characters, thousands of points tall)
          // drew nothing and showed an empty card
          Scroller(.vertical) {
            LazyVStack(alignment: .leading, spacing: 0) {
              ForEach(Array(textPieces(body, lines: Self.pieceLines, chars: Self.pieceChars).enumerated()), id: \.offset) {
                line($0.element)
              }
            }
            .padding(.horizontal, Metrics.commandX)
            .padding(.vertical, Metrics.commandY)
          }
          .scrollBounceBehavior(.basedOnSize)
        } else {
          // The transcript preview never scrolls; laying it out directly avoids `UIScrollView`'s late
          // self-sizing inside `FoldLayout` (which popped the card in several frames late over rows below).
          // Only its head is drawn, for the same reason as the sheet's pieces
          let head = String(body.prefix(Self.previewLines * Self.lineChars))
            .split(separator: "\n", maxSplits: Self.previewLines, omittingEmptySubsequences: false)
            .prefix(Self.previewLines).joined(separator: "\n")
          CappedPreview(limit: WebMetrics.codeOutputMax) {
            line(head)
              .padding(.horizontal, Metrics.commandX)
              .padding(.vertical, Metrics.commandY)
          }
        }
      }
      .modifier(CodeOutputFrame())
      .codeViewer(title: t("code.copyToolOutput"), copy: body, copyLabel: t("code.copyToolOutput")) { ToolOutput(text: text) }
    }
  }

  private func line(_ piece: String) -> some View {
    Text(web: piece, size: WebType.mono.size)
      .textStyle(WebType.mono)
      .foregroundStyle(Palette.fg2)
      .fixedSize(horizontal: false, vertical: true)
      .frame(maxWidth: .infinity, alignment: .leading)
  }
}

/// `text` cut into pieces of at most `lines` lines and `chars` characters, at line ends where a line fits and inside
/// a longer line otherwise. A blank line never ends a piece, where a `Text` may leave its trailing break undrawn: it
/// starts the next one instead
func textPieces(_ text: String, lines: Int, chars: Int) -> [String] {
  var pieces: [String] = []
  var current: [Substring] = []
  var size = 0
  func close() {
    var carried: [Substring] = []
    while current.count > 1, current.last?.isEmpty == true { carried.insert(current.removeLast(), at: 0) }
    if !current.isEmpty { pieces.append(current.joined(separator: "\n")) }
    current = carried
    size = carried.count
  }
  for var line in text.split(separator: "\n", omittingEmptySubsequences: false) {
    // A line longer than a piece goes out in slices of its own
    while line.count > chars {
      if current.contains(where: { !$0.isEmpty }) { close() }
      // Blank lines carried over go first, a line break each
      let end = line.index(line.startIndex, offsetBy: max(1, chars - current.count))
      current.append(line[..<end])
      close()
      line = line[end...]
    }
    if !current.isEmpty, current.count >= lines || size + line.count > chars { close() }
    current.append(line)
    size += line.count + 1
  }
  if !current.isEmpty { pieces.append(current.joined(separator: "\n")) }
  return pieces
}

/// The scrolling output area: capped by --code-output-max, sideways scrolling for long lines (in the transcript too)
struct OutputPane<Content: View>: View {
  var top: CGFloat
  @ViewBuilder let content: () -> Content
  @Environment(\.codeSheet) private var inSheet

  var body: some View {
    if inSheet {
      Scroller([.vertical, .horizontal]) { styledContent }
        .scrollBounceBehavior(.basedOnSize, axes: [.vertical, .horizontal])
        .scrollIndicators(.hidden)
    } else {
      HorizontalCappedPreview(limit: WebMetrics.codeOutputMax) {
        styledContent
      } content: {
        Scroller(.horizontal) { styledContent.codeCut(inSheet: false) }
          .scrollBounceBehavior(.basedOnSize, axes: .horizontal)
          .scrollIndicators(.hidden)
      }
    }
  }

  private var styledContent: some View {
    content()
      .textStyle(WebType.mono)
      .foregroundStyle(Palette.fg2)
      .fixedSize()
      .padding(.horizontal, Metrics.commandX)
      .padding(.top, top)
      .padding(.bottom, Metrics.commandY)
      .frame(maxWidth: .infinity, alignment: .leading)
  }
}

/// The full command, cut to three lines until expanded; wrapped lines hang under the command, not the `$`
struct CommandPrompt: View {
  let command: String
  var last: Bool
  @State private var expanded = false
  @State private var clipped = false
  @Environment(\.motionEnabled) private var motion
  @Environment(\.holdScroll) private var holdScroll

  var body: some View {
    let mono = WebType.mono
    HStack(alignment: .firstTextBaseline, spacing: 0) {
      Text(web: "$ ", size: WebType.mono.size).accessibilityHidden(true)
      VStack(alignment: .leading, spacing: 0) {
        Group {
          if clipped {
            // The height eases between the cut and the whole command through the layout, as a fold's reveal does.
            // Switching the line limit inside the animation resized the hosted thread row in one step, and the rows
            // above the card jumped and slid back
            LineClamp(progress: expanded ? 1 : 0) {
              Text(web: command, size: WebType.mono.size)
                .fixedSize(horizontal: false, vertical: true)
                .opacity(expanded ? 1 : 0)
                .accessibilityHidden(!expanded)
              cut.opacity(expanded ? 0 : 1).accessibilityHidden(expanded)
            }
            .clipped()
          } else {
            cut
          }
        }
          .background {
            // Measure the uncut height to know whether the cut hides anything
            Text(web: command, size: WebType.mono.size).textStyle(mono).fixedSize(horizontal: false, vertical: true).hidden()
              .onGeometryChange(for: CGFloat.self) { $0.size.height } action: {
                clipped = $0 > CGFloat(Metrics.commandPromptLines) * mono.lineHeight + 1
              }
          }
        if clipped {
          Button {
            toggle()
          } label: {
            Icon(.chevronDown, size: WebMetrics.chevron, strokeWidth: 1.75)
              .rotationEffect(.degrees(expanded ? 180 : 0))
              .padding(.horizontal, Metrics.gapHalf)
              .background(Palette.fg3.opacity(0.14), in: RoundedRectangle(cornerRadius: Metrics.rSm))
          }
          .buttonStyle(.plain)
          .foregroundStyle(Palette.fg3)
          .accessibilityLabel(Text(web: t(expanded ? "code.commandCollapse" : "code.commandExpand")))
        }
      }
    }
    .textStyle(mono)
    .foregroundStyle(Palette.fg3)
    .padding(.horizontal, Metrics.commandX)
    .padding(.top, Metrics.commandY)
    .padding(.bottom, last ? Metrics.commandY : 0)
    .frame(maxWidth: .infinity, alignment: .leading)
    // The chevron alone is far below a finger's size, and a miss opened the code sheet: a cut command expands from
    // anywhere on its lines, ahead of the card's own tap
    .contentShape(Rectangle())
    .gesture(TapGesture().onEnded { toggle() }, including: clipped ? .all : .subviews)
  }

  /// The command cut at `commandPromptLines`
  private var cut: some View {
    Text(web: command, size: WebType.mono.size)
      .lineLimit(Metrics.commandPromptLines)
      .truncationMode(.tail)
      .fixedSize(horizontal: false, vertical: true)
  }

  private func toggle() {
    let next = !expanded
    holdScroll()
    withAnimation(foldAnimation(opening: next, motion: motion)) { expanded = next }
  }
}

/// The whole text (first subview) shown down to a height between the cut text's (second subview) and its own:
/// `FoldLayout` with the cut as its floor. Both sit at the top; the caller clips and cross-fades them
private struct LineClamp: Layout {
  var progress: CGFloat
  var animatableData: CGFloat {
    get { progress }
    set { progress = newValue }
  }

  func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
    let free = ProposedViewSize(width: proposal.width, height: nil)
    let whole = subviews.first?.sizeThatFits(free) ?? .zero
    let cut = subviews.dropFirst().first?.sizeThatFits(free) ?? whole
    let shown = min(1, max(0, progress))
    return CGSize(width: proposal.width ?? max(whole.width, cut.width), height: cut.height + (whole.height - cut.height) * shown)
  }

  func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
    for subview in subviews {
      subview.place(at: bounds.origin, anchor: .topLeading, proposal: ProposedViewSize(width: bounds.width, height: nil))
    }
  }
}

func outputOf(_ b: ToolCallBlock) -> String {
  if let contents = b.contents, !contents.isEmpty {
    let texts = contents.compactMap { if case .text(let t) = $0 { t } else { nil } }
    if !texts.isEmpty { return texts.joined(separator: "\n") }
  }
  switch b.content {
  case .text(let t): return t
  case .list(let items): return items.joined(separator: "\n")
  case .diff(let lines, _): return lines.map(\.code).joined(separator: "\n")
  default: return ""
  }
}

extension String {
  func trimmingTrailingWhitespace() -> String {
    var s = self
    while let last = s.last, last.isWhitespace { s.removeLast() }
    return s
  }
}

// MARK: - Diff

/// DiffBlock: line numbers and signs in a gutter, tinted add / del rows, `···` between hunks. The rows fill at least
/// the card's width (`diff-table min-w-full w-max`), so a tint runs edge to edge however short its line is. Code is
/// coloured by the file's grammar (codeDiffHighlight.ts, the visible segments: the app has no whole-file source)
struct DiffBlockView: View {
  let lines: [DiffLine]
  /// The file, for the code sheet's title and the grammar
  var title = ""
  /// The whole files behind the diff (a page view keeps the path only) and where the diff sits, to ask for them
  var source: DiffSource?
  var locate: DiffLocation?
  @Environment(\.diffSource) private var fetchSource
  @Environment(\.codeSheet) private var inSheet
  @Environment(\.snapshot) private var snapshot
  @Environment(\.motionEnabled) private var motion
  @Environment(\.colorScheme) private var scheme
  @State private var viewport: CGFloat = 0
  @State private var rowWidth: CGFloat = 0
  @State private var colored: Colored?

  /// A pass's tokens, one entry per line of `lines`
  struct Colored { var lines: [DiffLine]; var theme: CodeTheme; var tokens: [[CodeToken]?] }
  struct Pass: Equatable { var lines: [DiffLine]; var title: String; var theme: CodeTheme }

  /// Only a completed pass is read while drawing. The full-length render tokenizes in place because it runs no tasks.
  private func tokens(_ lines: [DiffLine], theme: CodeTheme) -> [[CodeToken]?]? {
    if let colored, colored.theme == theme, colored.lines == lines { return colored.tokens }
    if snapshot { return SyntaxHighlighter.shared.diff(lines, path: title, theme: theme) }
    // Only the bounded preview reads synchronously; full diffs never rebuild their cache keys while drawing.
    return inSheet ? nil : SyntaxHighlighter.shared.cachedDiff(lines, path: title, theme: theme)
  }

  var body: some View {
    let mono = WebType.mono
    // One `ch` of Menlo (0.602 em), the face the desktop draws code in
    let ch = mono.size * 0.602
    let theme = CodeTheme(scheme)
    // The transcript never shows beyond its cap. Do not build or highlight the hidden tail just to clip it later.
    let display = inSheet ? lines : diffPreview(lines)
    let tokens = tokens(display, theme: theme)
    Group {
      if inSheet {
        // The reader owns the whole bounded viewport, even when the diff has only a few lines.
        sheetScroller(display, tokens: tokens, ch: ch)
          .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
      } else {
        // Resolve preview height and minimum row width synchronously on the first pass so opening the fold
        // glides from frame 1 instead of stalling on `UIScrollView` self-sizing and per-row geometry callbacks.
        HorizontalCappedPreview(limit: WebMetrics.codeOutputMax) {
          Color.clear.frame(width: 0, height: diffPreviewNaturalHeight(display))
        } content: {
          GeometryReader { proxy in
            Scroller(.horizontal) {
              VStack(alignment: .leading, spacing: 0) {
                rows(display, tokens: tokens, ch: ch, measureWidth: false)
              }
              .frame(minWidth: proxy.size.width, alignment: .leading)
              .padding(.vertical, Metrics.gapHalf)
              .codeCut(inSheet: false)
            }
            .scrollBounceBehavior(.basedOnSize, axes: .horizontal)
            .scrollIndicators(.hidden)
          }
        }
        .modifier(CodeOutputFrame())
      }
    }
    // Full-source cache lookup and tokenization stay off the main actor; a stale pass must not replace a newer source.
    .task(id: Pass(lines: display, title: title, theme: theme)) {
      guard !snapshot, tokens == nil, codeLanguage(title) != nil else { return }
      let started = ContinuousClock.now
      let title = title
      let result = await Task.detached(priority: .utility) { SyntaxHighlighter.shared.diff(display, path: title, theme: theme) }.value
      guard !Task.isCancelled, let result else { return }
      // Avoid rebuilding every diff row in the middle of an opening fold spring; apply the highlight once settled.
      if !inSheet && motion {
        let elapsed = started.duration(to: .now)
        let settle = Duration.seconds(Motion.foldSettle)
        if elapsed < settle { try? await Task.sleep(for: settle - elapsed) }
        guard !Task.isCancelled else { return }
      }
      colored = Colored(lines: display, theme: theme, tokens: result)
    }
    // CodeBlock.tsx: copy takes the new file, asked for when the view left its text out, else the visible new code.
    // The diff owns its viewport: an outer vertical scroller would make even a lazy table lay out its entire height.
    .codeViewer(title: title, copy: copyText, copyLabel: t(copiesSource ? "code.copySource" : "code.copyVisible"), read: readSource,
                wrapInScrollView: false, edgeToEdge: true) {
      DiffBlockView(lines: lines, title: title, source: source, locate: locate)
    }
  }

  private func sheetScroller(_ display: [DiffLine], tokens: [[CodeToken]?]?, ch: CGFloat) -> some View {
    Scroller([.vertical, .horizontal]) {
      Group {
        if !snapshot {
          LazyVStack(alignment: .leading, spacing: 0) { rows(display, tokens: tokens, ch: ch, measureWidth: true) }
        } else {
          VStack(alignment: .leading, spacing: 0) { rows(display, tokens: tokens, ch: ch, measureWidth: false) }
        }
      }
      // Keep the widest mounted row even after it is recycled, so sideways range never shrinks while reading.
      .frame(minWidth: max(viewport, rowWidth), alignment: .leading)
      .padding(.vertical, Metrics.gapHalf)
    }
    .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { viewport = $0 }
    .scrollBounceBehavior(.basedOnSize, axes: [.vertical, .horizontal])
    .scrollIndicators(.hidden)
  }

  private func rows(_ display: [DiffLine], tokens: [[CodeToken]?]?, ch: CGFloat, measureWidth: Bool) -> some View {
    let mono = WebType.mono
    let rows = display.indices.filter { i in !(display[i].kind == .hunk && (i == 0 || i == display.count - 1)) }
    return ForEach(rows, id: \.self) { i in
      let line = display[i]
      if line.kind == .hunk {
        Text(web: "···")
          .textStyle(WebType.text3)
          .foregroundStyle(Palette.fg3)
          .frame(width: (3.7 + 2.2) * ch, height: Metrics.diffHunk)
          .frame(maxWidth: .infinity, alignment: .leading)
          .overlay(alignment: .top) { Rectangle().fill(Palette.line).frame(height: 1) }
          .overlay(alignment: .bottom) { Rectangle().fill(Palette.line).frame(height: 1) }
          .padding(.vertical, Metrics.gapHalf)
      } else {
        let row = HStack(alignment: .firstTextBaseline, spacing: 0) {
          // diff-gutter: the number right-aligned in --diff-number-width, the sign centred in --diff-sign-width.
          // A context row keeps the sign's room with a space: an empty Text has no baseline, and aligned by its
          // line box's bottom it stretched the row
          Text(web: (line.kind == .del ? line.oldLine : line.newLine).map(String.init) ?? "", size: mono.size)
            .foregroundStyle(Palette.fg3)
            .monospacedDigit()
            .frame(minWidth: 3.7 * ch, alignment: .trailing)
          Text(web: line.kind == .add ? "+" : line.kind == .del ? "−" : " ", size: mono.size)
            .foregroundStyle(line.kind == .add ? Palette.ok : line.kind == .del ? Palette.danger : Palette.fg3)
            .frame(width: 2.2 * ch)
          Group {
            if let runs = tokens?[i], !line.code.isEmpty {
              Text(codeRuns(runs, size: mono.size))
            } else {
              Text(web: line.code.isEmpty ? "\u{200b}" : line.code, size: mono.size)
            }
          }
            .foregroundStyle(Palette.fg1)
            .fixedSize()
            .padding(.trailing, Metrics.pad)
        }
        .textStyle(WebType.codeOutput)
        .fixedSize(horizontal: true, vertical: false)
        Group {
          if measureWidth {
            row.onGeometryChange(for: CGFloat.self) { $0.size.width } action: { rowWidth = max(rowWidth, $0) }
          } else {
            row
          }
        }
        .frame(minHeight: WebMetrics.codeOutputLine)
        .frame(maxWidth: .infinity, alignment: .leading)
        // A plain rectangle: the shape-style background took a rounded container shape and cut the tint's ends
        .background(Rectangle().fill(line.kind == .add ? Palette.diffAdd : line.kind == .del ? Palette.diffDel : .clear))
      }
    }
  }

  private var copyText: String { diffCopyText(lines, source) }

  private var fetchable: Bool {
    if case .omitted? = source, locate != nil, fetchSource != nil { true } else { false }
  }

  private var copiesSource: Bool {
    if case .full? = source { true } else { fetchable }
  }

  /// The new file from the engine; the visible code when it no longer has the call
  private var readSource: (() async -> String)? {
    guard fetchable, let locate, let fetchSource else { return nil }
    let fallback = copyText
    return { await fetchSource(locate)?.newText ?? fallback }
  }
}

// MARK: - Code sheet

extension EnvironmentValues {
  /// Inside the code sheet: a code card grows to its whole height and scrolls there
  @Entry var codeSheet = false
}

/// Non-scrolling transcript preview capped at `limit` on the first layout pass, drawing the bottom fade only when
/// natural height exceeds `limit` (no `@State` measurement loop or late `UIScrollView` sizing inside `FoldLayout`).
private struct CappedPreview<Content: View>: View {
  var limit: CGFloat
  @ViewBuilder let content: () -> Content

  var body: some View {
    CappedBox(limit: limit) {
      content()
      LinearGradient(colors: [Palette.bgCode.opacity(0), Palette.bgCode], startPoint: .top, endPoint: .bottom)
        .allowsHitTesting(false)
    }
    .clipped()
  }
}

private struct CappedBox: Layout {
  var limit: CGFloat

  func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
    guard let content = subviews.first else { return .zero }
    let natural = content.sizeThatFits(ProposedViewSize(width: proposal.width, height: nil))
    return CGSize(width: proposal.width ?? natural.width, height: min(natural.height, limit))
  }

  func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
    guard let content = subviews.first else { return }
    let natural = content.sizeThatFits(ProposedViewSize(width: bounds.width, height: nil))
    content.place(at: bounds.origin, anchor: .topLeading,
                  proposal: ProposedViewSize(width: bounds.width, height: natural.height))
    guard let fade = subviews.dropFirst().first else { return }
    if natural.height > limit {
      let h = min(Metrics.scrollFade, bounds.height)
      fade.place(at: CGPoint(x: bounds.minX, y: bounds.maxY - h), anchor: .topLeading,
                 proposal: ProposedViewSize(width: bounds.width, height: h))
    } else {
      fade.place(at: bounds.origin, anchor: .topLeading, proposal: ProposedViewSize(width: 0, height: 0))
    }
  }
}

/// Horizontally scrolling transcript preview whose vertical height comes synchronously from `ruler` on frame 1,
/// keeping `FoldLayout` reveals smooth without waiting for UIKit's `UIScrollView` to self-size.
private struct HorizontalCappedPreview<Ruler: View, Content: View>: View {
  var limit: CGFloat
  @ViewBuilder let ruler: () -> Ruler
  @ViewBuilder let content: () -> Content

  var body: some View {
    HorizontalCappedLayout(limit: limit) {
      ruler()
        .hidden()
        .allowsHitTesting(false)
        .accessibilityHidden(true)
      content()
      LinearGradient(colors: [Palette.bgCode.opacity(0), Palette.bgCode], startPoint: .top, endPoint: .bottom)
        .allowsHitTesting(false)
    }
    .clipped()
  }
}

private struct HorizontalCappedLayout: Layout {
  var limit: CGFloat

  func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
    guard let ruler = subviews.first else { return .zero }
    let natural = ruler.sizeThatFits(ProposedViewSize(width: nil, height: nil))
    return CGSize(width: proposal.width ?? natural.width, height: min(natural.height, limit))
  }

  func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
    guard subviews.count >= 2 else { return }
    let natural = subviews[0].sizeThatFits(ProposedViewSize(width: nil, height: nil))
    let height = min(natural.height, limit)
    subviews[0].place(at: bounds.origin, anchor: .topLeading, proposal: ProposedViewSize(width: 0, height: 0))
    subviews[1].place(at: bounds.origin, anchor: .topLeading, proposal: ProposedViewSize(width: bounds.width, height: height))
    guard subviews.count >= 3 else { return }
    if natural.height > limit {
      let h = min(Metrics.scrollFade, height)
      subviews[2].place(at: CGPoint(x: bounds.minX, y: bounds.maxY - h), anchor: .topLeading,
                        proposal: ProposedViewSize(width: bounds.width, height: h))
    } else {
      subviews[2].place(at: bounds.origin, anchor: .topLeading, proposal: ProposedViewSize(width: 0, height: 0))
    }
  }
}

/// Natural height of `display` in the transcript diff preview (`VStack` rows + vertical padding), matching
/// `DiffBlockView.rows` synchronously so `HorizontalCappedPreview` never waits on `UIScrollView` measurement.
func diffPreviewNaturalHeight(_ display: [DiffLine]) -> CGFloat {
  var total = Metrics.gapHalf * 2
  for (i, line) in display.enumerated() {
    if line.kind == .hunk {
      guard i != 0, i != display.count - 1 else { continue }
      total += Metrics.diffHunk + Metrics.gapHalf * 2
    } else {
      total += WebMetrics.codeOutputLine
    }
  }
  return total
}

extension View {
  /// The transcript's cut inside a sideways-only scroller: the content keeps its top --code-output-max, so the strip
  /// is no taller than the card and a swipe across it scrolls long lines
  @ViewBuilder func codeCut(inSheet: Bool) -> some View {
    if inSheet {
      self
    } else {
      frame(maxHeight: WebMetrics.codeOutputMax, alignment: .top).clipped()
    }
  }

  /// A tap on a code card opens the whole card in the code sheet; inside the sheet the card stays as it is
  /// read: the text copy takes when it has to be fetched first (a diff's whole file); `copy` until then
  /// wrapInScrollView: false when the content owns a viewport, so its lazy rows see a bounded height
  func codeViewer<Full: View>(title: String, copy: String, copyLabel: String, read: (() async -> String)? = nil,
                              wrapInScrollView: Bool = true, edgeToEdge: Bool = false,
                              @ViewBuilder full: @escaping () -> Full) -> some View {
    modifier(CodeViewer(title: title, copy: copy, copyLabel: copyLabel, read: read,
                        wrapInScrollView: wrapInScrollView, edgeToEdge: edgeToEdge, full: full))
  }
}

private struct CodeViewer<Full: View>: ViewModifier {
  var title: String
  var copy: String
  var copyLabel: String
  var read: (() async -> String)?
  var wrapInScrollView: Bool
  var edgeToEdge: Bool
  let full: () -> Full
  @Environment(\.codeSheet) private var inSheet
  @Environment(\.initialPopup) private var initialPopup
  @State private var shown = false

  func body(content: Content) -> some View {
    if !inSheet {
      content
        .contentShape(Rectangle())
        .onTapGesture { shown = true }
        .sheet(isPresented: $shown) {
          CodeSheet(title: title, copy: copy, copyLabel: copyLabel, read: read,
                    wrapInScrollView: wrapInScrollView, edgeToEdge: edgeToEdge, content: full)
        }
        .task {
          // `-popup code` opens the first card that shows, for screenshots
          guard initialPopup == "code", !CodeSheetProbe.claimed else { return }
          CodeSheetProbe.claimed = true
          try? await Task.sleep(for: .seconds(1))
          shown = true
        }
    } else {
      content
    }
  }
}

/// `-popup code`: whether a card has already taken the launch's one sheet
enum CodeSheetProbe {
  nonisolated(unsafe) static var claimed = false
}

/// The code sheet: one card whole, both ways scrollable, under a title naming it, the
/// system's close button and copy in its bar
struct CodeSheet<Content: View>: View {
  var title: String
  var copy: String
  var copyLabel: String
  var read: (() async -> String)? = nil
  var wrapInScrollView = true
  /// Readers with their own viewport fill the sheet without an inset card.
  var edgeToEdge = false
  var detents: Set<PresentationDetent> = [.medium, .large]
  var copyEnabled = true
  /// Optional raw / rendered toggle; copying always uses the original text.
  var markdownPreview: Binding<Bool>? = nil
  let content: () -> Content
  @Environment(\.dismiss) private var dismiss
  @State private var copied = false

  var body: some View {
    NavigationStack {
      Group {
        if wrapInScrollView {
          ScrollView(.vertical) { sheetContent }
        } else {
          sheetContent
        }
      }
      .frame(maxWidth: .infinity, maxHeight: edgeToEdge ? .infinity : nil, alignment: .topLeading)
      .background(edgeToEdge ? Palette.bgCode : Palette.bg0)
      .navigationTitle(title)
      .navigationBarTitleDisplayModeInline()
      .toolbar {
        ToolbarItem(placement: .cancellationAction) {
          CloseButton { dismiss() }
        }
        ToolbarItemGroup(placement: .primaryAction) {
          if let markdownPreview {
            Button { markdownPreview.wrappedValue.toggle() } label: {
              Image(systemName: markdownPreview.wrappedValue ? "book.fill" : "book")
            }
            .barButtonBase()
            .accessibilityLabel(Text(web: t("panel.markdownPreview")))
            .accessibilityAddTraits(markdownPreview.wrappedValue ? .isSelected : [])
          }
          Button {
            guard let read else {
              UIPasteboardBridge.copy(copy)
              Haptics.play(.copy)
              copied = true
              return
            }
            Task {
              UIPasteboardBridge.copy(await read())
              Haptics.play(.copy)
              copied = true
            }
          } label: {
            Image(systemName: copied ? "checkmark" : "doc.on.doc")
          }
          .barButtonBase()
          .accessibilityLabel(Text(web: copied ? t("code.copied") : copyLabel))
          .disabled(!copyEnabled)
        }
      }
    }
    .presentationDetents(detents)
    .task(id: copied) {
      guard copied else { return }
      try? await Task.sleep(for: .milliseconds(1500))
      copied = false
    }
  }

  private var sheetContent: some View {
    content()
      .environment(\.codeSheet, true)
      .padding(.horizontal, edgeToEdge ? 0 : Metrics.pad)
      .padding(.vertical, edgeToEdge ? 0 : Metrics.s2)
  }
}

/// Enough rows to paint past the transcript's cut, including a leading hunk header (which is not drawn).
/// Keep the original array for the sheet and copy; only the preview's layout and highlighter use this prefix.
nonisolated func diffPreview(_ lines: [DiffLine]) -> [DiffLine] {
  // Hunk separators can be shorter than code rows; even a card full of them must still paint past the fade.
  let shortest = min(WebMetrics.codeOutputLine, Metrics.diffHunk + Metrics.gapHalf * 2)
  let limit = Int((WebMetrics.codeOutputMax / shortest).rounded(.up)) + 1
  return Array(lines.prefix(limit + (lines.first?.kind == .hunk ? 1 : 0)))
}

// MARK: - Diff sources

/// Where a diff sits in the session: its tool call and which of the call's diffs (CodeBlock.tsx DiffLocation)
nonisolated struct DiffLocation: Hashable, Sendable {
  var toolCallId: String
  var nth: Int
}

/// The location of the diff with these lines, counted among the call's diffs (`contents`, or `content` alone)
nonisolated func diffLocation(_ block: ToolCallBlock, _ lines: [DiffLine]) -> DiffLocation? {
  let diffs = (block.contents ?? block.content.map { [$0] } ?? []).compactMap { item -> [DiffLine]? in
    if case .diff(let lines, _) = item { lines } else { nil }
  }
  return diffs.firstIndex(of: lines).map { DiffLocation(toolCallId: block.id, nth: $0) }
}

/// codeDiff.ts diffCopyText: the new file when the diff carries it, else the visible new code without signs
nonisolated func diffCopyText(_ lines: [DiffLine], _ source: DiffSource?) -> String {
  if case .full(let full)? = source { return full.newText }
  return lines.filter { $0.kind == .add || $0.kind == .ctx }.map(\.code).joined(separator: "\n")
}

extension EnvironmentValues {
  /// Asks the engine for a diff's whole files in the chat's session (nil: no engine, or it no longer has the call)
  @Entry var diffSource: ((DiffLocation) async -> FullDiffSource?)? = nil
}
