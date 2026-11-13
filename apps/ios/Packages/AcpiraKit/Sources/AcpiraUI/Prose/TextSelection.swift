import AcpiraDesign
import AcpiraMarkdown
import SwiftUI
#if canImport(UIKit)
import UIKit

/// A native selection surface over the existing renderer. UIKit owns the handles and edit menu, while SwiftUI
/// keeps drawing the text, inline chips and streaming fades without changing the transcript's layout.
struct TextSelectionSurface: UIViewRepresentable {
  let text: NSAttributedString
  var actions: [SelectionAction] = []
  /// A sent prompt's command and summon pills, drawn over the SwiftUI text that colours their glyphs (PromptMarks.swift)
  var marks: PromptMarkNames?
  @Environment(\.textSelectionScope) private var selection
  @Environment(\.quoteProse) private var quote
  @Environment(\.holdScroll) private var holdScroll
  @Environment(\.openURL) private var openURL

  func makeUIView(context: Context) -> SelectionTextView {
    let view = SelectionTextView()
    view.isEditable = false
    view.isSelectable = true
    view.isScrollEnabled = false
    view.showsVerticalScrollIndicator = false
    view.showsHorizontalScrollIndicator = false
    view.isAccessibilityElement = true
    view.backgroundColor = .clear
    view.tintColor = .systemBlue
    view.textContainerInset = .zero
    view.textContainer.lineFragmentPadding = 0
    view.linkTextAttributes = [.foregroundColor: UIColor.clear]
    view.delegate = context.coordinator
    view.addInteraction(view.selectionMenu)
    return view
  }

  func updateUIView(_ view: SelectionTextView, context: Context) {
    context.coordinator.surface = self
    view.selectionScope = selection
    if selection?.target == nil { selection?.target = view }
    view.actions = actions
    if let quote {
      view.actions.insert(SelectionAction(title: t("quote.add"), image: "text.quote", run: quote.run), at: 0)
    }
    if let marks {
      context.coordinator.painter.names = marks
      context.coordinator.painter.attach(view)
    }
    guard !view.attributedText.isEqual(to: text) else { return }
    let selected = view.selectedRange
    view.attributedText = text
    view.accessibilityLabel = selectionText(text, range: NSRange(location: 0, length: text.length))
    // Streaming may extend a selected paragraph. Never replace a user's range with the incoming tail.
    if selected.length > 0, NSMaxRange(selected) <= text.length { view.selectedRange = selected }
  }

  func sizeThatFits(_ proposal: ProposedViewSize, uiView: SelectionTextView, context: Context) -> CGSize? {
    guard let width = proposal.width, let height = proposal.height else { return nil }
    return CGSize(width: width, height: height)
  }

  func makeCoordinator() -> Coordinator { Coordinator(surface: self) }

  static func dismantleUIView(_ view: SelectionTextView, coordinator: Coordinator) { coordinator.painter.detach() }

  final class Coordinator: NSObject, UITextViewDelegate {
    var surface: TextSelectionSurface
    lazy var painter = PromptMarkPainter(tints: false)
    init(surface: TextSelectionSurface) { self.surface = surface }

    func textViewDidEndEditing(_ textView: UITextView) {
      textView.selectedRange = NSRange(location: 0, length: 0)
      surface.selection?.active = false
    }

    func textViewDidChangeSelection(_ textView: UITextView) {
      surface.selection?.active = textView.selectedRange.length > 0
      guard textView.selectedRange.length > 0 else { return }
      surface.holdScroll()
      // A selection is a reading gesture even while the agent streams. The fold hold deliberately keeps a live
      // turn following, so release that anchor here rather than letting new text pull the handles away.
      var parent = textView.superview
      while let view = parent {
        if let transcript = view as? TranscriptCollectionView {
          transcript.releaseReaderAnchor()
          transcript.maintainsBottomAnchor = false
          break
        }
        parent = view.superview
      }
    }

