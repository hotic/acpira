import AcpiraDesign
import AcpiraMarkdown
import AcpiraModels
import SwiftUI

// QuoteToolbar.tsx and Quotes.tsx on a touch screen. A block's long-press menu keeps its whole-source actions;
// double tap or Select text enters native range selection, whose menu quotes just the selected text.
// The quotes wait in the composer's strip as one "N annotations" chip, each with its optional comment.

/// What selected reply text quotes into. Every one compares equal: a chat's transcript only ever quotes into
/// that chat, and the fresh closure each render of the chat builds must not count as a change that redraws every
/// reply's prose
struct QuoteAction: Equatable {
  let run: (String) -> Void
  static func == (_: Self, _: Self) -> Bool { true }
}

extension EnvironmentValues {
  /// Set along the agent's reply: its selection menu quotes the selected text into the composer
  @Entry var quoteProse: QuoteAction? = nil
  /// When true (hosted `TranscriptCollection` cells), per-block SwiftUI `.contextMenu` modifiers are omitted and
  /// handled by one shared `UIContextMenuInteraction` on the collection view using `rowHitTargets`.
  @Entry var deferQuoteMenu = false
  /// Per-cell block geometry registry populated by `BlockStackLayout` for deferred context menus and double-tap selection.
  @Entry var rowHitTargets: RowHitTargets? = nil
  /// Set inside a nested block (e.g. `ProseQuote`) so inner `ProseBlocks` do not overwrite the outer block's hit entry.
  @Entry var insideQuotedBlock = false
}

/// Named coordinate space on each hosted collection row so `ProseBlocks` / `ProseRun` map block frames into cell space.
enum RowHitSpace {
  nonisolated static let name = "acpira.rowHitSpace"
}

/// One top-level reply block's geometry and menu metadata within a hosted cell.
struct BlockHitEntry {
  /// Block passed to Quote / Copy (the whole list block for a sliced list run)
  var quoteBlock: MDBlock
  /// Block rendered in this row (the slice or single block) for link probing and preview
  var renderedBlock: MDBlock
  /// Frame local to its `BlockStackLayout` container
  var localFrame: CGRect
  var offersSelection: Bool
}

/// Non-observable per-row storage recording top-level reply block frames during layout, without adding any
/// `UIView`, `GeometryReader`, or SwiftUI `.contextMenu` per paragraph.
@MainActor final class RowHitTargets {
  struct Bucket {
    var origin: CGPoint = .zero
    var entries: [BlockHitEntry] = []
  }

  private var buckets: [ObjectIdentifier: Bucket] = [:]
  /// Turn index and tap-to-reveal flag for older replies (`ReplyTap`), recorded by the row's content
  var turnIndex: Int?
  var canTapActions = false

  func setOrigin(_ origin: CGPoint, for key: ObjectIdentifier) {
    buckets[key, default: Bucket()].origin = origin
  }

  func setEntries(_ entries: [BlockHitEntry], for key: ObjectIdentifier) {
    buckets[key, default: Bucket()].entries = entries
  }

  func removeBucket(_ key: ObjectIdentifier) {
    buckets[key] = nil
  }

  func clear() {
    buckets.removeAll(keepingCapacity: true)
    turnIndex = nil
    canTapActions = false
  }

  /// Resolves the reply block at `pointInRow` (in the `HostedRow` / cell content coordinate space).
  func hit(at pointInRow: CGPoint) -> (entry: BlockHitEntry, frameInRow: CGRect)? {
    for bucket in buckets.values {
      for entry in bucket.entries {
        let frame = entry.localFrame.offsetBy(dx: bucket.origin.x, dy: bucket.origin.y)
        // Slight vertical pad covers the block gap between paragraphs so pressing near a line edge still hits
        if frame.insetBy(dx: -4, dy: -4).contains(pointInRow) {
          return (entry, frame)
        }
      }
    }
    return nil
  }

  nonisolated static func offersSelection(_ block: MDBlock) -> Bool {
    switch block {
    case .math, .rule, .code(lang: "mermaid", _): false
    case .paragraph(let inlines): !inlines.contains { if case .image = $0 { true } else { false } }
    default: true
    }
  }
}

