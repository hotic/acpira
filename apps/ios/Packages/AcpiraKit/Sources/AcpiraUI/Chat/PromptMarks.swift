import AcpiraDesign
import SwiftUI
#if canImport(UIKit)
import CoreText
import UIKit
#endif

// The command and summon pills of a prompt being written (PromptInput.tsx): every `/command` the agent advertises and
// every `@name` of a summonable persona gets a tinted rounded pill behind it and its colour, the command blue, the
// summon violet. The webview paints a mirror under a transparent textarea; here a layer inside the native text view
// draws the pills from the view's own line layout, and TextKit 2 rendering attributes tint the glyphs, so the text
// (its storage, typing attributes, IME marked text, undo) is never touched.

extension View {
  /// Paints the marks of the native text view this modifier sits on (a `TextField` or `TextEditor`). `text` is the
  /// field's binding: a change SwiftUI makes (a pick, a restored draft) repaints, as a keystroke does
  func promptMarks(text: String, commands: [String], summons: [String]) -> some View {
    #if canImport(UIKit)
    background(PromptMarkProbe(text: text, commands: commands, summons: summons)
      .allowsHitTesting(false)
      .accessibilityHidden(true))
    #else
    self
    #endif
  }
}

/// What a painter marks: the advertised commands and the summonable personas
struct PromptMarkNames: Equatable {
  var commands: [String] = []
  var summons: [String] = []
}

extension EnvironmentValues {
  /// The names of the commands the session's agent advertises, for the pills of sent prompts
  @Entry var promptCommands: [String] = []
}

/// A sent prompt as `Text(web:)` sets it, its marks in their colours (Turns.tsx UserMessage). The pills themselves
/// are drawn by the selection surface over it, which lays the same text out natively
func markedPrompt(_ text: String, names: PromptMarkNames) -> AttributedString {
  var a = CJKWidth.attributed(text, size: TypeScale.text1.size)
  let found = SlashCommands.renderedMarks(String(a.characters), commands: names.commands, summons: names.summons)
  for (mark, span) in zip(found.marks, found.spans) {
    let start = a.characters.index(a.startIndex, offsetBy: span.lowerBound)
    let end = a.characters.index(start, offsetBy: span.count)
    a[start..<end].foregroundColor = mark.summon ? Palette.summon : Palette.command
  }
  return a
}

#if canImport(UIKit)
/// Sits in the field's background, a sibling of the native view under SwiftUI's host, and hands that view to its painter
private struct PromptMarkProbe: UIViewRepresentable {
  var text: String
  var commands: [String]
  var summons: [String]

  func makeUIView(context: Context) -> Probe { Probe() }
  func updateUIView(_ view: Probe, context: Context) {
    view.painter.names = PromptMarkNames(commands: commands, summons: summons)
    view.install()
  }
  static func dismantleUIView(_ view: Probe, coordinator: ()) { view.painter.detach() }

  final class Probe: UIView {
    let painter = PromptMarkPainter()

    override func didMoveToWindow() {
      super.didMoveToWindow()
      if window == nil { painter.detach() } else { install() }
    }

    func install() {
      // The native view may not be in the hierarchy yet on the first update
      Task { @MainActor [weak self] in
        guard let self, window != nil else { return }
        painter.attach(nearestTextInput(from: self) as? UITextView)
      }
    }
  }
}

/// Keeps one text view's pills and glyph tint in step with its text, width and appearance
final class PromptMarkPainter {
  var names = PromptMarkNames() {
    didSet { if names != oldValue { schedule() } }
  }
  private weak var view: UITextView?
  private let pills = PillLayer()
  private var observers: [NSObjectProtocol] = []
  private var resize: NSKeyValueObservation?
  /// Rendering attributes are on the view's layout manager
  private var tinted = false
  private var pending = false
  /// Whether the painter colours the glyphs too. A view whose glyphs SwiftUI draws (the sent prompt, under its
  /// selection surface) has them coloured there, and its pills are drawn over them: a fill of the glyphs' own colour
  /// at 15% leaves those glyphs as they were
  private let tints: Bool

  init(tints: Bool = true) {
    self.tints = tints
    pills.onTraits = { [weak self] in self?.schedule() }
  }

