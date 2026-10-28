import AcpiraDesign
import AcpiraFixtures
import AcpiraModels
import SwiftUI
#if canImport(UIKit)
import UIKit
#endif

/// Launch options read from the process arguments, so screenshots can pin a fixture, theme, motion and locale:
/// `-fixture showcase -theme light -motion none -locale zh-CN -screen list|chat|new|settings`, or follow a development
/// engine instead of the fixtures: `-engine PORT:TOKEN` (apps/ios/scripts/engine.sh)
public struct LaunchOptions: Sendable {
  public var fixture: String
  public var theme: ColorScheme?
  public var motion: Bool
  public var locale: String?
  public var screen: String
  /// `-render closed|open|all`: draw the fixture's full-length chat into tmp/acpira-render.png instead of the screens
  public var render: String?
  /// `-popup attach|attach.N|attach.paste|model|model.N|focus…|demo|ring|ring.pick|account|toast|code|lightbox|context|quotes|
  /// fold:N|menu:X,Y`: open that picker, sheet, card or fold, or play the liquid transitions, on launch (screenshots and
  /// recordings, docs/dev/ios.md)
  public var popup: String?
  /// `-replay`: the chat plays its last agent turn back as a live stream (motion recordings)
  public var replay = false
  /// `-open open|all|every`: the live chat with those folds pre-opened, as `-render` draws them
  public var open: String?
  /// `-prompt TEXT`: the chat sends this on launch (screenshots of the demo reply)
  public var prompt: String?
  /// `-draft TEXT`: the chat's composer starts with this typed (screenshots of the lit send button)
  public var draft: String?
  /// `-engine PORT:TOKEN`: the live engine `acpira --ws` serves on the Mac's loopback (development)
  public var engine: String?
  /// `-session ID`: with an engine, the chat of that session opens on launch (what `-fixture` is to the demo)
  public var session: String?
  /// `-start FOLDER`: with an engine, a session of the default agent starts there on launch, `-prompt` its first prompt
  public var start: String?
  /// `-terminal FOLDER`: with an engine, that project's terminal opens on launch; `-type SCRIPT` then types into it
  /// (AppModel.scriptTerminal)
  public var terminal: String?
  public var type: String?

  public static func current(_ arguments: [String] = ProcessInfo.processInfo.arguments) -> LaunchOptions {
    func value(_ flag: String) -> String? {
      guard let at = arguments.firstIndex(of: flag), at + 1 < arguments.count else { return nil }
      return arguments[at + 1]
    }
    let theme: ColorScheme? = switch value("-theme") { case "light": .light; case "dark": .dark; default: nil }
    return LaunchOptions(fixture: value("-fixture") ?? "showcase", theme: theme, motion: value("-motion") != "none",
                         locale: value("-locale"), screen: value("-screen") ?? "chat", render: value("-render"),
                         popup: value("-popup"), replay: arguments.contains("-replay"), open: value("-open"),
                         prompt: value("-prompt"), draft: value("-draft"), engine: value("-engine"),
                         session: value("-session"), start: value("-start"), terminal: value("-terminal"),
                         type: value("-type"))
  }
}

extension EnvironmentValues {
  /// `-popup NAME`: the menu, sheet or panel a screenshot opens on launch
  @Entry var initialPopup: String? = nil
  /// `-draft TEXT`: what the chat's composer starts with
  @Entry var initialDraft: String? = nil
  /// The time quota resets count down from: the real clock, or the fixtures' clock while the app shows fixtures
  @Entry var now: Date = .now
  /// Seconds the app's clock runs ahead of the real one: the fixtures' clock runs on from launch (`AppModel.now`), so a
  /// live counter reads the time the demo turns are stamped with; 0 on the real clock
  @Entry var clockOffset: TimeInterval = 0
}

/// The app's root: the session list, pushing a chat or the new-session screen, the settings over it as a sheet; the app
/// model holds the stack, so a language switch (which rebuilds every screen) keeps the reader where they were
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
    if let render = options.render {
      RenderScreen(fixture: options.fixture, mode: render, scheme: options.theme ?? .dark)
        .preferredColorScheme(options.theme)
        .environment(\.motionEnabled, false)
    } else {
      screens
    }
  }

  private var scheme: ColorScheme? {
    if let pinned = options.theme { return pinned }
    return switch theme { case "light": .light; case "dark": .dark; default: nil }
  }

  private var screens: some View {
    NavigationStack(path: Bindable(model).path) {
      SessionListScreen()
        .navigationDestination(for: Route.self) { route in destination(route) }
    }
    // A language switch redraws every screen with the new table (the settings switch the table first)
    .id(locale)
    .toastLayer(model)
    .sheet(item: Bindable(model).shared) { file in
      ShareSheet(url: file.url).presentationDetents([.medium, .large])
    }
    // Outside the `.id` above, so a language picked in it rebuilds the screens under the sheet, not the sheet away
    .sheet(isPresented: Bindable(model).settingsOpen) { SettingsSheet() }
    .environment(model)
    .preferredColorScheme(scheme)
    .environment(\.motionEnabled, options.motion && motion)
    .environment(\.initialPopup, options.popup)
    .environment(\.initialDraft, options.draft)
    .environment(\.preOpen, PreOpen.folds(options.open))
    // Fixtures carry their own clock (quota resets count down from it); an engine runs on the real one
    .environment(\.now, model.live ? .now : Fixtures.now)
    .environment(\.clockOffset, model.clockOffset)
    .tint(Palette.fg1)
    .task {
      #if DEBUG && canImport(UIKit)
      if let popup = options.popup, popup.hasPrefix("menu:") { await MenuProbe.open(popup) }
      #endif
      if let folder = options.terminal {
        model.openTerminal(folder)
        if model.live, let script = options.type { await model.scriptTerminal(folder, script) }
        return
      }
      if model.live, let folder = options.start {
        model.engineNewSession(agent: nil, cwd: folder, text: options.prompt ?? "")
        return
      }
      guard case .chat(let id)? = model.path.last else { return }
      // The engine refuses an action for a session its view does not show: switch it first, in posting order
      model.open(id)
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
    case .terminal(let cwd):
      TerminalPage(cwd: cwd)
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
  @State private var done = false

  var body: some View {
    let preOpen = PreOpen.folds(mode)
    Text(verbatim: done ? "rendered" : "rendering")
      .task {
        guard let session = Fixtures.session(fixture) else { return }
        // Tasks do not run in ImageRenderer: the images are at hand before it draws
        await MediaLoader.demo.preload(session)
        let poster = ChatPoster(session: session, width: 402, preOpen: preOpen)
          .environment(\.colorScheme, scheme)
          .environment(\.motionEnabled, false)
          .environment(\.snapshot, true)
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