/// Lightweight vertical stack `Layout` matching `VStack(alignment: .leading, spacing:)` that also records each child's
/// placed bounds into `RowHitTargets` in the same layout pass — zero extra views, gestures, or preference passes.
/// Caches subview measurements between `sizeThatFits` and `placeSubviews` so every paragraph is measured once per width.
struct BlockStackLayout: Layout {
  var spacing: CGFloat
  /// Non-owning record closure run during `placeSubviews`; nil behaves identically to `VStack`
  var onPlace: (@Sendable ([CGRect]) -> Void)? = nil

  struct Cache {
    var hasWidth = false
    var width: CGFloat? = nil
    var sizes: [CGSize] = []
    var totalSize: CGSize = .zero
  }

  func makeCache(subviews: Subviews) -> Cache { Cache() }

  func updateCache(_ cache: inout Cache, subviews: Subviews) {
    cache.hasWidth = false
    cache.sizes.removeAll(keepingCapacity: true)
  }

  private func measure(width: CGFloat?, subviews: Subviews, cache: inout Cache) {
    cache.sizes.removeAll(keepingCapacity: true)
    cache.sizes.reserveCapacity(subviews.count)
    var totalHeight: CGFloat = 0
    var maxWidth: CGFloat = 0
    let childProposal = ProposedViewSize(width: width, height: nil)
    for (i, subview) in subviews.enumerated() {
      let size = subview.sizeThatFits(childProposal)
      cache.sizes.append(size)
      if i > 0 { totalHeight += spacing }
      totalHeight += size.height
      if size.width > maxWidth { maxWidth = size.width }
    }
    cache.width = width
    cache.hasWidth = true
    cache.totalSize = CGSize(width: width ?? maxWidth, height: totalHeight)
  }

  func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout Cache) -> CGSize {
    if !cache.hasWidth || cache.width != proposal.width || cache.sizes.count != subviews.count {
      measure(width: proposal.width, subviews: subviews, cache: &cache)
    }
    return cache.totalSize
  }

  func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout Cache) {
    if !cache.hasWidth || cache.width != bounds.width || cache.sizes.count != subviews.count {
      measure(width: bounds.width, subviews: subviews, cache: &cache)
    }
    var y = bounds.minY
    var frames: [CGRect] = []
    if onPlace != nil { frames.reserveCapacity(subviews.count) }
    for (i, subview) in subviews.enumerated() {
      let size = cache.sizes[i]
      if i > 0 { y += spacing }
      subview.place(at: CGPoint(x: bounds.minX, y: y), anchor: .topLeading,
                    proposal: ProposedViewSize(width: bounds.width, height: size.height))
      if onPlace != nil {
        frames.append(CGRect(x: 0, y: y - bounds.minY, width: bounds.width, height: size.height))
      }
      y += size.height
    }
    onPlace?(frames)
  }
}

/// Long-press preview card matching `QuoteMenu`'s original styling, rendered on demand by `TranscriptCollection`
struct QuotePreviewCard: View {
  let text: String

  var body: some View {
    Text(web: text)
      .textStyle(WebType.text1)
      .foregroundStyle(Palette.fg1)
      .lineLimit(Metrics.messagePreviewLines)
      .truncationMode(.tail)
      .frame(width: Metrics.popLg, alignment: .leading)
      .fixedSize(horizontal: false, vertical: true)
      .padding(Metrics.s3)
      .background(Palette.bg2, in: RoundedRectangle(cornerRadius: Metrics.rCardRound))
  }
}

/// Stable reference token giving each mounted `ProseBlocks` / `ProseRun` its own bucket key in `RowHitTargets`
final class HitBucketToken {}

extension View {
  /// The original block actions and an explicit entry into native range selection.
  @ViewBuilder func quoteMenu(_ block: MDBlock, _ quote: QuoteAction?, deferred: Bool = false) -> some View {
    if !deferred, let quote, block != .rule {
      #if canImport(UIKit)
      modifier(QuoteMenu(block: block, quote: quote))
      #else
      contextMenu {
        Button(t("quote.add"), systemImage: "text.quote") { quote.run(QuoteText.of(block)) }
        Button(t("common.copy"), systemImage: "doc.on.doc") { UIPasteboardBridge.copy(QuoteText.of(block)) }
      }
      #endif
    } else {
      self
    }
  }
}

