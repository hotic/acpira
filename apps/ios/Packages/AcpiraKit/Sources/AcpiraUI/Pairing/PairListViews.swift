import AcpiraDesign
import AcpiraSync
import SwiftUI

// The session list's side of pairing: the bar title's menu of computers (it switches computers or adds one), the page
// that stands in for the list while there is nothing to list (no computer, connecting, offline, removed), and the root
// layer that raises the add-computer sheet, also for an `acpira://pair/…` link opened from outside.

/// The bar's centred title: the followed computer with its menu, the plain page name without pairing, nothing while
/// no computer is paired
struct ListBarTitle: View {
  @Environment(\.pairing) private var pairing

  var body: some View {
    if let pairing {
      if let current = pairing.selected {
        Menu { PairComputerMenu(pairing: pairing) } label: { label(current.name, menu: true) }
          .menuStyle(.button)
          .buttonStyle(.plain)
      }
    } else {
      label(t("mobile.home"), menu: false)
    }
  }

  private func label(_ title: String, menu: Bool) -> some View {
    HStack(spacing: Metrics.s1) {
      Text(web: title, size: TypeScale.barTitle.size, paragraph: false).lineLimit(1).truncationMode(.tail)
        .textStyle(TypeScale.barTitle, weight: .bold)
        .foregroundStyle(Palette.fgStrong)
      if menu { Icon(.chevronDown, size: Metrics.icon, strokeWidth: 2.5).foregroundStyle(Palette.fg3) }
    }
    .contentShape(Rectangle())
    .accessibilityAddTraits(.isHeader)
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

  /// Whether the list gives way to this page: until the followed computer's lists are in (the wire is up well before
  /// its init, and the list in between read as empty), and whenever its wire failed. While the computer the list shows
  /// connects again, its last lists stay
  static func covers(_ pairing: PairingModel, _ model: AppModel) -> Bool {
    guard pairing.loaded, let current = pairing.selected else { return true }
    if case .failed = pairing.link { return true }
    return !(model.computer == current.id && (model.synced || model.resumed))
  }

  var body: some View {
    ZStack(alignment: .top) {
      if pairing.loaded {
        PairHeroPlacement { hero }
      }
    }
    .frame(maxWidth: .infinity, maxHeight: .infinity)
    .animation(motion ? Motion.ease : nil, value: pairing.link)
    .background(Palette.bg0)
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
    // Online too until the engine's lists are in; a background close dials again on the way back
    case .idle, .connecting, .online, .failed(.suspended): (symbol, true, t("mobile.pair.connecting"), nil, false, {})
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
  @Environment(AppModel.self) private var model
  @Environment(\.motionEnabled) private var motion

  var body: some View {
    if let pairing {
      let covered = PairHeroList.covers(pairing, model)
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
