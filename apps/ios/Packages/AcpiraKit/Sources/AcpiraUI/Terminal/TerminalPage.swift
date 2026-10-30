import AcpiraDesign
import AcpiraModels
import SwiftUI

// The terminal page the chat's "…" menu pushes: the project's shell on the engine's computer, the desktop panel's
// terminal tab (panel/TerminalFrame.tsx) at phone size. The system bar names it after the program in the foreground
// with the project under it; its menu restarts or closes the shell. Copy and paste are the system's edit menu on a long
// press, as in any iOS text view; the keys the software keyboard lacks sit on a bar above it (TerminalKeys.swift).

/// The project's terminal under the system navigation bar
struct TerminalPage: View {
  let cwd: String
  @Environment(AppModel.self) private var model
  @Environment(\.colorScheme) private var scheme

  var body: some View {
    let session = model.terminal(cwd)
    surface(session)
      // The text set in from the edges like the desktop's (pt-gap pb-gap-half pl-pad); the right edge keeps what is
      // left of the last cell
      .padding(.top, Metrics.gap)
      .padding(.bottom, Metrics.gapHalf)
      .padding(.leading, Metrics.pad)
      // Paint behind the keyboard's rounded corners; only the ground ignores its safe area, not the terminal grid.
      .background(Palette.bg0.ignoresSafeArea())
      .navigationTitle(session.title ?? t("panel.terminal"))
      .navigationSubtitle(folderName(cwd))
      .navigationBarTitleDisplayModeInline()
      .backWithoutTitle()
      .toolbar {
        ToolbarItem(placement: .primaryAction) {
          Menu {
            Button { session.restart() } label: { Label(t("terminal.restart"), systemImage: "arrow.counterclockwise") }
            Button(role: .destructive) { model.closeTerminal(cwd) } label: {
              Label(t("mobile.terminal.close"), systemImage: "xmark")
            }
          } label: {
            Image(systemName: "ellipsis")
          }
          .accessibilityLabel(Text(web: t("session.more")))
        }
      }
  }

  @ViewBuilder
  private func surface(_ session: TerminalSession) -> some View {
    #if os(iOS)
    TerminalSurface(session: session, dark: scheme == .dark)
    #else
    Palette.bg0
    #endif
  }
}

extension AppModel {
  /// The project's terminal, made on first use; actions go to the engine
  func terminal(_ cwd: String) -> TerminalSession {
    terminals.session(for: cwd) { [weak self] in self?.post($0) }
  }

  /// The chat menu's "Terminal": the project's terminal page, or in the demo the note that it takes an engine (a
  /// shell needs a computer to run on)
  func openTerminal(_ cwd: String) {
    guard live else {
      show(t("mobile.engineNeeded"), icon: .unplug)
      return
    }
    path.append(.terminal(cwd: cwd))
  }

  /// The page's "close": the shell ends and the page goes
  func closeTerminal(_ cwd: String) {
    terminal(cwd).kill()
    if path.last == .terminal(cwd: cwd) { path.removeLast() }
  }

  /// `-terminal FOLDER -type SCRIPT` (screenshots and the live check, docs/dev/ios.md): once the folder's terminal
  /// opened, types SCRIPT a beat at a time. `\r` in it is Return; `{hide}` / `{show}` put the keyboard away or bring it
  /// back, `{reopen}` leaves the page and comes back (the replay), `{restart}` and `{close}` are the menu's, `{wait}`
  /// is one more beat
  func scriptTerminal(_ cwd: String, _ script: String) async {
    let session = terminal(cwd)
    func opened() async {
      for _ in 0..<100 where !(session.opened && session.title != nil) { try? await Task.sleep(for: .milliseconds(100)) }
    }
    await opened()
    for step in TerminalScript.steps(script) {
      try? await Task.sleep(for: TerminalScript.beat)
      switch step {
      case .text(let text): session.input(Array(text.utf8))
      case .action("hide"): session.screen?.focus(false)
      case .action("show"): session.screen?.focus(true)
      case .action("restart"): session.restart()
      case .action("close"): closeTerminal(cwd)
      case .action("reopen"):
        if path.last == .terminal(cwd: cwd) { path.removeLast() }
        try? await Task.sleep(for: TerminalScript.beat)
        path.append(.terminal(cwd: cwd))
        await opened()
      case .action: break
      }
    }
  }
}

/// `-type`'s little language: typed text with `{action}`s between
enum TerminalScript {
  enum Step: Equatable { case text(String), action(String) }
  static let beat = Duration.seconds(1.5)

  static func steps(_ script: String) -> [Step] {
    var steps: [Step] = []
    var rest = Substring(script)
    while !rest.isEmpty {
      if rest.first == "{", let end = rest.firstIndex(of: "}") {
        steps.append(.action(String(rest[rest.index(after: rest.startIndex)..<end])))
        rest = rest[rest.index(after: end)...]
      } else {
        let end = rest.dropFirst().firstIndex(of: "{") ?? rest.endIndex
        // A launch argument carries Return as the two characters `\r` (or `\n`)
        let text = rest[..<end].replacingOccurrences(of: "\\r", with: "\r").replacingOccurrences(of: "\\n", with: "\r")
        steps.append(.text(text))
        rest = rest[end...]
      }
    }
    return steps
  }
}
