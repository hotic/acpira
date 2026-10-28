// The key bar rides on SwiftTerm's view, linked on iOS only (Package.swift)
#if os(iOS)
import AcpiraDesign
import SwiftTerm
import SwiftUI
import UIKit

// The keys a shell wants that the software keyboard lacks, on a bar over it (SwiftTerm's own accessory, restyled to
// the app's glass controls): Esc, a sticky Ctrl, Tab, the arrows (repeating while held), symbols buried on the
// keyboard's second symbol page, and a key that puts the keyboard away. The row scrolls sideways where it does not fit;
// the keyboard key stays at its end.

/// A key on the bar
enum TerminalKey: Hashable {
  case esc, ctrl, tab, left, down, up, right, hide
  case text(String)

  /// Symbols the iOS keyboard keeps a page or two away
  static let symbols = ["~", "|", "/", "\\", "-", "_", "`"]
}

/// Whether Ctrl waits for the next key; SwiftTerm lets go of it once a key used it
@Observable
final class TerminalKeysState {
  var ctrl = false
}

/// The bar as the terminal's input accessory view: a clear strip the system lays over the keyboard, holding the SwiftUI
/// keys, with the keyboard's click on each key
final class TerminalKeysBar: UIView, UIInputViewAudioFeedback {
  weak var terminal: TerminalView? {
    didSet { watchCtrl() }
  }
  private let state = TerminalKeysState()
  private var ctrlReset: (any NSObjectProtocol)?

  init() {
    super.init(frame: CGRect(x: 0, y: 0, width: 0, height: Metrics.ctl + 2 * Metrics.s1_5))
    autoresizingMask = .flexibleWidth
    backgroundColor = .clear
    let host = UIHostingController(rootView: TerminalKeys(state: state) { [weak self] in self?.press($0) })
    host.view.backgroundColor = .clear
    host.view.frame = bounds
    host.view.autoresizingMask = [.flexibleWidth, .flexibleHeight]
    addSubview(host.view)
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) { fatalError("not decoded") }

  var enableInputClicksWhenVisible: Bool { true }

  private func watchCtrl() {
    if let ctrlReset { NotificationCenter.default.removeObserver(ctrlReset) }
    ctrlReset = NotificationCenter.default.addObserver(forName: .terminalViewControlModifierReset, object: terminal,
                                                       queue: .main) { [weak self] _ in
      MainActor.assumeIsolated { self?.state.ctrl = false }
    }
  }

  private func press(_ key: TerminalKey) {
    guard let terminal else { return }
    UIDevice.current.playInputClick()
    switch key {
    case .esc: terminal.send([0x1b])
    case .tab: terminal.send([0x09])
    case .ctrl:
      terminal.controlModifier.toggle()
      state.ctrl = terminal.controlModifier
    // The view's own arrow keys follow the program's cursor mode (vim and less want the application form)
    case .left: terminal.sendKeyLeft()
    case .right: terminal.sendKeyRight()
    case .up: terminal.sendKeyUp()
    case .down: terminal.sendKeyDown()
    // Typed like a key of the keyboard, so a waiting Ctrl applies (Ctrl+\ quits, Ctrl+_ undoes)
    case .text(let text): terminal.insertText(text)
    case .hide: _ = terminal.resignFirstResponder()
    }
  }
}

/// The keys: one scrolling row of glass capsules, the keyboard key fixed at the end. Spaced like the keyboard's own keys,
/// so on a phone the row shows Esc to → with the first symbol peeking out, which tells the row scrolls
struct TerminalKeys: View {
  let state: TerminalKeysState
  var press: (TerminalKey) -> Void

  var body: some View {
    HStack(spacing: Metrics.s1) {
      ScrollView(.horizontal) {
        HStack(spacing: Metrics.s1) {
          key(.esc) { Text(verbatim: "esc") }
          key(.ctrl, on: state.ctrl) { Text(verbatim: "ctrl") }
            .accessibilityAddTraits(state.ctrl ? .isSelected : [])
          key(.tab) { Text(verbatim: "tab") }
          key(.left, repeats: true) { Image(systemName: "arrow.left") }
          key(.down, repeats: true) { Image(systemName: "arrow.down") }
          key(.up, repeats: true) { Image(systemName: "arrow.up") }
          key(.right, repeats: true) { Image(systemName: "arrow.right") }
          ForEach(TerminalKey.symbols, id: \.self) { symbol in
            key(.text(symbol)) { Text(verbatim: symbol) }
          }
        }
        .padding(.horizontal, Metrics.s2)
      }
      .scrollIndicators(.hidden)
      key(.hide) { Image(systemName: "keyboard.chevron.compact.down") }
        .accessibilityLabel(Text(web: t("mobile.terminal.hideKeyboard")))
        .padding(.trailing, Metrics.s2)
    }
    .frame(maxHeight: .infinity)
  }

  /// A key at the control height, round for a glyph and a capsule for a word; a key that is on (Ctrl waiting) fills
  /// with the ink
  private func key(_ key: TerminalKey, on: Bool = false, repeats: Bool = false,
                   @ViewBuilder label: () -> some View) -> some View {
    Button { press(key) } label: {
      label()
        .textStyle(TypeScale.text3, weight: .medium)
        .foregroundStyle(on ? Palette.bg0 : Palette.fg1)
        .padding(.horizontal, Metrics.s2)
        .frame(minWidth: Metrics.ctl, minHeight: Metrics.ctl)
        .background { if on { Capsule().fill(Palette.fg1) } }
        .contentShape(Capsule())
    }
    .buttonStyle(.plain)
    .buttonRepeatBehavior(repeats ? .enabled : .disabled)
    .glassEffect(GlassStyle.control.interactive(), in: Capsule())
  }
}
#endif