  func attach(_ textView: UITextView?) {
    guard let textView else { return }
    guard textView !== view else { schedule(); return }
    detach()
    view = textView
    // Below the text's own canvas, inside the scroll view, so the pills scroll with the lines they sit under
    textView.insertSubview(pills, at: 0)
    observers = [NotificationCenter.default.addObserver(forName: UITextView.textDidChangeNotification, object: textView,
                                                        queue: .main) { [weak self] _ in
      MainActor.assumeIsolated { self?.schedule() }
    }]
    // A new width rewraps the lines; scrolling moves bounds too, which changes nothing here
    resize = textView.observe(\.bounds, options: [.old, .new]) { [weak self] _, change in
      guard change.oldValue?.size.width != change.newValue?.size.width else { return }
      MainActor.assumeIsolated { self?.schedule() }
    }
    schedule()
  }

  func detach() {
    if let view { untint(view) }
    pills.removeFromSuperview()
    observers.forEach(NotificationCenter.default.removeObserver)
    observers = []
    resize = nil
    view = nil
  }

  /// One paint per run loop turn, after the text view has laid out the change that asked for it
  private func schedule() {
    guard !pending else { return }
    pending = true
    Task { @MainActor [weak self] in
      guard let self else { return }
      pending = false
      paint()
    }
  }

  private func paint() {
    guard let view else { return }
    let text = view.text ?? ""
    let found = SlashCommands.renderedMarks(text, commands: names.commands, summons: names.summons)
    var shapes: [(rect: CGRect, summon: Bool)] = []
    var ranges: [(range: NSRange, summon: Bool)] = []
    // Character offsets to the UTF-16 ones UIKit counts in
    var index = text.startIndex, at = 0
    for (i, mark) in found.marks.enumerated() {
      let span = found.spans[i]
      let start = text.index(index, offsetBy: span.lowerBound - at)
      let end = text.index(start, offsetBy: span.count)
      index = start
      at = span.lowerBound
      let range = NSRange(start..<end, in: text)
      ranges.append((range, mark.summon))
      // A position right before the closing word joiner (CJKWidth.paragraphEnd) of a line filled to its edge comes
      // back at the line's start, which leaves the run zero wide: the geometry takes the zero-width hints after it in
      let units = text.utf16
      var length = range.length
      while let next = units.index(units.startIndex, offsetBy: range.location + length, limitedBy: units.endIndex),
            next < units.endIndex, units[next] == 0x2060 || units[next] == 0x200B { length += 1 }
      guard let from = view.position(from: view.beginningOfDocument, offset: range.location),
            let to = view.position(from: from, offset: length),
            let run = view.textRange(from: from, to: to) else { continue }
      let font = view.textStorage.attribute(.font, at: range.location, effectiveRange: nil) as? UIFont
        ?? view.font ?? UIFont.systemFont(ofSize: TypeScale.field.size)
      let reach = SlashCommands.markReach(found.plain, marks: found.marks, at: i, overhang: Self.overhang,
                                          space: font.pointSize * 0.25, clear: Self.clear)
      // One rect per line the token runs over (a long name can wrap), each its own pill
      for line in view.selectionRects(for: run) where line.rect.width > 0 && !line.rect.isNull {
        shapes.append((pill(line.rect, baseline: baseline(view, near: line.rect.midY), reach: reach, font: font),
                       mark.summon))
      }
    }
    pills.show(shapes, traits: view.traitCollection)
    guard tints else { return }
    let stale = tinted || !ranges.isEmpty
    untint(view)
    tint(view, ranges)
    // UITextView draws a laid-out fragment once: a rendering attribute shows only after its layout is redone
    if stale, let manager = view.textLayoutManager {
      manager.invalidateLayout(for: manager.documentRange)
      manager.textViewportLayoutController.layoutViewport()
    }
  }

  /// The pill around a line's run of the token: its face's ascent and descent around the line's baseline plus
  /// `--mark-pad` above and below (py-0.5, CSS's inline content area), the reach on either side. The line box's middle
  /// is not the glyphs' middle: an exact line height leaves its spare room unevenly around them, so without a baseline
  /// the pill sat low
  private func pill(_ run: CGRect, baseline: CGFloat?, reach: Double, font: UIFont) -> CGRect {
    let height = min(run.height, font.ascender - font.descender + Self.pad * 2)
    let top = baseline.map { $0 - font.ascender - Self.pad } ?? run.midY - height / 2
    return CGRect(x: run.minX - reach, y: top, width: run.width + reach * 2, height: height)
  }

