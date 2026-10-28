import AcpiraDesign
import AcpiraSync
import SwiftUI

// The settings' side of pairing: the computers section in place of the engine row, one computer's page with removing
// it, and the hooks the settings form needs for both (its pushed page and the add-computer sheet over the settings).

/// The settings' computers: each paired one with how it is reached right now, then add another
struct PairComputersSection: View {
  let pairing: PairingModel

  var body: some View {
    Section {
      ForEach(pairing.computers) { computer in
        NavigationLink(value: computer) {
          LabeledContent {
            // The state as the row's value, as the system's settings show one; nothing until a probe says
            if let status = pairing.status(computer) { Text(verbatim: status) }
          } label: {
            Label {
              Text(verbatim: computer.name).lineLimit(1)
            } icon: {
              // Online: a dot on the glyph's corner, the same place on every row whatever the value's width
              Image(systemName: computerSymbol(computer.model)).foregroundStyle(Palette.fg2)
                .overlay(alignment: .bottomTrailing) {
                  if pairing.online(computer) == true {
                    Circle().fill(Palette.ok).frame(width: Metrics.rowDot, height: Metrics.rowDot)
                      .padding(Metrics.pairDotCut)
                      .background(Circle().fill(Palette.formRow))
                      .offset(x: Metrics.rowDot / 2, y: Metrics.rowDot / 2)
                  }
                }
            }
          }
        }
      }
      Button { pairing.addOpen = true } label: {
        Label {
          Text(web: t("mobile.pair.add"), paragraph: false)
        } icon: {
          Image(systemName: "plus").foregroundStyle(Palette.fg2)
        }
      }
    } header: {
      Text(web: t("mobile.pair.computers"))
    } footer: {
      Text(web: t("mobile.pair.computers.desc"))
    }
  }
}

/// One paired computer: how it is reached and since when, and removing it
struct PairComputerDetail: View {
  let computer: PairedComputer
  let pairing: PairingModel
  @State private var confirming = false
  @Environment(\.dismiss) private var dismiss

  var body: some View {
    Form {
      Section {
        LabeledContent(t("mobile.pair.connection")) {
          if let status = pairing.status(computer) {
            Text(verbatim: status)
          } else {
            ProgressView().controlSize(.small)
          }
        }
        LabeledContent(t("mobile.pair.pairedAt"), value: pairedAt)
      }
      Section {
        Button(role: .destructive) { confirming = true } label: { Text(web: t("mobile.pair.remove"), paragraph: false) }
      } footer: {
        Text(web: t("mobile.pair.remove.desc"))
      }
    }
    .formPage()
    .navigationTitle(computer.name)
    .navigationBarTitleDisplayModeInline()
    .confirmationDialog(t("mobile.pair.remove.confirm", ["name": computer.name]), isPresented: $confirming,
                        titleVisibility: .visible) {
      Button(t("common.remove"), role: .destructive) {
        Task {
          await pairing.remove(computer.id)
          dismiss()
        }
      }
      Button(t("mobile.cancel"), role: .cancel) {}
    }
  }

  private var pairedAt: String {
    computer.pairedAt.formatted(Date.FormatStyle(date: .abbreviated, time: .omitted, locale: Locale(identifier: L10n.locale)))
  }
}

/// On the settings form: a computer's page when its row is tapped, the add-computer sheet over the settings (the root
/// cannot raise a second sheet while the settings are up), fresh states each time the page shows, and
/// `-popup pair.computer` (the followed computer's page) for screenshots
struct PairSettingsHooks: ViewModifier {
  let pairing: PairingModel
  @State private var shown: PairedComputer?
  @Environment(\.initialPopup) private var initialPopup

  func body(content: Content) -> some View {
    content
      .navigationDestination(for: PairedComputer.self) { PairComputerDetail(computer: $0, pairing: pairing) }
      .navigationDestination(item: $shown) { PairComputerDetail(computer: $0, pairing: pairing) }
      .sheet(isPresented: Bindable(pairing).addOpen) { PairSheet() }
      .task {
        while !pairing.loaded { try? await Task.sleep(for: .milliseconds(50)) }
        if initialPopup == "pair.computer" { shown = pairing.selected }
        await pairing.refreshPresence()
      }
  }
}

extension View {
  /// The settings form's pairing hooks, when the app has a pairing service
  @ViewBuilder
  func pairSettingsHooks(_ pairing: PairingModel?) -> some View {
    if let pairing { modifier(PairSettingsHooks(pairing: pairing)) } else { self }
  }
}
