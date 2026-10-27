import AcpiraDesign
import AcpiraModels
import SwiftUI
#if canImport(UIKit)
import UIKit
#endif

// Design study `chrome=system` (2026-10-26): the system's own menus, bars and settings form in place of the app's
// popups and sheets. The "…", project and agent pickers are system `Menu`s with the same choices as the custom
// panels (SessionActionsMenu, ProjectList, AgentList). The model keeps its own panel (the effort scale and Fast stay
// open while they change) inside a system popover, and attach opens a tray inside the composer (Composer.swift).

// MARK: - Marks as menu images

/// A vendor mark drawn into a template image: a `Menu` row only renders `Image` and `Text`, so the custom-drawn mark
/// is rasterised once per brand and tinted by the menu like an SF Symbol
@MainActor
enum MenuMark {
  private static var cache: [String: Image] = [:]

  static func image(brand: String?, name: String) -> Image {
    let key = "\(brand ?? "")|\(name)"
    if let hit = cache[key] { return hit }
    let renderer = ImageRenderer(content: VendorMark(brand: brand, name: name, size: Metrics.glassIcon)
      .foregroundStyle(.black)
      .frame(width: Metrics.glassIcon, height: Metrics.glassIcon))
    renderer.scale = 3
    #if canImport(UIKit)
    let image = renderer.uiImage.map { Image(uiImage: $0.withRenderingMode(.alwaysTemplate)) } ?? Image(systemName: "circle")
    #else
    let image = renderer.nsImage.map { Image(nsImage: $0) } ?? Image(systemName: "circle")
    #endif
    cache[key] = image
    return image
  }
}

// MARK: - Session "…"

/// SessionActionsMenu as a system menu: the account, pin and rename, the exports in a submenu, settings, delete
struct NativeSessionMenu: View {
  var pinned: Bool
  var actions = HeaderActions()

  var body: some View {
    if let account = actions.account {
      Section { Button(action: account) { Label(t("common.account"), systemImage: "person.crop.circle") } }
    }
    Section {
      Button(action: actions.pin) { Label(pinned ? t("common.unpin") : t("common.pin"), systemImage: pinned ? "pin.slash" : "pin") }
      Button(action: actions.rename) { Label(t("common.rename"), systemImage: "pencil") }
      Menu {
        Button { actions.export(false) } label: { Label(t("session.export.markdown"), systemImage: "doc.text") }
        Button { actions.export(true) } label: { Label(t("session.export.json"), systemImage: "curlybraces") }
      } label: {
        Label(t("session.export"), systemImage: "square.and.arrow.up")
      }
    }
    Section { Button(action: actions.settings) { Label(t("session.settings"), systemImage: "gearshape") } }
    Section { Button(role: .destructive, action: actions.delete) { Label(t("common.delete"), systemImage: "trash") } }
  }
}

// MARK: - Project / agent

/// ProjectList as menu rows: the recent folders as a checked picker, then "Add folder" (the caller raises its alert)
struct NativeProjectMenu: View {
  let cwd: String
  let recent: [String]
  var onSelect: (String) -> Void
  var onAdd: () -> Void

  var body: some View {
    var seen = Set<String>()
    let folders = ([cwd] + recent).filter { !$0.isEmpty && seen.insert($0).inserted }
    Section(t("mobile.projects.recent")) {
      Picker(selection: Binding(get: { cwd }, set: { onSelect($0) })) {
        ForEach(folders, id: \.self) { folder in
          // The subtitle line of a menu row: the full path under the folder's name
          Button {} label: {
            Text(verbatim: folderName(folder))
            Text(verbatim: folder)
          }
          .tag(folder)
        }
      } label: {
        EmptyView()
      }
      .pickerStyle(.inline)
    }
    Button(action: onAdd) { Label(t("mobile.projects.add"), systemImage: "folder.badge.plus") }
  }
}

/// The add-folder alert the project menus raise (ProjectList's, for a caller that shows a system menu)
struct AddProjectAlert: ViewModifier {
  @Binding var isPresented: Bool
  var onAdd: (String) -> Void
  @State private var path = ""