    func textView(_ textView: UITextView, editMenuForTextIn range: NSRange,
                  suggestedActions: [UIMenuElement]) -> UIMenu? {
      guard let view = textView as? SelectionTextView else { return nil }
      return view.menu(for: range, suggestedActions: suggestedActions)
    }

    func textView(_ textView: UITextView, primaryActionFor textItem: UITextItem,
                  defaultAction: UIAction) -> UIAction? {
      guard case .link(let url) = textItem.content else { return defaultAction }
      return UIAction { [weak self] _ in self?.surface.openURL(url) }
    }
  }
}

struct SelectionAction {
  var title: String
  var image: String
  var run: (String) -> Void
}

/// One message or Markdown block owns its menu; its last touched paragraph supplies the native range selection.
@Observable final class TextSelectionScope {
  var active = false
  var previewText: String?
  @ObservationIgnored weak var target: SelectionTextView?

  func select() {
    // Let the message context menu dismiss before presenting the native handles and edit menu.
    DispatchQueue.main.async { [weak self] in self?.target?.beginSelection() }
  }
}

extension EnvironmentValues {
  @Entry var textSelectionScope: TextSelectionScope?
}

final class SelectionTextView: UITextView, UIEditMenuInteractionDelegate {
  var actions: [SelectionAction] = []
  weak var selectionScope: TextSelectionScope?
  lazy var selectionMenu = UIEditMenuInteraction(delegate: self)
  private var previewOffset = 0

  /// Start a long paragraph's preview near the touched line, not at an off-screen beginning.
  var previewText: String {
    let start = max(0, min(previewOffset, attributedText.length))
    return selectionText(attributedText, range: NSRange(location: start, length: attributedText.length - start))
  }

  override func hitTest(_ point: CGPoint, with event: UIEvent?) -> UIView? {
    let hit = super.hitTest(point, with: event)
    if hit != nil {
      selectionScope?.target = self
      if let position = closestPosition(to: point),
         let line = tokenizer.rangeEnclosingPosition(position, with: .line, inDirection: UITextDirection(rawValue: UITextStorageDirection.forward.rawValue)) {
        previewOffset = offset(from: beginningOfDocument, to: line.start)
      }
      if event?.type == .touches { selectionScope?.previewText = previewText }
    }
    return hit
  }

  override func gestureRecognizerShouldBegin(_ gestureRecognizer: UIGestureRecognizer) -> Bool {
    // A fresh long press belongs to the block menu. UIKit still owns double tap and selected-range dragging.
    if selectionScope != nil, gestureRecognizer is UILongPressGestureRecognizer, selectedRange.length == 0 { return false }
    return super.gestureRecognizerShouldBegin(gestureRecognizer)
  }

  func beginSelection() {
    guard window != nil else { return }
    becomeFirstResponder()
    selectAll(nil)
    showSelectionMenu()
  }

  private func showSelectionMenu() {
    let rect = selectedTextRange.map { firstRect(for: $0) } ?? .zero
    selectionMenu.presentEditMenu(with: UIEditMenuConfiguration(identifier: nil,
      sourcePoint: CGPoint(x: rect.midX, y: rect.minY)))
  }

  func editMenuInteraction(_ interaction: UIEditMenuInteraction, menuFor configuration: UIEditMenuConfiguration,
                           suggestedActions: [UIMenuElement]) -> UIMenu? {
    menu(for: selectedRange, suggestedActions: suggestedActions)
  }

  func menu(for range: NSRange, suggestedActions: [UIMenuElement]) -> UIMenu {
    let selected = selectionText(attributedText, range: range)
    let custom = actions.map { action in
      UIAction(title: action.title, image: UIImage(systemName: action.image)) { [weak self] _ in
        self?.resignFirstResponder()
        action.run(selected)
      }
    }
    return UIMenu(children: custom + suggestedActions)
  }

  override func resignFirstResponder() -> Bool {
    let resigned = super.resignFirstResponder()
    if resigned {
      selectionMenu.dismissMenu()
      selectedRange = NSRange(location: 0, length: 0)
      selectionScope?.active = false
    }
    return resigned
  }

