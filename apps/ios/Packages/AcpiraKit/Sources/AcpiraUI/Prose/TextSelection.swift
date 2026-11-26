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
  @Environment(\.replyTap) private var replyTap
  @Environment(\.selectionEndHandler) private var onSelectionEnd

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
    view.replyTap = replyTap
    view.onSelectionEnd = onSelectionEnd?.run
    view.openLink = { [openURL] url in openURL(url) }
    if selection?.target == nil { selection?.target = view }
    view.actions = actions
    if let quote {
      view.actions.insert(SelectionAction(title: t("quote.add"), image: "text.quote", run: quote.run), at: 0)
    }
    if let marks {
      // The names can arrive after the text was held back (the agent advertises its commands late)
      view.loadText()
      context.coordinator.painter.names = marks
      context.coordinator.painter.attach(view)
    }
    guard !view.selectableText.isEqual(to: text) else { return }
    view.accessibilityLabel = selectionText(text, range: NSRange(location: 0, length: text.length))
    // The prompt pills are painted from the view's own layout, so a surface carrying them lays its text out at once
    if view.holdText(text, unless: marks != nil) { return }
    let selected = view.selectedRange
    view.attributedText = text
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
      (textView as? SelectionTextView)?.onSelectionEnd?()
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
      return UIAction { [weak textView] _ in (textView as? SelectionTextView)?.open(url) }
    }

    /// This view only carries selection: its glyphs are clear and SwiftUI draws the text underneath, so the
    /// system's lifted link preview snapshots an empty range and shows a dark blank platter. Keep the menu,
    /// drop the preview.
    func textView(_ textView: UITextView, menuConfigurationFor textItem: UITextItem,
                  defaultMenu: UIMenu) -> UITextItem.MenuConfiguration? {
      UITextItem.MenuConfiguration(preview: nil, menu: defaultMenu)
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
  /// The web link under the last touch on the block's text: its long-press menu leads with the link's own actions.
  /// A press between the texts (a bullet, a gap) reaches no text view and keeps the last one
  var link: URL?
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
  /// Called when an on-demand selection surface loses first responder so the hosting cell can unmount it
  var onSelectionEnd: (() -> Void)?
  /// What a plain single tap on the text does (an older reply's action row); nil leaves taps to UIKit alone
  var replyTap: ReplyTap?
  /// The touch began over a selected range: that tap only clears it
  private var tapClearsSelection = false
  private lazy var singleTap: UITapGestureRecognizer = {
    let tap = UITapGestureRecognizer(target: self, action: #selector(tapped(_:)))
    tap.cancelsTouchesInView = false
    return tap
  }()

  /// Opens a link of the text (the surface's `openURL`)
  var openLink: ((URL) -> Void)?
  /// UIKit's own link tap does not fire on this surface (iOS 27 simulator, with every recogniser and override here
  /// switched off), so a link is opened by a tap of the view's own. It takes no part in the double tap's wait: a
  /// link opens on touch up
  private lazy var linkTap: UITapGestureRecognizer = {
    let tap = UITapGestureRecognizer(target: self, action: #selector(linkTapped(_:)))
    tap.cancelsTouchesInView = false
    tap.delegate = linkTouches
    return tap
  }()
  private let linkTouches = LinkTouches()
  /// One event opens one page, should UIKit's link action and the view's tap both report it
  private var opening = false
  /// The text, held back from TextKit until a touch lands on the view or a menu selects it (`loadText`). Every paragraph
  /// of a reply carries one of these views, and laying out each one's clear copy of its text cost about as much as
  /// drawing the text when a long chat opened; a streamed paragraph laid its copy out again on every chunk
  private var heldText: NSAttributedString?
  private var loaded = false

  /// The text the view selects, whether laid out yet or not
  var selectableText: NSAttributedString { heldText ?? attributedText }

  /// Keeps `text` back while nothing has needed the view's layout; false once the text is laid out (or `now`), when
  /// the caller sets it on the view itself
  func holdText(_ text: NSAttributedString, unless now: Bool) -> Bool {
    if now { loadText() }
    guard !loaded else { return false }
    heldText = text
    return true
  }

  /// Lays the held text out: from here on the view carries it like any text view
  func loadText() {
    guard !loaded else { return }
    loaded = true
    if let heldText { attributedText = heldText }
    heldText = nil
  }

  override func didMoveToWindow() {
    super.didMoveToWindow()
    guard window != nil else { return }
    if singleTap.view == nil { addGestureRecognizer(singleTap) }
    if linkTap.view == nil { addGestureRecognizer(linkTap) }
    // Wait out UIKit's double tap (word selection), so selecting a word never also flips the row
    for case let tap as UITapGestureRecognizer in gestureRecognizers ?? [] where tap !== singleTap && tap.numberOfTapsRequired >= 2 {
      singleTap.require(toFail: tap)
    }
  }

  @objc private func linkTapped(_ tap: UITapGestureRecognizer) {
    guard !tapClearsSelection, let url = link(at: tap.location(in: self)) else { return }
    open(url)
  }

  func open(_ url: URL) {
    guard !opening else { return }
    opening = true
    DispatchQueue.main.async { [weak self] in self?.opening = false }
    // The tap used the link up: a later press beside the text must not offer it again
    selectionScope?.link = nil
    openLink?(url)
  }

  @objc private func tapped(_ tap: UITapGestureRecognizer) {
    guard let replyTap, !tapClearsSelection else { return }
    // A link keeps its own tap
    if link(at: tap.location(in: self)) != nil { return }
    replyTap()
  }

  /// The link whose glyph lies under the point. The closest position is a caret between two characters, so the
  /// one on each side is asked for its own rect: the nearest caret alone names the neighbour for half of each glyph
  func link(at point: CGPoint) -> URL? {
    guard let position = closestPosition(to: point) else { return nil }
    let caret = offset(from: beginningOfDocument, to: position)
    for at in [caret, caret - 1] where at >= 0 && at < attributedText.length {
      guard let url = attributedText.attribute(.link, at: at, effectiveRange: nil) as? URL,
            let start = self.position(from: beginningOfDocument, offset: at),
            let end = self.position(from: start, offset: 1),
            let glyph = textRange(from: start, to: end),
            firstRect(for: glyph).contains(point) else { continue }
      return url
    }
    return nil
  }

  /// Start a long paragraph's preview near the touched line, not at an off-screen beginning.
  var previewText: String {
    let start = max(0, min(previewOffset, attributedText.length))
    return selectionText(attributedText, range: NSRange(location: start, length: attributedText.length - start))
  }

  override func hitTest(_ point: CGPoint, with event: UIEvent?) -> UIView? {
    let hit = super.hitTest(point, with: event)
    if hit != nil {
      // Hit-testing asks every sibling; only the view under the point lays its text out, before anything here reads it
      loadText()
      selectionScope?.target = self
      if let position = closestPosition(to: point),
         let line = tokenizer.rangeEnclosingPosition(position, with: .line, inDirection: UITextDirection(rawValue: UITextStorageDirection.forward.rawValue)) {
        previewOffset = offset(from: beginningOfDocument, to: line.start)
      }
      if event?.type == .touches {
        selectionScope?.previewText = previewText
        tapClearsSelection = selectedRange.length > 0
        // File and session links are the app's own pages: only a web address has somewhere else to go
        let web = link(at: point).flatMap { $0.isWebPage ? $0 : nil }
        if selectionScope?.link != web { selectionScope?.link = web }
      }
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
    // The block menu's Select may reach a paragraph no touch has laid out
    loadText()
    becomeFirstResponder()
    selectAll(nil)
    showSelectionMenu()
  }

  /// Enters native range selection at `point` (for an on-demand double tap), selecting the word under the finger
  func beginWordSelection(at point: CGPoint) {
    guard window != nil else { return }
    loadText()
    layoutIfNeeded()
    becomeFirstResponder()
    let forward = UITextDirection(rawValue: UITextStorageDirection.forward.rawValue)
    let backward = UITextDirection(rawValue: UITextStorageDirection.backward.rawValue)
    if let pos = closestPosition(to: point),
       let range = tokenizer.rangeEnclosingPosition(pos, with: .word, inDirection: forward)
         ?? tokenizer.rangeEnclosingPosition(pos, with: .word, inDirection: backward)
         ?? tokenizer.rangeEnclosingPosition(pos, with: .character, inDirection: forward) {
      let loc = offset(from: beginningOfDocument, to: range.start)
      let len = offset(from: range.start, to: range.end)
      if len > 0 {
        selectedRange = NSRange(location: loc, length: len)
      } else {
        selectAll(nil)
      }
    } else {
      selectAll(nil)
    }
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
      onSelectionEnd?()
    }
    return resigned
  }

  override func copy(_ sender: Any?) {
    // The renderer inserts invisible wrapping hints and code padding. Neither belongs on the clipboard.
    UIPasteboard.general.string = selectionText(attributedText, range: selectedRange)
  }
}

/// Keeps the link tap to touches that land on a link, where it recognises beside UIKit's own taps on the text
private final class LinkTouches: NSObject, UIGestureRecognizerDelegate {
  func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer, shouldReceive touch: UITouch) -> Bool {
    guard let view = gestureRecognizer.view as? SelectionTextView else { return false }
    return view.link(at: touch.location(in: view)) != nil
  }

  func gestureRecognizer(_ gestureRecognizer: UIGestureRecognizer,
                         shouldRecognizeSimultaneouslyWith other: UIGestureRecognizer) -> Bool { true }
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
        let pad = selectionDecoration(InlineBuilder.codePad, style: padStyle)
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
/// On-demand offscreen text probe used only at the instant of a long press (or link hit) when no persistent
/// `SelectionTextView` is mounted on the paragraph. Computes the exact web link and touched-line excerpt.
@MainActor enum TextSelectionProbe {
  struct Inspection {
    var link: URL?
    var previewText: String
  }

  /// Whether `block` contains any `http`/`https` web link worth probing on long press
  static func hasWebLink(_ block: MDBlock) -> Bool {
    switch block {
    case .paragraph(let inlines), .heading(_, let inlines):
      inlinesHaveWebLink(inlines)
    case .list(_, _, let items):
      items.contains { $0.contains(where: hasWebLink) }
    case .quote(let inner):
      inner.contains(where: hasWebLink)
    case .table(_, let header, let rows):
      header.contains(where: inlinesHaveWebLink) || rows.contains { $0.contains(where: inlinesHaveWebLink) }
    case .code, .math, .rule:
      false
    }
  }

  private static func inlinesHaveWebLink(_ inlines: [MDInline]) -> Bool {
    inlines.contains { inline in
      switch inline {
      case .link(let url, _):
        URL(string: url)?.isWebPage == true
      case .strong(let c), .emphasis(let c), .strike(let c):
        inlinesHaveWebLink(c)
      default:
        false
      }
    }
  }

  /// Whether `block` contains any link (web, file path in backticks, or session link)
  static func hasAnyLink(_ block: MDBlock) -> Bool {
    switch block {
    case .paragraph(let inlines), .heading(_, let inlines):
      inlinesHaveAnyLink(inlines)
    case .list(_, _, let items):
      items.contains { $0.contains(where: hasAnyLink) }
    case .quote(let inner):
      inner.contains(where: hasAnyLink)
    case .table(_, let header, let rows):
      header.contains(where: inlinesHaveAnyLink) || rows.contains { $0.contains(where: inlinesHaveAnyLink) }
    case .code, .math, .rule:
      false
    }
  }

  private static func inlinesHaveAnyLink(_ inlines: [MDInline]) -> Bool {
    inlines.contains { inline in
      switch inline {
      case .link: true
      case .code(let text): SessionLinks.id(ofLink: text) == nil && FileLinks.parse(text) != nil
      case .strong(let c), .emphasis(let c), .strike(let c): inlinesHaveAnyLink(c)
      default: false
      }
    }
  }

  /// Resolves any link (web, file, or session) at `pointInBlock`, used only when a tap lands on an older reply block
  /// that contains links so tapping a link never toggles the turn's action row.
  static func anyLink(in block: MDBlock, width: CGFloat, pointInBlock: CGPoint) -> URL? {
    guard width > 1, hasAnyLink(block) else { return nil }
    return probe(block: block, width: width, point: pointInBlock,
                 style: ProseStyle(tone: .reply, accent: Palette.brand), includePreview: false).link
  }

  /// Inspects a single block at `pointInBlock` for a web link and line-local preview excerpt.
  /// Only called on user long press, never during scrolling or streaming.
  static func inspect(block: MDBlock, quoteBlock: MDBlock, width: CGFloat, pointInBlock: CGPoint) -> Inspection {
    let fallback = QuoteText.of(quoteBlock)
    guard width > 1 else { return Inspection(link: nil, previewText: fallback) }
    // Only lay out offscreen when the block has a web link or is long enough to benefit from line-local preview
    guard hasWebLink(block) || fallback.utf8.count > 100 else {
      return Inspection(link: nil, previewText: fallback)
    }
    let hit = probe(block: block, width: width, point: pointInBlock,
                    style: ProseStyle(tone: .reply, accent: Palette.brand), includePreview: true)
    let web = hit.link.flatMap { $0.isWebPage ? $0 : nil }
    // For container blocks (lists/quotes), keep the whole quote text as preview unless a single long paragraph was pressed
    let preview: String
    switch block {
    case .paragraph, .heading, .code:
      preview = hit.preview ?? fallback
    default:
      preview = fallback
    }
    return Inspection(link: web, previewText: preview)
  }

  private struct ProbeHit {
    var height: CGFloat
    var link: URL?
    var preview: String?
  }

  /// Walks leaf and container blocks (`list`, `quote`) using the same vertical gaps and indents as `ProseList` /
  /// `ProseQuote` to resolve link hits and line-local previews without keeping any persistent `UITextView` mounted.
  private static func probe(block: MDBlock, width: CGFloat, point: CGPoint, style: ProseStyle,
                            includePreview: Bool) -> ProbeHit {
    switch block {
    case .paragraph(let inlines):
      let attr = InlineBuilder(style: style.body, ink: style.ink, strongInk: style.strongInk, accent: style.accent)
        .selectionParagraph(inlines, weight: .regular)
      return measureLeaf(attr, width: width, point: point, includePreview: includePreview)
    case .heading(let level, let inlines):
      let headingStyle = style.heading(level)
      let attr = InlineBuilder(style: headingStyle, ink: style.strongInk, strongInk: style.strongInk, accent: style.accent)
        .selectionParagraph(inlines, weight: .semibold)
      return measureLeaf(attr, width: width, point: point, includePreview: includePreview)
    case .code(_, let text):
      let attr = selectionContent(text, style: WebType.mono)
      return measureLeaf(attr, width: width, point: point, includePreview: includePreview)
    case .list(_, _, let items):
      let indent = style.listIndent
      let innerWidth = max(1, width - indent)
      var y: CGFloat = 0
      var foundLink: URL?
      for (itemIndex, item) in items.enumerated() {
        if itemIndex > 0 { y += Metrics.s0_5 }
        y += Metrics.s1
        for (blockIndex, inner) in item.enumerated() {
          if blockIndex > 0 { y += Metrics.s0_5 }
          let local = CGPoint(x: point.x - indent, y: point.y - y)
          let child = probe(block: inner, width: innerWidth, point: local, style: style, includePreview: false)
          if foundLink == nil, point.y >= y - Metrics.s1, point.y <= y + child.height + Metrics.s1, let url = child.link {
            foundLink = url
          }
          y += child.height
        }
        y += Metrics.s1
      }
      return ProbeHit(height: y, link: foundLink, preview: nil)
    case .quote(let inner):
      let innerWidth = max(1, width - Metrics.pad * 2)
      var quiet = style
      quiet.inkOverride = Palette.fg2
      var y = Metrics.gap
      var foundLink: URL?
      for (i, childBlock) in inner.enumerated() {
        if i > 0 { y += Metrics.gap }
        let local = CGPoint(x: point.x - Metrics.pad, y: point.y - y)
        let child = probe(block: childBlock, width: innerWidth, point: local, style: quiet, includePreview: false)
        if foundLink == nil, point.y >= y - Metrics.gap / 2, point.y <= y + child.height + Metrics.gap / 2,
           let url = child.link {
          foundLink = url
        }
        y += child.height
      }
      y += Metrics.gap
      return ProbeHit(height: y, link: foundLink, preview: nil)
    default:
      return ProbeHit(height: 0, link: nil, preview: nil)
    }
  }

  private static func measureLeaf(_ attributed: NSAttributedString, width: CGFloat, point: CGPoint,
                                  includePreview: Bool) -> ProbeHit {
    let storage = NSTextStorage(attributedString: attributed)
    let layout = NSLayoutManager()
    let container = NSTextContainer(size: CGSize(width: width, height: .greatestFiniteMagnitude))
    container.lineFragmentPadding = 0
    layout.addTextContainer(container)
    storage.addLayoutManager(layout)
    layout.ensureLayout(for: container)
    let height = layout.usedRect(for: container).height
    let clamped = CGPoint(x: max(0, min(point.x, width - 1)), y: max(0, min(point.y, max(0, height - 1))))
    var link: URL?
    if point.y >= -Metrics.s1, point.y <= height + Metrics.s1, attributed.length > 0 {
      var fraction: CGFloat = 0
      let glyph = layout.glyphIndex(for: clamped, in: container, fractionOfDistanceThroughGlyph: &fraction)
      if glyph < layout.numberOfGlyphs {
        let charIdx = layout.characterIndexForGlyph(at: glyph)
        let glyphRect = layout.boundingRect(forGlyphRange: NSRange(location: glyph, length: 1), in: container)
        if glyphRect.insetBy(dx: -2, dy: -2).contains(clamped),
           charIdx < attributed.length,
           let url = attributed.attribute(.link, at: charIdx, effectiveRange: nil) as? URL {
          link = url
        }
      }
    }
    var preview: String?
    if includePreview, attributed.length > 0 {
      let glyph = layout.glyphIndex(for: clamped, in: container)
      if glyph < layout.numberOfGlyphs {
        var lineRange = NSRange()
        _ = layout.lineFragmentRect(forGlyphAt: glyph, effectiveRange: &lineRange)
        let start = layout.characterIndexForGlyph(at: lineRange.location)
        let excerpt = selectionText(attributed, range: NSRange(location: start, length: attributed.length - start))
        if !excerpt.isEmpty { preview = excerpt }
      }
    }
    return ProbeHit(height: height, link: link, preview: preview)
  }
}
#endif

/// Equatable wrapper for the on-demand selection dismissal callback so `EnvironmentValues` comparisons stay cheap.
struct SelectionEndHandler: Equatable {
  let id: String
  let run: () -> Void
  static func == (a: Self, b: Self) -> Bool { a.id == b.id }
}

extension EnvironmentValues {
  /// When true (hosted `TranscriptCollection` cells), paragraphs and code surfaces defer creating native
  /// `UITextView` selection overlays until `selectionSurfaceActive` turns true on demand.
  @Entry var deferSelectionSurface = false
  /// True only for the single collection cell currently hosting native range selection.
  @Entry var selectionSurfaceActive = false
  /// Callback invoked when an on-demand selection surface ends editing so its host cell can return to idle.
  @Entry var selectionEndHandler: SelectionEndHandler? = nil
}

/// Content left out of a still render: ImageRenderer draws a native view as a yellow no-entry placeholder, which a
/// selection surface laid over its text would cover it with
struct LiveOnly<Content: View>: View {
  @ViewBuilder var content: Content
  @Environment(\.snapshot) private var snapshot

  var body: some View {
    if !snapshot { content }
  }
}

#if canImport(UIKit)
private struct DeferredSelectableText: ViewModifier {
  let text: String
  let style: TextStyle
  @Environment(\.deferSelectionSurface) private var deferred
  @Environment(\.selectionSurfaceActive) private var active

  func body(content: Content) -> some View {
    if !deferred || active {
      content
        .accessibilityHidden(true)
        .overlay { LiveOnly { TextSelectionSurface(text: selectionContent(text, style: style)) } }
    } else {
      content
    }
  }
}
#endif

extension View {
  /// The native view is an overlay, so enabling selection never changes the existing text's size or streaming.
  @ViewBuilder func selectableText(_ text: String, style: TextStyle) -> some View {
    #if canImport(UIKit)
    modifier(DeferredSelectableText(text: text, style: style))
    #else
    textSelection(.enabled)
    #endif
  }
}