  func body(content: Content) -> some View {
    content.alert(t("mobile.projects.add"), isPresented: $isPresented) {
      TextField(text: $path) { Text(web: t("mobile.projects.path")) }
        .autocorrectionDisabled()
        #if os(iOS)
        .textInputAutocapitalization(.never)
        #endif
      Button(t("mobile.cancel"), role: .cancel) { path = "" }
      Button(t("mobile.add")) {
        let folder = path.trimmingCharacters(in: .whitespacesAndNewlines)
        path = ""
        if !folder.isEmpty { onAdd(folder) }
      }
    } message: {
      Text(web: t("mobile.projects.pathHint"))
    }
  }
}

/// AgentList as a checked picker; an agent that is not installed stays listed, disabled
struct NativeAgentMenu: View {
  let agents: [AgentInfo]
  let current: AgentId
  var onSelect: (AgentId) -> Void

  var body: some View {
    Picker(selection: Binding(get: { current }, set: { onSelect($0) })) {
      ForEach(launchable(agents)) { a in
        Label { Text(verbatim: a.name) } icon: { MenuMark.image(brand: a.id, name: a.name) }
          .tag(a.id)
          .disabled(a.available == false)
      }
    } label: {
      EmptyView()
    }
    .pickerStyle(.inline)
  }
}

// MARK: - Composer chips

/// Design study `chips=menu`: `+` as the system's menu of the three sources, on the same glass circle as the liquid
/// chip; a pick closes it and opens that source's picker
struct MenuAttach: View {
  var onPick: (AttachSource) -> Void

  var body: some View {
    Menu {
      Button { onPick(.photos) } label: { Label(t("mobile.attach.photos"), systemImage: "photo.on.rectangle") }
      Button { onPick(.camera) } label: { Label(t("mobile.attach.camera"), systemImage: "camera") }
      Button { onPick(.files) } label: { Label(t("mobile.attach.files"), systemImage: "folder") }
    } label: {
      Icon(.plus, size: Metrics.barIcon, strokeWidth: 1.75)
        .foregroundStyle(Palette.fg1)
        .frame(width: Metrics.barCtl, height: Metrics.barCtl)
        .contentShape(Circle())
    }
    .menuStyle(.button)
    .buttonStyle(.plain)
    .glassEffect(GlassStyle.control.interactive(), in: Circle())
    .accessibilityLabel(Text(web: t("mobile.attach")))
  }
}

/// Design study `chips=menu`: the model chip as the system's menu, the counterpart of the liquid beans. The chip is the
/// same glass circle with the model's mark; its menu holds one submenu per setting (the model, the reasoning level,
/// the mode), each naming its current value under its title, and Fast as a checkmark. A pick keeps the menu open, so
/// several settings change in one go; a tap outside closes it
struct MenuControls: View {
  let controls: SessionControls
  var onSetMode: (String) -> Void
  var onSetConfig: (String, String) -> Void

  var body: some View {
    let c = ModelChoices(controls: controls, onSetConfig: onSetConfig)
    let effort = c.effort.flatMap { $0.levels.count > 1 ? $0 : nil }
    if c.cur != nil || effort != nil || !c.modes.isEmpty || c.fast != nil {
      Menu {
        if let cur = c.cur {
          Menu {
            Picker(selection: Binding(get: { cur.key }, set: { key in
              if let f = c.families.first(where: { $0.key == key }) { c.pickFamily(f) }
            })) {
              ForEach(c.families, id: \.key) { f in
                Label { Text(verbatim: f.name) } icon: { MenuMark.image(brand: f.brand, name: f.name) }
                  .tag(f.key)
              }
            } label: {
              EmptyView()
            }
            .pickerStyle(.inline)
          } label: {
            // Title, the value in force under it, then the glyph: the shape a system submenu row reads
            Text(verbatim: t("stats.model"))
            Text(verbatim: cur.name)
            MenuMark.image(brand: cur.brand, name: cur.name)
          }
        }
        if let effort {
          Menu {
            Picker(selection: Binding(get: { effort.value }, set: { effort.pick($0) })) {
              ForEach(effort.levels, id: \.id) { Text(verbatim: $0.name).tag($0.id) }
            } label: {
              EmptyView()
            }
            .pickerStyle(.inline)
          } label: {
            Text(verbatim: t("composer.effort"))
            Text(verbatim: effort.levels.first { $0.id == effort.value }?.name ?? "")
            Image(systemName: "brain")
          }
        }
        if !c.modes.isEmpty {
          Menu {
            Picker(selection: Binding(get: { c.mode?.id ?? "" }, set: { onSetMode($0) })) {
              ForEach(c.modes) { m in
                Label(modeLabel(m), systemImage: modeSymbol(m)).tag(m.id)
              }
            } label: {
              EmptyView()
            }
            .pickerStyle(.inline)
          } label: {
            Text(verbatim: t("composer.mode"))
            Text(verbatim: c.mode.map(modeLabel) ?? "")
            Image(systemName: c.mode.map(modeSymbol) ?? "slider.horizontal.3")
          }
        }
        if let fast = c.fast {
          Toggle(isOn: Binding(get: { fast.on }, set: { fast.set($0) })) { Label("Fast", systemImage: "bolt") }
            .disabled(fast.disabled)
        }
      } label: {
        ChipMark(cur: c.cur, fast: c.fast?.on == true)
          .foregroundStyle(Palette.fg1)
          .frame(width: Metrics.barCtl, height: Metrics.barCtl)
          .contentShape(Circle())
      }
      .menuStyle(.button)
      .buttonStyle(.plain)
      .menuStaysOpen()
      .glassEffect(GlassStyle.control.interactive(), in: Circle())
      .accessibilityLabel(Text(web: c.cur?.name ?? t("stats.model")))
    }
  }
}