  override func copy(_ sender: Any?) {
    // The renderer inserts invisible wrapping hints and code padding. Neither belongs on the clipboard.
    UIPasteboard.general.string = selectionText(attributedText, range: selectedRange)
  }
}

private let selectionDecorationKey = NSAttributedString.Key("acpira.selectionDecoration")

private func selectionDecoration(_ string: String, style: TextStyle) -> NSAttributedString {
  let text = NSMutableAttributedString(attributedString: selectionContent(string, style: style, paragraph: false))
  text.addAttribute(selectionDecorationKey, value: true, range: NSRange(location: 0, length: text.length))
  return text
}

/// The selected source text, excluding only runs added by the renderer (not invisible characters in the source).
func selectionText(_ text: NSAttributedString, range: NSRange) -> String {
  guard range.location != NSNotFound, NSMaxRange(range) <= text.length else { return "" }
  var result = ""
  text.enumerateAttribute(selectionDecorationKey, in: range) { decoration, run, _ in
    if decoration == nil { result += (text.string as NSString).substring(with: run) }
  }
  return result
}

/// TextKit uses the same faces, line boxes and CJK break opportunities as the visible SwiftUI text. Its glyphs
/// are transparent: only the native selection highlight is painted over the transcript.
func selectionContent(_ string: String, style: TextStyle, weight: FontWeight = .regular,
                      italic: Bool = false, paragraph: Bool = true) -> NSAttributedString {
  let font = (style.mono ? MonoFont.ctFont(size: style.size, weight: weight)
              : InterFont.ctFont(size: style.size, weight: weight, italic: italic)) as UIFont
  let line = NSMutableParagraphStyle()
  line.minimumLineHeight = style.lineHeight
  line.maximumLineHeight = style.lineHeight
  line.lineBreakStrategy = []
  let attributes: [NSAttributedString.Key: Any] = [
    .font: font, .foregroundColor: UIColor.clear, .paragraphStyle: line,
    .kern: style.tracking,
  ]
  var rendered = ""
  var decorations: [NSRange] = []
  var widened: [NSRange] = []
  var offset = 0
  var previous: Unicode.Scalar?
  for scalar in string.unicodeScalars {
    if let previous, CJKWidth.breakable(previous), CJKWidth.breakable(scalar) {
      rendered.append("\u{200B}")
      decorations.append(NSRange(location: offset, length: 1))
      offset += 1
    }
    let count = scalar.value > 0xFFFF ? 2 : 1
    rendered.unicodeScalars.append(scalar)
    if CJKWidth.widened(scalar) { widened.append(NSRange(location: offset, length: count)) }
    offset += count
    previous = scalar
  }
  if paragraph, CJKWidth.needsParagraphEnd(string) {
    rendered.append(CJKWidth.paragraphEnd)
    decorations.append(NSRange(location: offset, length: 1))
  }
  let result = NSMutableAttributedString(string: rendered, attributes: attributes)
  for range in widened { result.addAttribute(.kern, value: style.size * CJKWidth.extra, range: range) }
  for range in decorations { result.addAttribute(selectionDecorationKey, value: true, range: range) }
  return result
}

extension InlineBuilder {
  func selectionParagraph(_ inlines: [MDInline], weight: FontWeight) -> NSAttributedString {
    let result = selectionInlines(inlines, Marks(weight: weight))
    if CJKWidth.needsParagraphEnd(Markdown.plain(inlines)) {
      result.append(selectionDecoration(CJKWidth.paragraphEnd, style: style))
    }
    return result
  }

