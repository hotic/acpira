import AcpiraDesign
import SwiftUI
#if canImport(UIKit)
import UIKit
#endif

// The phone's settings: the webview's appearance and language preferences that make sense on a phone (theme, motion,
// the interface language, how a turn's process reads), the engine's settings a phone may change (EngineSettings.swift), the engine connection and
// the app's version, as a system form in a sheet. Preferences
// persist in UserDefaults (`Prefs`); launch arguments for screenshots override them.

/// UserDefaults keys of the phone's preferences
enum Prefs {
  /// `system`, `light` or `dark`
  static let theme = "acpira.theme"
  /// `auto` or one of L10n.locales
  static let locale = "acpira.locale"
  static let motion = "acpira.motion"
  /// The process group (`ProcessPrefs`): the step kinds left out (raw names, comma-separated), whether steps keep their
  /// icons, whether a finished turn's process stays open
  static let hiddenSteps = "acpira.process.hidden"
  static let stepIcons = "acpira.process.stepIcons"
  static let keepProcess = "acpira.process.keepOpen"
  /// The session list's view: `time` (day captions) or `project` (folding project groups)
  static let listView = "acpira.listView"
  /// Where pinned sessions sit by project: `top` or `project` (PinPlace)
  static let pinPlace = "acpira.pinPlace"
  /// The first-run flow is over, or was never due (a computer already paired when it would have shown)
  static let welcomed = "acpira.welcomed"
  /// Developer switches for the frame-rate and hitch overlay (`PerfHUD`)
  static let hudFPS = "acpira.hud.fps"
  static let hudHitches = "acpira.hud.hitches"

  /// The table a stored language choice selects
  static func resolve(_ locale: String) -> String { locale == "auto" ? L10n.preferred() : locale }
}

/// The settings: a sheet over the list (as Trae's phone app brings its settings up), its title
/// centred in the sheet's own bar with close at the bar's end. A pushed page put a large title under back, stacked
/// at the left edge, where the system's own pages centre it beside back
struct SettingsSheet: View {
  @Environment(\.dismiss) private var dismiss
  /// `-settingsPage agents | agent:ID | models:ID | providers | developer` opens a page for screenshots
  @State private var path = SettingsRoute.launchPath(UserDefaults.standard.string(forKey: "settingsPage"))
  var body: some View {
    NavigationStack(path: $path) {
      SettingsScreen()
        .backWithoutMenu()
        .toolbar {
          ToolbarItem(placement: .primaryAction) { CloseButton { dismiss() } }
        }
    }
  }
}

/// The settings as a system `Form` under an inline title: menu pickers, a toggle and read-only values in grouped
/// sections
struct SettingsScreen: View {
  @AppStorage(Prefs.theme) private var theme = "system"
  @AppStorage(Prefs.locale) private var locale = "auto"
  @AppStorage(Prefs.motion) private var motion = true
  @Environment(\.pairing) private var pairing
  @Environment(AppModel.self) private var model

  var body: some View {
    Form {
      // Everything about how the app looks and reads in one group: the two menus side by side, then the motion switch
      Section(t("settings.appearance.title")) {
        Picker(t("settings.theme"), selection: $theme) {
          Text(web: t("mobile.theme.system")).tag("system")
          Text(web: t("settings.theme.light")).tag("light")
          Text(web: t("settings.theme.dark")).tag("dark")
        }
        // The same system menu as the theme, so every choice in the group opens the same way; a pick switches the
        // table first, so the rebuilt screens read the new language
        Picker(t("settings.language"), selection: Binding(get: { locale }, set: { id in
          L10n.setLocale(Prefs.resolve(id))
          locale = id
        })) {
          ForEach(["auto"] + L10n.locales, id: \.self) { id in
            Text(verbatim: languageName(id)).tag(id)
          }
        }
        // The app tints controls with the body ink; a switch keeps the system's own green (not the app's muted
        // --ok) so "on" reads as on, as in every system app
        Toggle(t("settings.motion"), isOn: $motion).tint(.green)
      }
      ProcessSettingsSection()
      // The engine's own settings, shared with the computer
      EngineSettingsSections()
      UsageSettingsSection()
      if let pairing {
        // Paired remote access: the paired computers take the engine row's place
        PairComputersSection(pairing: pairing)
        NotificationsSection()
      } else {
        Section {
          // One fact: the state, with its dot; what it means goes in the footnote
          LabeledContent(t("mobile.engine.status")) {
            HStack(spacing: Metrics.s1_5) {
              Circle().fill(model.live ? Palette.ok : Palette.fg3).frame(width: Metrics.rowDot, height: Metrics.rowDot)
              Text(web: t(model.live ? "connections.state.connected" : "connections.state.off"), paragraph: false)
            }
          }
        } header: {
          Text(web: t("mobile.engine"))
        } footer: {
          // The demo's word on what it shows; a development engine needs none
          if !model.live { Text(web: t("mobile.engine.desc")) }
        }
      }
      Section(t("mobile.about")) {
        LabeledContent(t("settings.fact.version"), value: version)
        // Developer tools sit under the version, one tap away, as the system's own Developer page does
        NavigationLink(value: SettingsRoute.developer) { Text(web: t("mobile.dev"), paragraph: false) }
      }
    }
    // The sheet sits outside the root's `.id(locale)`, and its sections are views of their own whose inputs a language
    // pick leaves unchanged, so SwiftUI keeps their old text (and a menu picker its old value label); a new identity
    // rebuilds the whole form in the new table. The language row sits in the first section, so the reset scroll is moot
    .id(locale)
    .formPage()
    .pairSettingsHooks(pairing)
    .navigationDestination(for: SettingsRoute.self) { route in
      // Every pushed page hands on the same untitled back item as the root (`backWithoutMenu`)
      Group {
        switch route {
        case .agent(let id): AgentSettingsPage(id: id)
        case .models(let id): ModelsSettingsPage(id: id)
        case .providers: ProvidersPage()
        case .steps: StepsSettingsPage()
        case .usage: UsagePage()
        case .developer: DeveloperSettingsPage()
        }
      }
      .backWithoutMenu()
    }
    .navigationTitle(t("session.settings"))
    .navigationBarTitleDisplayModeInline()
  }

