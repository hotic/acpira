import AcpiraDesign
import SwiftUI

// The phone's settings: the webview's appearance and language preferences that make sense on a phone (theme, motion,
// the interface language), the engine's settings a phone may change (EngineSettings.swift), the engine connection and
// the app's version, as a system form in a sheet. Preferences
// persist in UserDefaults (`Prefs`); launch arguments for screenshots override them.

/// UserDefaults keys of the phone's preferences
enum Prefs {
  /// `system`, `light` or `dark`
  static let theme = "acpira.theme"
  /// `auto` or one of L10n.locales
  static let locale = "acpira.locale"
  static let motion = "acpira.motion"
  /// The session list's view: `time` (day captions) or `project` (folding project groups)
  static let listView = "acpira.listView"
  /// Where pinned sessions sit by project: `top` or `project` (PinPlace)
  static let pinPlace = "acpira.pinPlace"

  /// The table a stored language choice selects
  static func resolve(_ locale: String) -> String { locale == "auto" ? L10n.preferred() : locale }
}

/// The settings: a sheet over the list (as Trae's phone app brings its settings up), its title
/// centred in the sheet's own bar with close at the bar's end. A pushed page put a large title under back, stacked
/// at the left edge, where the system's own pages centre it beside back
struct SettingsSheet: View {
  @Environment(\.dismiss) private var dismiss
  /// `-settingsPage agents | agent:ID | models:ID` opens a page for screenshots
  @State private var path = SettingsRoute.launchPath(UserDefaults.standard.string(forKey: "settingsPage"))
  var body: some View {
    NavigationStack(path: $path) {
      SettingsScreen()
        .toolbar {
          ToolbarItem(placement: .primaryAction) { Button(role: .close) { dismiss() } }
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
  @Environment(AppModel.self) private var model

  var body: some View {
    Form {
      // Everything about how the app looks and reads in one group: theme, motion, language
      Section(t("settings.appearance.title")) {
        Picker(t("settings.theme"), selection: $theme) {
          Text(web: t("mobile.theme.system")).tag("system")
          Text(web: t("settings.theme.light")).tag("light")
          Text(web: t("settings.theme.dark")).tag("dark")
        }
        // The app tints controls with the body ink; a switch keeps the system's own green (not the app's muted
        // --ok) so "on" reads as on, as in every system app
        Toggle(t("settings.motion"), isOn: $motion).tint(.green)
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
      }
      // The engine's own settings, shared with the computer
      EngineSettingsSections()
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
      Section(t("mobile.about")) {
        LabeledContent(t("settings.fact.version"), value: version)
      }
    }
    .formPage()
    .navigationDestination(for: SettingsRoute.self) { route in
      switch route {
      case .agents: AgentsSettingsPage()
      case .agent(let id): AgentSettingsPage(id: id)
      case .models(let id): ModelsSettingsPage(id: id)
      }
    }
    .navigationTitle(t("session.settings"))
    .navigationBarTitleDisplayModeInline()
  }

  private var version: String {
    let info = Bundle.main.infoDictionary
    return "\(info?["CFBundleShortVersionString"] as? String ?? "0") (\(info?["CFBundleVersion"] as? String ?? "0"))"
  }
}

/// A language's own name, or "follow the system"
func languageName(_ id: String) -> String {
  id == "auto" ? t("mobile.language.system") : t("settings.language.\(id)")
}