  private func selectionInlines(_ inlines: [MDInline], _ marks: Marks) -> NSMutableAttributedString {
    let result = NSMutableAttributedString(string: "")
    for (index, inline) in inlines.enumerated() {
      let before = index > 0 ? Markdown.plain([inlines[index - 1]]) : ""
      let after = index + 1 < inlines.count ? Markdown.plain([inlines[index + 1]]) : ""
      let current = Markdown.plain([inline])
      let code: Bool = if case .code = inline { true } else { false }
      let previousCode: Bool = index > 0 && { if case .code = inlines[index - 1] { true } else { false } }()
      if !code, !previousCode, before.unicodeScalars.last.map(CJKWidth.breakable) == true,
         current.unicodeScalars.first.map(CJKWidth.breakable) == true {
        result.append(selectionDecoration("\u{200B}", style: style))
      }
      switch inline {
      case .text(let text), .image(_, let text):
        result.append(selectionContent(text, style: style, weight: marks.weight, italic: marks.italic, paragraph: false))
      case .lineBreak:
        result.append(selectionContent("\n", style: style, paragraph: false))
      case .strong(let children):
        var next = marks; next.weight = .semibold
        result.append(selectionInlines(children, next))
      case .emphasis(let children):
        var next = marks; next.italic = true
        result.append(selectionInlines(children, next))
      case .strike(let children):
        result.append(selectionInlines(children, marks))
      case .link(let url, let children):
        let run = selectionInlines(children, marks)
        let target = SessionLinks.id(ofLink: url) == nil ? FileLinks.parse(url).flatMap(FileLinks.url) : nil
        if let link = target ?? URL(string: url) { run.addAttribute(.link, value: link, range: NSRange(location: 0, length: run.length)) }
        result.append(run)
      case .code(let text):
        let codeStyle = TextStyle(size: WebType.mono.size, lineHeight: style.lineHeight, mono: true)
        let padStyle = TextStyle(size: WebType.mono.size * 0.4 / 0.6, lineHeight: style.lineHeight, mono: true)
        let pad = selectionDecoration("\u{00A0}", style: padStyle)
        let opens = before.last?.unicodeScalars.first.map {
          [.openPunctuation, .initialPunctuation].contains($0.properties.generalCategory)
        } == true
        if let previous = before.last, previous.isWhitespace || !opens {
          result.append(selectionDecoration("\u{200B}", style: style))
        }
        result.append(pad)
        let code = NSMutableAttributedString(attributedString: selectionContent(text, style: codeStyle, paragraph: false))
        // Match InlineBuilder's slash joiners without changing the copied path.
        let source = code.string as NSString
        var search = NSRange(location: 0, length: source.length)
        var slashes: [Int] = []
        while search.length > 0 {
          let slash = source.range(of: "/", range: search)
          guard slash.location != NSNotFound else { break }
          slashes.append(NSMaxRange(slash))
          search = NSRange(location: NSMaxRange(slash), length: source.length - NSMaxRange(slash))
        }
        for offset in slashes.reversed() { code.insert(selectionDecoration("\u{2060}", style: codeStyle), at: offset) }
        if SessionLinks.id(ofLink: text) == nil, let file = FileLinks.parse(text), let url = FileLinks.url(file) {
          code.addAttribute(.link, value: url, range: NSRange(location: 0, length: code.length))
        }
        result.append(code)
        result.append(pad)
        if let next = after.first, !next.isWhitespace, !next.isPunctuation, !next.isSymbol {
          result.append(selectionDecoration("\u{200B}", style: style))
        }
      }
    }
    return result
  }
}
#endif

/// Content left out of a still render: ImageRenderer draws a native view as a yellow no-entry placeholder, which a
/// selection surface laid over its text would cover it with
struct LiveOnly<Content: View>: View {
  @ViewBuilder var content: Content
  @Environment(\.snapshot) private var snapshot

  var body: some View {
    if !snapshot { content }
  }
}

extension View {
  /// The native view is an overlay, so enabling selection never changes the existing text's size or streaming.
  @ViewBuilder func selectableText(_ text: String, style: TextStyle) -> some View {
    #if canImport(UIKit)
    accessibilityHidden(true).overlay { LiveOnly { TextSelectionSurface(text: selectionContent(text, style: style)) } }
    #else
    textSelection(.enabled)
    #endif
  }
}