#if canImport(UIKit)
private struct QuoteMenu: ViewModifier {
  let block: MDBlock
  let quote: QuoteAction
  @State private var selection = TextSelectionScope()
  @Environment(\.textSelectionScope) private var parentSelection

  func body(content: Content) -> some View {
    if parentSelection != nil {
      // A list or block quote keeps its original whole-block menu, including its nested paragraphs.
      content
    } else {
      content
        .environment(\.textSelectionScope, selection)
        .contextMenu {
          if !selection.active {
            // A press on a web link leads with the link: a tap already reads it in the app, so the menu offers
            // the way out and the address itself
            if let link = selection.link {
              Section(link.absoluteString) {
                Button(t("browser.openSystem"), systemImage: "safari") { WebPage.openOutside(link) }
                Button(t("mobile.link.copy"), systemImage: "link") { UIPasteboardBridge.copy(link.absoluteString) }
              }
            }
            Button(t("quote.add"), systemImage: "text.quote") { quote.run(QuoteText.of(block)) }
            Button(t("common.copy"), systemImage: "doc.on.doc") { UIPasteboardBridge.copy(QuoteText.of(block)) }
            // Diagrams, formulas and image-only blocks have no native text surface.
            if offersSelection {
              Button(t("history.selectText"), systemImage: "text.cursor") { selection.select() }
            }
          }
        } preview: {
          if offersSelection {
            // Only the preview is shortened. Copy and quote still use the complete original block.
            Text(web: selection.previewText ?? QuoteText.of(block))
              .textStyle(WebType.text1)
              .foregroundStyle(Palette.fg1)
              .lineLimit(Metrics.messagePreviewLines)
              .truncationMode(.tail)
              .frame(width: Metrics.popLg, alignment: .leading)
              .fixedSize(horizontal: false, vertical: true)
              .padding(Metrics.s3)
              .background(Palette.bg2, in: RoundedRectangle(cornerRadius: Metrics.rCardRound))
          } else {
            content.background(Palette.bg0)
          }
        }
    }
  }

  private var offersSelection: Bool {
    switch block {
    case .math, .rule, .code(lang: "mermaid", _): false
    case .paragraph(let inlines): !holdsImage(inlines)
    default: true
    }
  }
}
#endif

/// The plain text a quoted block carries: what a selection over it would copy
enum QuoteText {
  static func of(_ block: MDBlock) -> String {
    switch block {
    case .paragraph(let inlines), .heading(_, let inlines):
      return Markdown.plain(inlines)
    case .list(let ordered, let start, let items):
      return items.enumerated().map { n, item in
        (ordered ? "\(start + n). " : "- ") + item.map(of).joined(separator: "\n")
      }.joined(separator: "\n")
    case .quote(let inner):
      return inner.map(of).joined(separator: "\n\n")
    case .code(_, let text), .math(let text):
      return text
    case .table(_, let header, let rows):
      return ([header] + rows).map { $0.map(Markdown.plain).joined(separator: " | ") }.joined(separator: "\n")
    case .rule:
      return ""
    }
  }
}

/// The staged quotes as one chip (Quotes.tsx QuoteChip): a tap opens their card, where each one shows its text and
/// takes a comment or leaves. Read-only on a sent or queued prompt: the card is a receipt of the quotes and comments
struct QuoteChip: View {
  @Binding var items: [Attachment]
  var readOnly = false
  @State private var open = false
  @Environment(\.initialPopup) private var initialPopup

  /// The quotes' places among the staged attachments
  private var quotes: [Int] { items.indices.filter { if case .quote = items[$0] { true } else { false } } }