  /// The baseline, in the text view's coordinates, of the glyphs on the line running through `y`. A tinting painter
  /// sits on text TextKit draws, so it takes TextKit's baseline, which leaves an exact line's spare room above the
  /// glyphs. Otherwise the view is a transparent copy over SwiftUI `Text`, which leaves that room below them: the
  /// baseline sits the line's tallest face's ascent (PingFang on a CJK line) below the line's top
  private func baseline(_ view: UITextView, near y: CGFloat) -> CGFloat? {
    guard let manager = view.textLayoutManager else { return nil }
    let inset = view.textContainerInset
    let point = CGPoint(x: inset.left + 1, y: y - inset.top)
    guard let fragment = manager.textLayoutFragment(for: point) else { return nil }
    let frame = fragment.layoutFragmentFrame
    for line in fragment.textLineFragments {
      let bounds = line.typographicBounds.offsetBy(dx: frame.minX, dy: frame.minY)
      guard bounds.minY <= point.y, point.y < bounds.maxY else { continue }
      if tints { return inset.top + bounds.minY + line.glyphOrigin.y }
      let run = line.attributedString.attributedSubstring(from: line.characterRange)
      var ascent: CGFloat = 0
      CTLineGetTypographicBounds(CTLineCreateWithAttributedString(run), &ascent, nil, nil)
      return inset.top + bounds.minY + ascent
    }
    return nil
  }

  private func tint(_ view: UITextView, _ ranges: [(range: NSRange, summon: Bool)]) {
    guard !ranges.isEmpty, let manager = view.textLayoutManager, let content = manager.textContentManager else { return }
    for (range, summon) in ranges {
      guard let start = content.location(content.documentRange.location, offsetBy: range.location),
            let end = content.location(start, offsetBy: range.length),
            let span = NSTextRange(location: start, end: end) else { continue }
      manager.addRenderingAttribute(.foregroundColor, value: PillLayer.ink(summon, view.traitCollection), for: span)
    }
    tinted = true
  }

  private func untint(_ view: UITextView) {
    guard tinted, let manager = view.textLayoutManager else { return }
    manager.removeRenderingAttribute(.foregroundColor, for: manager.documentRange)
    tinted = false
  }

  /// `--mark-overhang`, `--mark-clear` and the pill's `py-0.5` (tokens.css)
  private static let overhang = 4.0
  private static let clear = 1.5
  private static let pad = Metrics.s0_5
}

/// The pills, one rounded layer each, in the text view's content coordinates
private final class PillLayer: UIView {
  var onTraits: () -> Void = {}

  override init(frame: CGRect) {
    super.init(frame: frame)
    isUserInteractionEnabled = false
    isAccessibilityElement = false
    registerForTraitChanges([UITraitUserInterfaceStyle.self]) { (view: PillLayer, _) in view.onTraits() }
  }
  required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

  /// The mark's colour, `--command` or `--summon`
  static func ink(_ summon: Bool, _ traits: UITraitCollection) -> UIColor {
    UIColor(summon ? Palette.summon : Palette.command).resolvedColor(with: traits)
  }

  func show(_ shapes: [(rect: CGRect, summon: Bool)], traits: UITraitCollection) {
    let layers = layer.sublayers ?? []
    CATransaction.begin()
    // Pills follow the text as it is typed, never sliding in from where they were
    CATransaction.setDisableActions(true)
    for (i, shape) in shapes.enumerated() {
      let pill = i < layers.count ? layers[i] : { let l = CALayer(); layer.addSublayer(l); return l }()
      pill.frame = shape.rect
      pill.cornerRadius = Metrics.rSm
      pill.cornerCurve = .continuous
      // bg-command/15, bg-summon/15
      pill.backgroundColor = Self.ink(shape.summon, traits).withAlphaComponent(0.15).cgColor
    }
    layers.dropFirst(shapes.count).forEach { $0.removeFromSuperlayer() }
    CATransaction.commit()
  }
}
#endif
