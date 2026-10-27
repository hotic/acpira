import AcpiraDesign
import SwiftUI

// The phone's settings: the webview's appearance and language preferences that make sense on a phone (theme, motion,
// the interface language), the engine connection and the app's version, as grouped cards in the
// app's own palette. Preferences persist in UserDefaults (`Prefs`); launch arguments for screenshots override them.

/// UserDefaults keys of the phone's preferences
enum Prefs {
  /// `system`, `light` or `dark`
  static let theme = "acpira.theme"
  /// `auto` or one of L10n.locales
  static let locale = "acpira.locale"
  static let motion = "acpira.motion"
  /// The session list's view: `time` (day captions) or `project` (folding project groups)
  static let listView = "acpira.listView"

  /// The table a stored language choice selects
  static func resolve(_ locale: String) -> String { locale == "auto" ? L10n.preferred() : locale }
}

struct SettingsScreen: View {
  var onBack: () -> Void
  @AppStorage(Prefs.theme) private var theme = "system"
  @AppStorage(Prefs.locale) private var locale = "auto"
  @AppStorage(Prefs.motion) private var motion = true
  @State private var languages = false
  @Environment(\.initialPopup) private var initialPopup

  var body: some View {
    VStack(spacing: 0) {
      SettingsHeader(onBack: onBack)
      ScrollView(.vertical) {
        VStack(alignment: .leading, spacing: Metrics.s6) {
          SettingsGroup(t("settings.appearance.title")) {
            SettingsRow(.palette, t("settings.theme")) {
              Segmented(options: [("system", t("mobile.theme.system")), ("light", t("settings.theme.light")),
                                  ("dark", t("settings.theme.dark"))], value: $theme)
            }
            SettingsToggle(.sparkles, t("settings.motion"), isOn: $motion, divider: false)
          }
          SettingsGroup(t("settings.language")) {
            Button { languages = true } label: {
              SettingsRow(.globe, t("settings.language"), divider: false) {
                Text(web: languageName(locale), paragraph: false).foregroundStyle(Palette.fg3)
                Icon(.chevronRight, size: Metrics.chevron, strokeWidth: 1.75).foregroundStyle(Palette.fg3)
              }
            }
            .buttonStyle(ListRowStyle(active: false))
          }
          SettingsGroup(t("mobile.engine"), footnote: t("mobile.engine.desc")) {
            SettingsRow(.unplug, t("mobile.engine.status"), divider: false) {
              Text(web: t("mobile.engine.demo"), paragraph: false).foregroundStyle(Palette.fg3)
            }
          }
          SettingsGroup(t("mobile.about")) {
            SettingsRow(.bookOpen, t("settings.fact.version"), divider: false) {
              Text(verbatim: version).foregroundStyle(Palette.fg3).monospacedDigit()
            }
          }
        }
        .padding(.horizontal, Metrics.pad)
        .padding(.top, Metrics.s2)
        .padding(.bottom, Metrics.msgGap)
      }
      .scrollIndicators(.hidden)
    }
    .background(Palette.bg0)
    .pickerSheet(isPresented: $languages, title: t("settings.language"), current: locale) {
      PopupList(current: locale) {
        ForEach(["auto"] + L10n.locales, id: \.self) { id in
          MenuOption(languageName(id), checked: id == locale, checkSlot: true) {
            // The table switches before the stored choice changes, so the rebuilt screens read the new language
            L10n.setLocale(Prefs.resolve(id))
            locale = id
            languages = false
          } mark: {
            EmptyView()
          }
          .id(id)
        }
      }
    }
    .onAppear { if initialPopup == "language" { languages = true } }
  }

  private func languageName(_ id: String) -> String {
    id == "auto" ? t("mobile.language.system") : t("settings.language.\(id)")
  }

  private var version: String {
    let info = Bundle.main.infoDictionary
    let short = info?["CFBundleShortVersionString"] as? String ?? "0"
    let build = info?["CFBundleVersion"] as? String ?? "0"
    return "\(short) (\(build))"
  }
}

/// The header: back on the left, the title centred
private struct SettingsHeader: View {
  var onBack: () -> Void
  var body: some View {
    ZStack {
      Text(web: t("session.settings"), paragraph: false)
        .textStyle(TypeScale.text2, weight: .semibold)
        .foregroundStyle(Palette.fgStrong)
      HStack(spacing: 0) {
        IconButton(.chevronLeft, label: t("common.back"), action: onBack)
        Spacer(minLength: 0)
      }
      .padding(.horizontal, Metrics.pad - Metrics.s1_5)
    }
    .frame(height: Metrics.hdr)
  }
}

/// A titled card of rows, with an optional footnote under it
struct SettingsGroup<Content: View>: View {
  let title: String
  var footnote: String?
  let content: Content