  var body: some View {
    let count = quotes.count
    Button { open = true } label: {
      HStack(spacing: Metrics.s1) {
        Icon(.messageSquareText, size: WebMetrics.icon, strokeWidth: 1.5).foregroundStyle(Palette.fg3)
        Text(web: count == 1 ? t("quote.chipOne") : t("quote.chip", ["n": count]), paragraph: false).lineLimit(1)
      }
      // AttachmentPill's chip
      .textStyle(WebType.text3, weight: .medium)
      .foregroundStyle(Palette.fg2)
      .padding(.horizontal, Metrics.s2)
      .frame(height: WebMetrics.ctlSm)
      .background(RoundedRectangle(cornerRadius: Metrics.rSm).fill(Palette.chip))
    }
    .buttonStyle(.plain)
    .popover(isPresented: $open) {
      QuoteCard(items: $items, readOnly: readOnly) { open = false }
        .presentationCompactAdaptation(.popover)
    }
    .onAppear {
      // `-popup quotes`: the composer's card open on launch (screenshots)
      if initialPopup == "quotes" && !readOnly { open = true }
    }
  }
}

/// The quotes' card: numbered entries, each its text (five lines at most), a comment field and its removal
private struct QuoteCard: View {
  @Binding var items: [Attachment]
  var readOnly = false
  var onEmpty: () -> Void

  /// The quotes' places, read from the live items: a snapshot from the chip goes stale once a removal shortens
  /// the array, and the next pass would index past its end
  private var places: [Int] { items.indices.filter { if case .quote = items[$0] { true } else { false } } }

  var body: some View {
    ScrollView {
      VStack(alignment: .leading, spacing: Metrics.s3) {
        ForEach(Array(places.enumerated()), id: \.element) { n, at in
          if items.indices.contains(at), case .quote(let text, let note) = items[at] {
            HStack(alignment: .firstTextBaseline, spacing: Metrics.gapHalf) {
              Text(web: "\(n + 1).", paragraph: false).textStyle(TypeScale.text3).foregroundStyle(Palette.fg3).monospacedDigit()
              VStack(alignment: .leading, spacing: Metrics.s0_5) {
                Text(web: t("quote.selected"), paragraph: false).textStyle(TypeScale.text3).foregroundStyle(Palette.fg3)
                Text(web: text)
                  .textStyle(TypeScale.text2)
                  .foregroundStyle(Palette.fg1)
                  .lineLimit(5)
                  .fixedSize(horizontal: false, vertical: true)
                if readOnly {
                  // A sent quote shows its comment under a caption, as the desktop's receipt does
                  if let note {
                    Text(web: t("quote.comment"), paragraph: false).textStyle(TypeScale.text3).foregroundStyle(Palette.fg3)
                      .padding(.top, Metrics.s2)
                    Text(web: note)
                      .textStyle(TypeScale.text2)
                      .foregroundStyle(Palette.fg1)
                      .fixedSize(horizontal: false, vertical: true)
                  }
                } else {
                  TextField(text: comment(at), axis: .vertical) {
                    Text(web: t("quote.commentPlaceholder")).foregroundStyle(Palette.fg3)
                  }
                  .textStyle(TypeScale.text2)
                  .foregroundStyle(Palette.fgStrong)
                  .lineLimit(1...4)
                  .accessibilityLabel(Text(web: t("quote.comment")))
                  .padding(.top, Metrics.s1)
                }
              }
              .frame(maxWidth: .infinity, alignment: .leading)
              if !readOnly {
                IconButton(.trash2, size: .sm, label: t("quote.remove")) { remove(at) }
                  .alignmentGuide(.firstTextBaseline) { $0[VerticalAlignment.center] }
              }
            }
          }
        }
      }
      .padding(Metrics.s3)
    }
    .scrollBounceBehavior(.basedOnSize)
    .frame(width: Metrics.popLg)
    .frame(maxHeight: Metrics.popLg)
  }

  /// A quote's comment as the field edits it; cleared, the quote goes out without one
  private func comment(_ at: Int) -> Binding<String> {
    Binding {
      guard items.indices.contains(at), case .quote(_, let comment) = items[at] else { return "" }
      return comment ?? ""
    } set: { value in
      guard items.indices.contains(at), case .quote(let text, _) = items[at] else { return }
      items[at] = .quote(text: text, comment: value.isEmpty ? nil : value)
    }
  }

  private func remove(_ at: Int) {
    guard items.indices.contains(at) else { return }
    let last = places.count == 1
    items.remove(at: at)
    if last { onEmpty() }
  }
}
