import AcpiraDesign
import AcpiraModels
import SwiftUI
#if canImport(UIKit)
import UIKit
#endif

// The system's own menus, bars and settings form. The "…", project and agent pickers are system `Menu`s; the model's
// choices and attach open liquid beans out of the composer (Composer.swift, Goo.swift).

// MARK: - Marks as menu images

// MARK: - Session "…"

/// The session's "…" menu: the account and the session's changes, pin and rename, copy / move / export in submenus,
/// the project's terminal, settings, delete
struct NativeSessionMenu: View {
  var pinned: Bool
  var actions = HeaderActions()

  var body: some View {
    if actions.account != nil || actions.changes != nil {
      Section {
        if let account = actions.account { Button(action: account) { Label(t("common.account"), systemImage: "person.crop.circle") } }
        if let changes = actions.changes { Button(action: changes) { Label(t("panel.changes"), systemImage: "plusminus") } }
      }
    }
    Section {
      Button(action: actions.pin) { Label(pinned ? t("common.unpin") : t("common.pin"), systemImage: pinned ? "pin.slash" : "pin") }
      Button(action: actions.rename) { Label(t("common.rename"), systemImage: "pencil") }
      Menu {
        Button { actions.copy(.link) } label: { Label(t("session.copy.link"), systemImage: "link") }
        Button { actions.copy(.markdown) } label: { Label(t("session.copy.markdown"), systemImage: "doc.text") }
        if !actions.cwd.isEmpty {
          Button { actions.copy(.cwd) } label: { Label(t("session.copy.cwd"), systemImage: "folder") }
        }
      } label: {
        Label(t("session.copy"), systemImage: "doc.on.doc")
      }
      if let move = actions.move, actions.projects.contains(where: { $0 != actions.cwd }) {
        Menu {
          // The other known folders, the latest first; the session's own is left out
          ForEach(actions.projects.filter { $0 != actions.cwd }, id: \.self) { folder in
            Button { move(folder) } label: {
              Text(verbatim: folderName(folder))
              Text(verbatim: folder)
            }
          }
        } label: {
          Label(t("session.moveToProject"), systemImage: "folder.badge.arrow.forward")
        }
      }
      Menu {
        Button { actions.export(false) } label: { Label(t("session.export.markdown"), systemImage: "doc.text") }
        Button { actions.export(true) } label: { Label(t("session.export.json"), systemImage: "curlybraces") }
      } label: {
        Label(t("session.export"), systemImage: "square.and.arrow.up")
      }
    }
    Section {
      // The composer's subagent entry on the desktop: the session's relation tree, its tallies as the subtitle
      if let subagents = actions.subagents {
        Button(action: subagents) {
          Label {
            Text(web: t("subagents.graph"))
            Text(web: actions.subagentCounts)
          } icon: {
            Image(systemName: "point.3.connected.trianglepath.dotted")
          }
        }
      }
      Button(action: actions.terminal) { Label(t("panel.terminal"), systemImage: "apple.terminal") }
      Button(action: actions.settings) { Label(t("session.settings"), systemImage: "gearshape") }
    }
    Section { Button(role: .destructive, action: actions.delete) { Label(t("common.delete"), systemImage: "trash") } }
  }
}

// MARK: - Project / agent

/// The project menu's rows: the recent folders as a checked picker, then "Add folder" (the caller raises its alert)
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

/// The add-folder alert the project menu raises: a path on the engine's computer
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

/// The accounts as a system sheet: a form under an inline title, the system's close button and "+" in its bar, on
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
  var title: String
  func body(content: Content) -> some View {
    content.navigationTitle(title).navigationBarTitleDisplayModeInline().backWithoutTitle()
  }
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
