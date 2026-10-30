// SwiftTerm is linked on iOS only (Package.swift); the Mac build keeps the terminal's logic and draws no terminal
#if os(iOS)
import AcpiraDesign
import AcpiraModels
import SwiftTerm
import SwiftUI
import UIKit

// SwiftTerm's view (the emulator xterm.js is to the desktop) wired to a TerminalSession: output in, keys and the laid
// out grid out. It opens the engine's terminal at its first layout, so the shell starts at the size the page shows, and
// takes the keyboard then, as the desktop focuses a terminal tab it shows.

/// The terminal view as a SwiftUI view; `dark` picks the look (the app's appearance, which may differ from the device's)
struct TerminalSurface: UIViewRepresentable {
  let session: TerminalSession
  var dark: Bool

  func makeUIView(context: Context) -> PhoneTerminalView {
    let view = PhoneTerminalView(session: session)
    view.terminalDelegate = context.coordinator
    view.apply(TerminalLook.of(dark: dark), dark: dark)
    return view
  }

  func updateUIView(_ view: PhoneTerminalView, context: Context) {
    if view.dark != dark { view.apply(TerminalLook.of(dark: dark), dark: dark) }
  }

  /// The page went: the engine stops sending to it, and the view's renderer stops (SwiftTerm's own SwiftUI wrapper
  /// tears it down the same way, retrying while the GPU still holds it)
  static func dismantleUIView(_ view: PhoneTerminalView, coordinator: Coordinator) {
    view.session.close(from: view)
    shutDown(view)
  }

  private static func shutDown(_ view: PhoneTerminalView) {
    guard !view.updateUiClosed() else { return }
    Task { @MainActor in
      try? await Task.sleep(for: .milliseconds(100))
      shutDown(view)
    }
  }

  func makeCoordinator() -> Coordinator { Coordinator() }

  /// What the view reports: keys and pastes to the shell, a new grid to the pty. The title is the engine's (the
  /// foreground program), not the one a program sets, as on the desktop
  final class Coordinator: NSObject, TerminalViewDelegate {
    func send(source: TerminalView, data: ArraySlice<UInt8>) {
      (source as? PhoneTerminalView)?.session.input(Array(data))
    }

    func sizeChanged(source: TerminalView, newCols: Int, newRows: Int) {
      (source as? PhoneTerminalView)?.session.resize(cols: newCols, rows: newRows)
    }

    /// A web address opens in the system browser. One on the loopback names the engine's computer, not the phone, so
    /// it goes nowhere
    func requestOpenLink(source: TerminalView, link: String, params: [String: String]) {
      guard let url = URL(string: link), ["http", "https"].contains(url.scheme?.lowercased() ?? ""),
            let host = url.host(), !loopback(host) else { return }
      UIApplication.shared.open(url)
    }

    /// OSC 52: a program may put text on the clipboard (vim, tmux over ssh), never read it back (xterm.ts)
    func clipboardCopy(source: TerminalView, content: Data) {
      UIPasteboard.general.string = String(decoding: content, as: UTF8.self)
    }

    func setTerminalTitle(source: TerminalView, title: String) {}
    func hostCurrentDirectoryUpdate(source: TerminalView, directory: String?) {}
    func scrolled(source: TerminalView, position: Double) {}
    func rangeChanged(source: TerminalView, startY: Int, endY: Int) {}

    private func loopback(_ host: String) -> Bool {
      host == "localhost" || host == "::1" || host == "0.0.0.0" || host.hasPrefix("127.")
    }
  }
}

/// SwiftTerm's view as the session's screen, in the desktop terminal's look, with the phone's key bar over the keyboard
final class PhoneTerminalView: TerminalView, TerminalScreen {
  let session: TerminalSession
  private(set) var dark = false
  /// The engine's terminal was opened from this view (at its first layout)
  private var opened = false
  private let keys = TerminalKeysBar()

  init(session: TerminalSession) {
    self.session = session
    let font = MonoFont.ctFont(size: TerminalLook.fontSize, weight: .regular) as UIFont
    super.init(frame: .zero, font: font, options: TerminalOptions(scrollback: TerminalLook.scrollback))
    let bold = MonoFont.ctFont(size: TerminalLook.fontSize, weight: .bold) as UIFont
    setFonts(normal: font, bold: bold, italic: italic(font), boldItalic: italic(bold))
    lineSpacing = TerminalLook.lineHeight
    minimumContrastRatio = TerminalLook.minimumContrast
    keys.terminal = self
    inputAccessoryView = keys
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) { fatalError("not decoded") }

  private func italic(_ font: UIFont) -> UIFont {
    font.fontDescriptor.withSymbolicTraits(font.fontDescriptor.symbolicTraits.union(.traitItalic))
      .map { UIFont(descriptor: $0, size: font.pointSize) } ?? font
  }

  /// The look for one appearance: the page's ground and inks, the sixteen colours (SwiftTerm derives the rest of the
  /// 256 from them and the ground), and a keyboard of the same appearance
  func apply(_ look: TerminalLook, dark: Bool) {
    self.dark = dark
    nativeForegroundColor = look.foreground.uiColor
    // The ground is the layer's colour, which this sets. Never set the view's `backgroundColor` as well: the layer
    // then lost its colour and the ground drew black
    nativeBackgroundColor = look.background.uiColor
    caretColor = look.cursor.uiColor
    caretTextColor = look.cursorText.uiColor
    selectedTextBackgroundColor = look.selection.uiColor
    installColors(look.ansi.map(\.terminalColor))
    keyboardAppearance = dark ? .dark : .light
    // The accessory lives in the keyboard's window, outside the page's background.
    keys.backgroundColor = look.background.uiColor
    keys.overrideUserInterfaceStyle = dark ? .dark : .light
    if isFirstResponder { reloadInputViews() }
  }

  override func layoutSubviews() {
    super.layoutSubviews()
    // The first layout in a window: the grid the shell should start at
    guard !opened, window != nil, bounds.width > 0, bounds.height > 0 else { return }
    opened = true
    let grid = terminalDimensions
    session.open(on: self, cols: grid.cols, rows: grid.rows)
    _ = becomeFirstResponder()
  }

  func write(_ text: String) { feed(text: text) }

  func replay(_ text: String) {
    // RIS clears the screen and the scrollback (xterm's reset); the theme's colours stay installed
    resetToInitialState()
    if !text.isEmpty { feed(text: text) }
  }

  func note(_ text: String) { feed(text: "\r\n\u{1b}[2m\(text)\u{1b}[22m\r\n") }

  func focus(_ on: Bool) {
    _ = on ? becomeFirstResponder() : resignFirstResponder()
  }
}

extension RGBA {
  var uiColor: UIColor { UIColor(red: r, green: g, blue: b, alpha: a) }
  /// SwiftTerm's 16-bit channels
  var terminalColor: SwiftTerm.Color {
    SwiftTerm.Color(red: UInt16((r * 65535).rounded()), green: UInt16((g * 65535).rounded()), blue: UInt16((b * 65535).rounded()))
  }
}
#endif