  init(_ title: String, footnote: String? = nil, @ViewBuilder content: () -> Content) {
    self.title = title; self.footnote = footnote; self.content = content()
  }

  var body: some View {
    VStack(alignment: .leading, spacing: Metrics.s2) {
      Text(web: title, paragraph: false)
        .textStyle(TypeScale.text3, weight: .medium)
        .foregroundStyle(Palette.fg3)
        .padding(.horizontal, Metrics.s1)
      VStack(spacing: 0) { content }
        .background(RoundedRectangle(cornerRadius: Metrics.rLg).fill(Palette.bg1))
        .overlay(RoundedRectangle(cornerRadius: Metrics.rLg).strokeBorder(Palette.line))
        .clipShape(RoundedRectangle(cornerRadius: Metrics.rLg))
      if let footnote {
        Text(web: footnote)
          .textStyle(TypeScale.text3)
          .foregroundStyle(Palette.fg3)
          .fixedSize(horizontal: false, vertical: true)
          .padding(.horizontal, Metrics.s1)
      }
    }
  }
}

/// One row: the glyph, the label (and a line under it), the control or value on the right; a hairline below that
/// starts at the label column
struct SettingsRow<Trailing: View>: View {
  let icon: LucideIcon
  let label: String
  var detail: String?
  var divider: Bool
  let trailing: Trailing

  init(_ icon: LucideIcon, _ label: String, detail: String? = nil, divider: Bool = true, @ViewBuilder trailing: () -> Trailing) {
    self.icon = icon; self.label = label; self.detail = detail; self.divider = divider; self.trailing = trailing()
  }

  var body: some View {
    HStack(spacing: Metrics.gap) {
      Icon(icon, size: Metrics.iconCtl, strokeWidth: 1.75).foregroundStyle(Palette.fg2)
      VStack(alignment: .leading, spacing: Metrics.s0_5) {
        Text(web: label, paragraph: false).textStyle(TypeScale.text1).foregroundStyle(Palette.fg1)
        if let detail {
          Text(web: detail).textStyle(TypeScale.text3).foregroundStyle(Palette.fg3).fixedSize(horizontal: false, vertical: true)
        }
      }
      .frame(maxWidth: .infinity, alignment: .leading)
      HStack(spacing: Metrics.s1_5) { trailing }
        .textStyle(TypeScale.text2)
    }
    .padding(.horizontal, Metrics.pad)
    .padding(.vertical, Metrics.s2)
    .frame(minHeight: Metrics.settingsRow)
    .overlay(alignment: .bottom) {
      if divider {
        Rectangle().fill(Palette.line).frame(height: Metrics.hairline)
          .padding(.leading, Metrics.pad + Metrics.iconCtl + Metrics.gap)
      }
    }
    .contentShape(Rectangle())
  }
}

/// A row whose control is a switch
struct SettingsToggle: View {
  let icon: LucideIcon
  let label: String
  var detail: String?
  @Binding var isOn: Bool
  var divider = true

  init(_ icon: LucideIcon, _ label: String, detail: String? = nil, isOn: Binding<Bool>, divider: Bool = true) {
    self.icon = icon; self.label = label; self.detail = detail; _isOn = isOn; self.divider = divider
  }

  var body: some View {
    SettingsRow(icon, label, detail: detail, divider: divider) {
      Toggle(isOn: $isOn) { Text(web: label) }
        .labelsHidden()
        .toggleStyle(.switch)
        // The app tints with fg-1, which would draw a white track under a white knob in dark mode
        .tint(Palette.ok)
    }
  }
}

/// A segmented choice on the chip fill, the current segment raised on the card colour
struct Segmented: View {
  let options: [(id: String, label: String)]
  @Binding var value: String
  @Environment(\.motionEnabled) private var motion

  var body: some View {
    HStack(spacing: 0) {
      ForEach(options, id: \.id) { option in
        let on = option.id == value
        Button {
          withAnimation(motion ? Motion.ease : nil) { value = option.id }
        } label: {
          Text(web: option.label, paragraph: false)
            .textStyle(TypeScale.text3, weight: on ? .semibold : .regular)
            .foregroundStyle(on ? Palette.fgStrong : Palette.fg2)
            .lineLimit(1)
            .padding(.horizontal, Metrics.s2_5)
            .frame(height: Metrics.ctlSm)
            .background(RoundedRectangle(cornerRadius: Metrics.rSm).fill(on ? Palette.bg0 : .clear))
            .contentShape(RoundedRectangle(cornerRadius: Metrics.rSm))
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(on ? .isSelected : [])
      }
    }
    .padding(Metrics.s0_5)
    .background(RoundedRectangle(cornerRadius: Metrics.rMd).fill(Palette.chip))
    // Every label whole: the row's title gives way first
    .fixedSize()
  }
}