/// modeIcon's glyph as an SF Symbol, for a mode's row in a system menu
func modeSymbol(_ mode: SessionOption) -> String {
  switch modeIcon(mode) {
  case .shieldOff: "shield.slash"
  case .bug: "ladybug"
  case .listTodo: "checklist"
  case .messageCircleQuestionMark: "questionmark.bubble"
  case .sparkles: "sparkles"
  case .code: "chevron.left.forwardslash.chevron.right"
  case .infinity: "infinity"
  default: "circle.dashed"
  }
}

/// AccountPanel as a system sheet: a form under an inline title, the system's close button and "+" in its bar, on
/// the sheet's own glass. A row is one stored login (identity, check, quota bars); a swipe removes it
struct NativeAccountSheet: View {
  let agent: AgentInfo
  let accounts: [AccountInfo]
  var accountId: String?
  var onSelect: (String) -> Void
  var onAdd: () -> Void
  var onRemove: (String) -> Void = { _ in }
  @Environment(\.dismiss) private var dismiss

  var body: some View {
    NavigationStack {
      Form {
        if let local = agent.localAccount {
          // Before a login is found the label is only the product name the bar already shows
          let known = local.label != agent.name || local.detail != nil
          Section {
            if known {
              NativeAccountRow(title: local.label, plan: local.detail, caption: t("quota.officialAccount"), quota: local.quota,
                               status: local.quota == nil ? t("quota.status.\(local.status)") : nil)
            } else {
              NativeAccountRow(title: local.status == "login_required" ? t("composer.notLoggedIn") : t("quota.officialAccount"),
                               caption: t("quota.status.\(local.status)"))
            }
          } footer: {
            if local.quota != nil { Text(web: t("quota.local.desc")).textStyle(TypeScale.footnote) }
          }
        } else {
          Section {
            if accounts.isEmpty {
              NativeAccountRow(title: t("settings.agent.accounts.none"), caption: t("composer.noAccounts"))
            }
            ForEach(accounts) { a in
              Button {
                onSelect(a.id)
                dismiss()
              } label: {
                NativeAccountRow(title: a.label, caption: a.detail, checked: a.id == accountId, checkSlot: true, quota: a.quota)
                  .contentShape(Rectangle())
              }
              .buttonStyle(.plain)
              .swipeActions {
                Button(role: .destructive) { onRemove(a.id) } label: { Label(t("common.delete"), systemImage: "trash") }
              }
            }
          } footer: {
            if agent.credentialsLocked == true { Text(web: t("notice.locked.short")) }
          }
        }
      }
      .listSectionSpacingCompact()
      .listRowBackground(Palette.formRow)
      // The sheet's own glass shows through
      .scrollContentBackground(.hidden)
      .navigationTitle(t("common.account"))
      // The agent whose channel is open names the sheet under its title, not a caption over the rows
      .navigationSubtitle(agent.name)
      .navigationBarTitleDisplayModeInline()
      .toolbar {
        ToolbarItem(placement: .cancellationAction) {
          Button(role: .close) { dismiss() }
        }
        if agent.localAccount == nil {
          ToolbarItem(placement: .primaryAction) {
            Button(action: onAdd) { Image(systemName: "plus") }
              .accessibilityLabel(Text(web: t("composer.addAccount")))
          }
        }
      }
    }
    .presentationDetents([.medium, .large])
  }
}

