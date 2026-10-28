import AcpiraDesign
import AcpiraSync
import SwiftUI

// The session list's side of pairing: the title row naming the followed computer (its menu switches computers or adds
// one), the page that stands in for the list while there is nothing to list (no computer, connecting, offline, removed),
// and the root layer that raises the add-computer sheet, also for an `acpira://pair/…` link opened from outside.

/// The list's title row with a paired computer: its name opens the computers menu, settings at the row's end
struct PairTitleRow: View {
  let pairing: PairingModel
  let current: PairedComputer
  var onSettings: () -> Void

  var body: some View {
    HStack(spacing: Metrics.s2) {
      Menu {
        PairComputerMenu(pairing: pairing)
      } label: {
        HStack(spacing: Metrics.s1_5) {
          Text(web: current.name, size: TypeScale.title.size, paragraph: false).lineLimit(1).truncationMode(.tail)
            .textStyle(TypeScale.title, weight: .bold)
            .foregroundStyle(Palette.fgStrong)
          Icon(.chevronDown, size: Metrics.icon, strokeWidth: 2.5).foregroundStyle(Palette.fg3)
        }
        .contentShape(Rectangle())
      }
      .menuStyle(.button)
      .buttonStyle(.plain)
      .accessibilityAddTraits(.isHeader)
      Spacer(minLength: 0)
      GlassIconButton(icon: .settings, label: t("session.settings"), action: onSettings)
    }
    .padding(.horizontal, Metrics.pad)
    .frame(maxWidth: .infinity, minHeight: Metrics.largeTitle, alignment: .leading)
  }
}

/// The title's menu: every paired computer with how it is reached (ticked: the one followed), then add another
struct PairComputerMenu: View {
  let pairing: PairingModel

  var body: some View {
    Picker(selection: Binding(get: { pairing.selected?.id ?? "" }, set: { pairing.select($0) })) {
      ForEach(pairing.computers) { computer in
        // A menu row's title, subtitle and image, as NativeProjectMenu builds its rows
        Button {} label: {
          Text(verbatim: computer.name)
          if let status = pairing.status(computer) { Text(verbatim: status) }
          Image(systemName: computerSymbol(computer.model))
        }
        .tag(computer.id)
      }
    } label: { EmptyView() }
    .pickerStyle(.inline)
    Section {
      Button { pairing.addOpen = true } label: { Label(t("mobile.pair.add"), systemImage: "plus") }
    }
  }
}

/// The page in the list's place while it has nothing to list: no computer yet, the followed one connecting, offline,
/// removed this phone, or refused the wire. One connect frame shows every case, so a retry that fails again or a
/// computer coming back moves nothing but the sentence
struct PairHeroList: View {
  let pairing: PairingModel
  @Environment(AppModel.self) private var model
  @Environment(\.motionEnabled) private var motion

  /// Whether the list gives way to this page
  static func covers(_ pairing: PairingModel) -> Bool {
    guard pairing.loaded, pairing.selected != nil else { return true }
    if case .online = pairing.link { return false }
    return true
  }

  var body: some View {
    ZStack(alignment: .top) {
      if pairing.loaded {
        PairHeroPlacement { hero }
      }
      title
    }
    .animation(motion ? Motion.ease : nil, value: pairing.link)
    .background(Palette.bg0)
    .navigationTitle("")
    .hiddenNavigationBar()
  }

  @ViewBuilder
  private var title: some View {
    if let current = pairing.selected {
      PairTitleRow(pairing: pairing, current: current) { model.settingsOpen = true }
    } else {
      // Nothing paired: no title to give the page, settings alone at the row's end
      LargeTitle(title: "") { model.settingsOpen = true }
    }
  }

  /// One frame for every case, so only its contents change between them
  private var hero: some View {
    let state = heroState
    return PairHero(symbol: state.symbol, ring: state.spin ? .spin : .rest, line: state.line, action: state.action,
                    dim: state.dim, onAction: state.act)
  }

  private var heroState: (symbol: String, spin: Bool, line: String, action: String?, dim: Bool, act: () -> Void) {
    guard let current = pairing.selected else {
      return ("laptopcomputer.and.iphone", false, t("mobile.pair.none"), t("mobile.pair.add"), false, { pairing.addOpen = true })
    }
    let named = ["name": current.name]
    let symbol = computerSymbol(current.model)
    let retry = { pairing.connectSelected() }
    return switch pairing.link {
    case .failed(.offline): (symbol, false, t("mobile.pair.offline", named), t("mobile.pair.retry"), true, retry)
    case .failed(.revoked): (symbol, false, t("mobile.pair.revoked", named), t("mobile.pair.again"), true, { pairing.addOpen = true })
    case .failed(.incompatible): (symbol, false, t("mobile.pair.incompatible", named), t("mobile.pair.retry"), true, retry)
    case .failed(.other): (symbol, false, t("mobile.pair.connectFailed", named), t("mobile.pair.retry"), true, retry)
    case .idle, .connecting, .online: (symbol, true, t("mobile.pair.connecting"), nil, false, {})
    }
  }
}

/// On the root: loads the paired computers, raises the add-computer sheet, and takes `acpira://pair/…` links opened from
/// outside the app (the camera app, AirDrop, a tapped link), which the sheet confirms before pairing
struct PairingLayer: ViewModifier {
  @Environment(\.pairing) private var pairing
  @Environment(AppModel.self) private var model
  @Environment(\.initialPopup) private var initialPopup
  @Environment(\.scenePhase) private var scenePhase

  func body(content: Content) -> some View {
    if let pairing {
      content
        // While the settings are up they raise the sheet themselves (PairSettingsHooks)
        .sheet(isPresented: Binding(get: { pairing.addOpen && !model.settingsOpen }, set: { pairing.addOpen = $0 })) {
          PairSheet()
        }
        .onOpenURL { url in pairing.open(url) }
        .task {
          await pairing.load()
          await popup(pairing)
        }
        .onChange(of: scenePhase) { _, phase in
          if phase == .active { Task { await pairing.refreshPresence() } }
        }
    } else {
      content
    }
  }

  /// `-popup pair.add|pair.code…|pair.link…`: the sheet, a code page, or a link as if opened from outside.
  /// `-pairing.link URL` (debug builds) puts a real gateway's link there, so a simulator pairs without a tap
  private func popup(_ pairing: PairingModel) async {
    guard let popup = initialPopup, popup.hasPrefix("pair.") else { return }
    if popup.hasPrefix("pair.link") {
      var link = popup == "pair.link.expired" ? PreviewPairingService.expiredLink : PreviewPairingService.link
      #if DEBUG
      let args = ProcessInfo.processInfo.arguments
      if let at = args.firstIndex(of: "-pairing.link"), at + 1 < args.count { link = args[at + 1] }
      #endif
      if let url = URL(string: link) { pairing.open(url) }
    } else if popup == "pair.add" || popup.hasPrefix("pair.code") {
      pairing.addOpen = true
    }
  }
}

/// The session list, or the connect frame in its place while pairing says there is nothing to list yet; the two cross
/// fade. Without a pairing service the list shows as it always has
struct PairListGate<List: View>: View {
  let pairing: PairingModel?
  @ViewBuilder var list: () -> List
  @Environment(\.motionEnabled) private var motion

  var body: some View {
    if let pairing {
      let covered = PairHeroList.covers(pairing)
      ZStack {
        if covered {
          PairHeroList(pairing: pairing).transition(.opacity)
        } else {
          list().transition(.opacity)
        }
      }
      .animation(motion ? Motion.ease : nil, value: covered)
    } else {
      list()
    }
  }
}
