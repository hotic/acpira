import AcpiraDesign
import AcpiraFixtures
import AcpiraModels
import SwiftUI
#if canImport(UIKit)
import UIKit
#endif

/// Launch options read from the process arguments, so screenshots can pin a fixture, theme, motion and locale:
/// `-fixture showcase -theme light -motion none -locale zh-CN -screen list|chat|new|settings`
public struct LaunchOptions: Sendable {
  public var fixture: String
  public var theme: ColorScheme?
  public var motion: Bool
  public var locale: String?
  public var screen: String
  /// `-render closed|open|all`: draw the fixture's full-length chat into tmp/acpira-render.png instead of the screens
  public var render: String?
  /// `-popup model|attach|project|agent|ring|more|language`: open that menu or sheet, or put the new-session agent ring
  /// out, on launch (screenshots of the panels)
  public var popup: String?
  /// `-replay`: the chat plays its last agent turn back as a live stream (motion recordings)
  public var replay = false
  /// `-open open|all|every`: the live chat with those folds pre-opened, as `-render` draws them
  public var open: String?
  /// `-prompt TEXT`: the chat sends this on launch (screenshots of the demo reply)
  public var prompt: String?
  /// `-draft TEXT`: the chat's composer starts with this typed (screenshots of the lit send button)
  public var draft: String?
  /// `-lab a` or `-lab header=float,…`: the design study's style for this launch (ShellStyle), over the stored one
  public var lab: String?

  public static func current(_ arguments: [String] = ProcessInfo.processInfo.arguments) -> LaunchOptions {
    func value(_ flag: String) -> String? {
      guard let at = arguments.firstIndex(of: flag), at + 1 < arguments.count else { return nil }
      return arguments[at + 1]
    }
    let theme: ColorScheme? = switch value("-theme") { case "light": .light; case "dark": .dark; default: nil }
    return LaunchOptions(fixture: value("-fixture") ?? "showcase", theme: theme, motion: value("-motion") != "none",
                         locale: value("-locale"), screen: value("-screen") ?? "chat", render: value("-render"),
                         popup: value("-popup"), replay: arguments.contains("-replay"), open: value("-open"),
                         prompt: value("-prompt"), draft: value("-draft"), lab: value("-lab"))
  }
}

/// The app's root: the session list, pushing a chat or the new-session screen, the settings over it as a sheet (a page
/// under custom chrome); the app model holds the stack, so a language switch (which rebuilds every screen) keeps the
/// reader where they were
public struct RootView: View {
  private let options: LaunchOptions
  @State private var model: AppModel
  @AppStorage(Prefs.theme) private var theme = "system"
  @AppStorage(Prefs.locale) private var locale = "auto"
  @AppStorage(Prefs.motion) private var motion = true

  public init(options: LaunchOptions = .current()) {
    self.options = options
    // `-locale` pins a screenshot's language; otherwise the stored choice, then the device's
    L10n.setLocale(options.locale ?? Prefs.resolve(UserDefaults.standard.string(forKey: Prefs.locale) ?? "auto"))
    InterFont.register()
    _model = State(initialValue: AppModel(options: options))
  }

  public var body: some View {
    // The reading sizes are read straight from the tokens, so they are switched before anything draws; a change
    // rebuilds the screens (the `.id` below)
    let _ = Reading.phone = shellStyle.scale == .phone
    if let render = options.render {
      RenderScreen(fixture: options.fixture, mode: render, scheme: options.theme ?? .dark, style: shellStyle)
        .preferredColorScheme(options.theme)
        .environment(\.motionEnabled, false)
    } else {
      screens
    }
  }

  /// The design study's look: pinned by `-lab`, else the stored one (preset 丁 by default)
  /// The look: preset 丁 (system chrome), picked on 2026-10-27. The settings' switcher is gone, so a code stored by it
  /// (UserDefaults `acpira.lab.style`) is no longer read; `-lab` still pins another one for screenshots
  private var shellStyle: ShellStyle { ShellStyle(code: options.lab ?? "d") }

  private var scheme: ColorScheme? {
    if let pinned = options.theme { return pinned }
    return switch theme { case "light": .light; case "dark": .dark; default: nil }
  }