  private var version: String {
    let info = Bundle.main.infoDictionary
    return "\(info?["CFBundleShortVersionString"] as? String ?? "0") (\(info?["CFBundleVersion"] as? String ?? "0"))"
  }
}

/// How a turn's work reads, the desktop's Process group (Appearance.tsx): which step kinds it lists, a page of its own
/// as the system's multi-choice rows are, then the two switches, titles alone like the other switches in the sheet
struct ProcessSettingsSection: View {
  @AppStorage(Prefs.hiddenSteps) private var hiddenSteps = ""
  @AppStorage(Prefs.stepIcons) private var stepIcons = true
  @AppStorage(Prefs.keepProcess) private var keepProcess = false

  var body: some View {
    let shown = StepKind.allCases.count - ProcessPrefs.hidden(hiddenSteps).count
    Section {
      NavigationLink(value: SettingsRoute.steps) {
        // A value, not a phrase: every kind, none, or the bare count between
        LabeledContent(t("settings.steps")) {
          Text(verbatim: shown == StepKind.allCases.count ? t("settings.steps.all") : shown == 0 ? t("settings.steps.none") : "\(shown)")
        }
      }
      Toggle(t("settings.stepIcons"), isOn: $stepIcons).tint(.green)
      Toggle(t("settings.keepProcess"), isOn: $keepProcess).tint(.green)
    } header: {
      Text(web: t("settings.group.process"))
    }
  }
}

/// The step kinds a turn's process lists, one switch each in reading order; the reply itself always shows
struct StepsSettingsPage: View {
  @AppStorage(Prefs.hiddenSteps) private var hiddenSteps = ""

  var body: some View {
    let hidden = ProcessPrefs.hidden(hiddenSteps)
    Form {
      Section {
        ForEach(StepKind.allCases, id: \.self) { kind in
          Toggle(isOn: Binding(get: { !hidden.contains(kind) }, set: { on in
            var next = ProcessPrefs.hidden(hiddenSteps)
            if on { next.remove(kind) } else { next.insert(kind) }
            hiddenSteps = ProcessPrefs.stored(next)
          })) {
            Text(web: t("settings.steps.\(kind.rawValue)"), paragraph: false)
          }
          .tint(.green)
        }
      } footer: {
        Text(web: t("settings.steps.desc"))
      }
    }
    .formPage()
    .navigationTitle(t("settings.steps"))
    .navigationBarTitleDisplayModeInline()
  }
}

/// Notifications from the paired computers: the system's word on them, one tap to ask or to change it there. Sound,
/// banners and Focus are the system's settings, so the app keeps no switches of its own
struct NotificationsSection: View {
  @State private var push = PushCenter.shared

  var body: some View {
    Section {
      Button(action: act) {
        LabeledContent(t("mobile.notify")) {
          Text(web: t(state), paragraph: false)
        }
      }
      .foregroundStyle(Palette.fg1)
    } footer: {
      Text(web: t("mobile.notify.desc"))
    }
    .task { await push.refresh() }
  }

  private var state: String {
    switch push.permission {
    case .on: "mobile.notify.on"
    case .off: "mobile.notify.off"
    case .notAsked, .unknown: "mobile.notify.ask"
    }
  }

  private func act() {
    switch push.permission {
    case .notAsked, .unknown: Task { await push.activate() }
    case .on, .off:
      #if canImport(UIKit)
      if let url = URL(string: UIApplication.openNotificationSettingsURLString) { UIApplication.shared.open(url) }
      #endif
    }
  }
}

/// Settings → Usage: usage across every session; an older engine predating paired usage reads stays off
private struct UsageSettingsSection: View {
  @Environment(AppModel.self) private var model

  var body: some View {
    Section {
      NavigationLink(value: SettingsRoute.usage) {
        Text(web: t("settings.nav.usage"), paragraph: false)
      }
      .disabled(!model.supportsUsage)
    } footer: {
      if !model.supportsUsage { Text(web: t("mobile.outdated")) }
    }
  }
}

/// Developer: the on-screen frame-rate and hitch overlay (`PerfHUD`), for judging smoothness on the phone itself
struct DeveloperSettingsPage: View {
  @AppStorage(Prefs.hudFPS) private var fps = false
  @AppStorage(Prefs.hudHitches) private var hitches = false

  var body: some View {
    Form {
      Section {
        Toggle(t("mobile.dev.fps"), isOn: $fps).tint(.green)
        Toggle(t("mobile.dev.hitches"), isOn: $hitches).tint(.green)
      } footer: {
        Text(web: t("mobile.dev.desc"))
      }
    }
    .formPage()
    .navigationTitle(t("mobile.dev"))
    .navigationBarTitleDisplayModeInline()
  }
}

/// A language's own name, or "follow the system"
func languageName(_ id: String) -> String {
  id == "auto" ? t("mobile.language.system") : t("settings.language.\(id)")
}