/// One login in the system account sheet, at the phone's reading sizes (the popup's AccountIdentity is set for a dense
/// desktop panel): a list row's avatar, the name with its plan as a tag over a caption, then the quota windows a step
/// above the popup's. The check keeps its slot in a list of logins, so picking one moves nothing
struct NativeAccountRow: View {
  var title: String
  var plan: String?
  var caption: String?
  var checked = false
  var checkSlot = false
  var quota: AccountQuota?
  /// Shown in place of the quota when there is none (still loading, signed out …)
  var status: String?

  var body: some View {
    VStack(alignment: .leading, spacing: Metrics.s3) {
      HStack(spacing: Metrics.s3) {
        Circle()
          .fill(Palette.popHover)
          .frame(width: Metrics.avatar, height: Metrics.avatar)
          .overlay { Icon(.userRound, size: Metrics.barIcon, strokeWidth: 1.75).foregroundStyle(Palette.fg2) }
        VStack(alignment: .leading, spacing: Metrics.s0_5) {
          HStack(alignment: .firstTextBaseline, spacing: Metrics.s2) {
            Text(web: title, paragraph: false)
              .textStyle(TypeScale.body, weight: .semibold)
              .foregroundStyle(Palette.fgStrong)
              .lineLimit(1)
              .truncationMode(.middle)
            if let plan {
              Text(web: plan, paragraph: false)
                .textStyle(TypeScale.text4, weight: .medium)
                .foregroundStyle(Palette.fg1)
                .padding(.horizontal, Metrics.s2)
                .padding(.vertical, Metrics.s0_5)
                .background(Capsule().fill(Palette.popActive))
                .fixedSize()
            }
          }
          if let caption {
            Text(web: caption, paragraph: true)
              .textStyle(TypeScale.text3)
              .foregroundStyle(Palette.fg2)
              .fixedSize(horizontal: false, vertical: true)
          }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        if checkSlot {
          Image(systemName: "checkmark").font(.body.weight(.semibold)).foregroundStyle(Palette.ok).opacity(checked ? 1 : 0)
        }
      }
      if let quota {
        QuotaBars(quota: quota, style: TypeScale.text2, ink: Palette.fg1)
      } else if let status {
        Text(web: status, paragraph: false).textStyle(TypeScale.text3).foregroundStyle(Palette.fg2)
      }
    }
    .padding(.vertical, Metrics.s1_5)
  }
}

/// A pushed screen's inline title in the system bar, with the bare back chevron
struct SystemTitle: ViewModifier {
  var on: Bool
  var title: String
  func body(content: Content) -> some View {
    if on {
      content.navigationTitle(title).navigationBarTitleDisplayModeInline().backWithoutTitle()
    } else {
      content
    }
  }
}

// MARK: - Settings

/// The settings under system chrome: a sheet over the list (as Trae's phone app brings its settings up), its title
/// centred in the sheet's own bar with close at the bar's end. A pushed page put a large title under back, stacked
/// at the left edge, where the system's own pages centre it beside back
struct SettingsSheet: View {
  @Environment(\.dismiss) private var dismiss
  var body: some View {
    NavigationStack {
      NativeSettingsScreen()
        .toolbar {
          ToolbarItem(placement: .primaryAction) { Button(role: .close) { dismiss() } }
        }
    }
  }
}

/// SettingsScreen as a system `Form` under an inline title: menu pickers, a toggle and read-only values in grouped
/// sections
struct NativeSettingsScreen: View {
  @AppStorage(Prefs.theme) private var theme = "system"
  @AppStorage(Prefs.locale) private var locale = "auto"
  @AppStorage(Prefs.motion) private var motion = true

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
      Section {
        // One fact: the state, with its dot; what it means goes in the footnote
        LabeledContent(t("mobile.engine.status")) {
          HStack(spacing: Metrics.s1_5) {
            Circle().fill(Palette.fg3).frame(width: Metrics.rowDot, height: Metrics.rowDot)
            Text(web: t("connections.state.off"), paragraph: false)
          }
        }
      } header: {
        Text(web: t("mobile.engine"))
      } footer: {
        Text(web: t("mobile.engine.desc"))
      }
      Section(t("mobile.about")) {
        LabeledContent(t("settings.fact.version"), value: version)
      }
    }
    .formPage()
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

extension ToolbarItemPlacement {
  /// The bar's leading edge (`topBarLeading` on iOS, the navigation slot elsewhere)
  static var leadingBar: ToolbarItemPlacement {
    #if os(iOS)
    .topBarLeading
    #else
    .navigation
    #endif
  }
}

extension View {
  func navigationBarTitleDisplayModeLarge() -> some View {
    #if os(iOS)
    navigationBarTitleDisplayMode(.large)
    #else
    self
    #endif
  }