  private var screens: some View {
    NavigationStack(path: Bindable(model).path) {
      SessionListScreen()
        .navigationDestination(for: Route.self) { route in
          destination(route).navigationBar(system: shellStyle.chrome == .system)
        }
    }
    // A language switch redraws every screen with the new table (SettingsScreen switches the table first), a
    // switch of reading sizes with the new tokens
    .id("\(locale)|\(shellStyle.scale)")
    .toastLayer(model)
    // Design study: the style switcher (LabOverlay) is off now that a look is picked; `-lab` still pins one
    .sheet(item: Bindable(model).shared) { file in
      ShareSheet(url: file.url).presentationDetents([.medium, .large])
    }
    // Outside the `.id` above, so a language picked in it rebuilds the screens under the sheet, not the sheet away
    .sheet(isPresented: Bindable(model).settingsOpen) { SettingsSheet() }
    .environment(model)
    .preferredColorScheme(scheme)
    .environment(\.motionEnabled, options.motion && motion)
    .environment(\.shellStyle, shellStyle)
    .environment(\.initialPopup, options.popup)
    .environment(\.initialDraft, options.draft)
    .environment(\.preOpen, PreOpen.folds(options.open))
    // Fixtures carry their own clock (quota resets count down from it)
    .environment(\.now, Fixtures.now)
    .environment(\.clockOffset, model.clockOffset)
    .tint(Palette.fg1)
    .task {
      // `-screen settings` under custom chrome: its settings are a page
      if shellStyle.chrome == .custom && model.settingsOpen { model.settingsOpen = false; model.path = [.settings] }
      #if DEBUG && canImport(UIKit)
      if let popup = options.popup, popup.hasPrefix("menu:") { await MenuProbe.open(popup) }
      #endif
      guard case .chat(let id)? = model.path.last else { return }
      if options.replay { model.replay(id) }
      if let prompt = options.prompt { model.send(id, text: prompt, attachments: []) }
    }
  }

  @ViewBuilder
  private func destination(_ route: Route) -> some View {
    switch route {
    case .chat(let id):
      ChatScreen(id: id)
    case .new(let agent, let cwd):
      let start = agent ?? model.summaries.max { $0.updatedAt < $1.updatedAt }?.agent
        ?? launchable(model.agents).first { $0.available != false }?.id ?? ""
      NewSessionScreen(agent: start, cwd: cwd ?? model.projects.first ?? "", controls: model.controls(for: start))
    case .settings:
      SettingsScreen { if !model.path.isEmpty { model.path.removeLast() } }
    }
  }
}

#if canImport(UIKit)
/// The system share sheet for an exported file
struct ShareSheet: UIViewControllerRepresentable {
  let url: URL
  func makeUIViewController(context: Context) -> UIActivityViewController {
    UIActivityViewController(activityItems: [url], applicationActivities: nil)
  }
  func updateUIViewController(_ controller: UIActivityViewController, context: Context) {}
}
#else
struct ShareSheet: View {
  let url: URL
  var body: some View { Text(verbatim: url.path) }
}
#endif

/// Renders the poster once laid out and writes it where `ios/scripts/render.sh` picks it up
struct RenderScreen: View {
  let fixture: String
  let mode: String
  let scheme: ColorScheme
  /// The live screens' look (`-lab`, preset 丁 by default): the renderer's content is its own hierarchy and
  /// sees none of the root's environment
  let style: ShellStyle
  @State private var done = false

  var body: some View {
    let preOpen = PreOpen.folds(mode)
    Text(verbatim: done ? "rendered" : "rendering")
      .task {
        guard let session = Fixtures.session(fixture) else { return }
        let poster = ChatPoster(session: session, width: 402, preOpen: preOpen)
          .environment(\.colorScheme, scheme)
          .environment(\.motionEnabled, false)
          .environment(\.snapshot, true)
          .environment(\.shellStyle, style)
          // The fixture's clock, as the live chat reads it
          .environment(\.clockOffset, Fixtures.now.timeIntervalSinceNow)
        let renderer = ImageRenderer(content: poster)
        renderer.scale = 2  // the LAB reference shots are @2x
        #if canImport(UIKit)
        // The palette's dynamic UIColors resolve against the current trait collection, not the SwiftUI environment
        let traits = UITraitCollection(userInterfaceStyle: scheme == .dark ? .dark : .light)
        var image: UIImage?
        // The first pass lays out; measured paddings (chip lines) settle in state, the second pass draws with them
        traits.performAsCurrent { _ = renderer.uiImage }
        try? await Task.sleep(for: .milliseconds(500))
        traits.performAsCurrent { image = renderer.uiImage }
        if let data = image?.pngData() {
          try? data.write(to: FileManager.default.temporaryDirectory.appendingPathComponent("acpira-render.png"))
        }
        #endif
        done = true
      }
  }
}

#Preview {
  RootView(options: LaunchOptions(fixture: "showcase", theme: .dark, motion: false, locale: "zh-CN", screen: "chat"))
}
