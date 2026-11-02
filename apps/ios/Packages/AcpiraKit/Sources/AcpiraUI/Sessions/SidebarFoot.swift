import AcpiraDesign
import AcpiraSync
import SwiftUI

// The sidebar's foot in a regular width, as the desktop's session column ends: settings, then the followed computer
// with its menu (switch computers, add one). On the phone both sit in the list's top bar instead (ListTopBar).

struct SidebarFoot: View {
  @Environment(AppModel.self) private var model
  @Environment(\.pairing) private var pairing

  var body: some View {
    GlassEffectContainer {
      HStack(spacing: Metrics.gap) {
        Button { model.settingsOpen = true } label: {
          Image(systemName: "gearshape").font(.system(size: Metrics.glassGlyph, weight: .medium))
            .foregroundStyle(Palette.fg1)
            .frame(width: Metrics.glassCtl, height: Metrics.glassCtl)
            .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .glassEffect(GlassStyle.control.interactive(), in: Circle())
        .accessibilityLabel(Text(web: t("session.settings")))
        if let pairing, let current = pairing.selected { computer(pairing, current) }
      }
      .frame(maxWidth: .infinity, alignment: .leading)
    }
    .padding(.horizontal, Metrics.pad)
    .padding(.bottom, Metrics.s0_5)
    .background { FootFade() }
  }

  /// The followed computer: its kind, its name, and a dot lit while its wire is up; the menu says how it is reached
  private func computer(_ pairing: PairingModel, _ current: PairedComputer) -> some View {
    let online = if case .online = pairing.link { true } else { false }
    return Menu { PairComputerMenu(pairing: pairing) } label: {
      HStack(spacing: Metrics.gapHalf) {
        Image(systemName: computerSymbol(current.model)).foregroundStyle(Palette.fg2)
        Text(verbatim: current.name).textStyle(TypeScale.text2).foregroundStyle(Palette.fg1).lineLimit(1)
        Spacer(minLength: Metrics.gapHalf)
        Circle().fill(online ? Palette.ok : Palette.fg3).frame(width: Metrics.railDot * 2, height: Metrics.railDot * 2)
      }
      .padding(.horizontal, Metrics.pad)
      .frame(height: Metrics.glassCtl)
      .contentShape(Capsule())
    }
    .menuStyle(.button)
    .buttonStyle(.plain)
    .glassEffect(GlassStyle.control.interactive(), in: Capsule())
    .accessibilityLabel(Text(verbatim: [current.name, pairing.status(current)].compactMap { $0 }.joined(separator: t("common.metaSep"))))
  }
}

/// The page colour under the foot's glass, fading up into the list, as TopFade does under the top bar
private struct FootFade: View {
  var body: some View {
    LinearGradient(stops: [.init(color: Palette.bg0.opacity(0), location: 0), .init(color: Palette.bg0.opacity(0.5), location: 0.2),
                           .init(color: Palette.bg0, location: 0.5), .init(color: Palette.bg0, location: 1)],
                   startPoint: .top, endPoint: .bottom)
      .padding(.top, -Metrics.topFadeTail)
      .ignoresSafeArea(edges: .bottom)
      .allowsHitTesting(false)
  }
}