  func navigationBarTitleDisplayModeInline() -> some View {
    #if os(iOS)
    navigationBarTitleDisplayMode(.inline)
    #else
    self
    #endif
  }

  /// A settings form on the form page (in dark the same page as list and chat): the system's grouped structure, rows
  /// on the form fill, sections a step apart. Every pushed form uses it, so a step deeper keeps the same page
  func formPage() -> some View {
    listSectionSpacingCompact()
      .listRowBackground(Palette.formRow)
      .scrollContentBackground(.hidden)
      .background(Palette.formPage)
  }

  /// Sections a step apart instead of the system's tall default gap
  func listSectionSpacingCompact() -> some View {
    #if os(iOS)
    listSectionSpacing(.compact)
    #else
    self
    #endif
  }

  /// The back button as the bare chevron, without the previous screen's title
  func backWithoutTitle() -> some View {
    #if os(iOS)
    toolbarRole(.editor)
    #else
    self
    #endif
  }

  /// A menu whose picks leave it open, so several settings change in one go (iOS only: macOS menus have no such mode)
  func menuStaysOpen() -> some View {
    #if os(iOS)
    menuActionDismissBehavior(.disabled)
    #else
    self
    #endif
  }

  /// The navigation bar: shown under `chrome=system`, the app's own headers otherwise
  func navigationBar(system: Bool) -> some View {
    #if os(iOS)
    toolbar(system ? .automatic : .hidden, for: .navigationBar)
    #else
    self
    #endif
  }
}

// MARK: - Screenshot probe

#if DEBUG && canImport(UIKit)
/// `-popup menu:X,Y` (points): opens the system menu whose button covers that point, for screenshots of an open
/// `Menu` (a launch argument cannot open one, and the simulator takes no scripted taps here). Debug builds only: it
/// calls UIKit's private `_presentMenuAtLocation:`
@MainActor
enum MenuProbe {
  static func open(_ spec: String) async {
    let xy = spec.dropFirst("menu:".count).split(separator: ",").compactMap { Double($0) }
    guard xy.count == 2 else { return }
    try? await Task.sleep(for: .seconds(1.5))
    let point = CGPoint(x: xy[0], y: xy[1])
    guard let window = UIApplication.shared.connectedScenes.compactMap({ ($0 as? UIWindowScene)?.keyWindow }).first else { return }
    var hit: (UIContextMenuInteraction, UIView)?
    func walk(_ view: UIView) {
      for case let interaction as UIContextMenuInteraction in view.interactions
      where view.convert(view.bounds, to: window).contains(point) {
        // The deepest view under the point wins
        hit = (interaction, view)
      }
      view.subviews.forEach(walk)
    }
    walk(window)
    let selector = NSSelectorFromString("_presentMenuAtLocation:")
    guard let (interaction, view) = hit, interaction.responds(to: selector) else {
      print("MenuProbe: no menu at \(point)")
      return
    }
    typealias Present = @convention(c) (AnyObject, Selector, CGPoint) -> Void
    unsafeBitCast(interaction.method(for: selector), to: Present.self)(interaction, selector, window.convert(point, to: view))
  }
}
#endif
