import AcpiraDesign
import AcpiraMarkdown
import AcpiraModels
import SwiftUI

// QuoteToolbar.tsx and Quotes.tsx on a touch screen. The phone has no text selection in the transcript, so a long press
// on a block of the agent's reply (a paragraph, a list, a code block …) offers "Add to chat", which stages the block as a
// `quote` attachment; the quotes wait in the composer's strip as one "N annotations" chip whose card lists them, each
// with its optional comment.

/// What a long press on a reply block quotes into. Every one compares equal: a chat's transcript only ever quotes into
/// that chat, and the fresh closure each render of the chat builds must not count as a change that redraws every
/// reply's prose
struct QuoteAction: Equatable {
  let run: (String) -> Void
  static func == (_: Self, _: Self) -> Bool { true }
}

extension EnvironmentValues {
  /// Set along the agent's reply: a long press on one of its blocks quotes it into the composer
  @Entry var quoteProse: QuoteAction? = nil
}

extension View {
  /// The reply block's long-press menu: quote it into the composer, or copy it
  @ViewBuilder func quoteMenu(_ block: MDBlock, _ quote: QuoteAction?) -> some View {
    if let quote, block != .rule {
      contextMenu {
        Button(t("quote.add"), systemImage: "text.quote") { quote.run(QuoteText.of(block)) }
        Button(t("common.copy"), systemImage: "doc.on.doc") { UIPasteboardBridge.copy(QuoteText.of(block)) }
      }
    } else {
      self
    }
  }
}

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
/// takes a comment or leaves
struct QuoteChip: View {
  @Binding var items: [Attachment]
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
      QuoteCard(items: $items, places: quotes) { open = false }
        .presentationCompactAdaptation(.popover)
    }
    .onAppear {
      // `-popup quotes`: the card open on launch (screenshots)
      if initialPopup == "quotes" { open = true }
    }
  }
}

/// The quotes' card: numbered entries, each its text (five lines at most), a comment field and its removal
private struct QuoteCard: View {
  @Binding var items: [Attachment]
  let places: [Int]
  var onEmpty: () -> Void

  var body: some View {
    ScrollView {
      VStack(alignment: .leading, spacing: Metrics.s3) {
        ForEach(Array(places.enumerated()), id: \.element) { n, at in
          if case .quote(let text, _) = items[at] {
            HStack(alignment: .firstTextBaseline, spacing: Metrics.gapHalf) {
              Text(web: "\(n + 1).", paragraph: false).textStyle(TypeScale.text3).foregroundStyle(Palette.fg3).monospacedDigit()
              VStack(alignment: .leading, spacing: Metrics.s0_5) {
                Text(web: t("quote.selected"), paragraph: false).textStyle(TypeScale.text3).foregroundStyle(Palette.fg3)
                Text(web: text)
                  .textStyle(TypeScale.text2)
                  .foregroundStyle(Palette.fg1)
                  .lineLimit(5)
                  .fixedSize(horizontal: false, vertical: true)
                TextField(text: comment(at), axis: .vertical) {
                  Text(web: t("quote.commentPlaceholder")).foregroundStyle(Palette.fg3)
                }
                .textStyle(TypeScale.text2)
                .foregroundStyle(Palette.fgStrong)
                .lineLimit(1...4)
                .accessibilityLabel(Text(web: t("quote.comment")))
                .padding(.top, Metrics.s1)
              }
              .frame(maxWidth: .infinity, alignment: .leading)
              IconButton(.trash2, size: .sm, label: t("quote.remove")) { remove(at) }
                .alignmentGuide(.firstTextBaseline) { $0[VerticalAlignment.center] }
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
