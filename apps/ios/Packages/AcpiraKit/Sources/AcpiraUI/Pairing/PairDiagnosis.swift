import AcpiraDesign
import AcpiraModels
import AcpiraSync
import SwiftUI

// Where a failed connection stopped, behind the failure page's "Details": the versions on both sides, the relay, the
// connection's own words, and the whole of it as text to copy and send.

struct PairDiagnosisSheet: View {
  let computer: PairedComputer
  let diagnosis: ConnectionDiagnosis
  @State private var copied = false
  @Environment(\.dismiss) private var dismiss

  var body: some View {
    NavigationStack {
      Form {
        Section {
          LabeledContent(t("mobile.pair.details.app"), value: Self.appVersion)
          LabeledContent(t("connections.remoteVersion"), value: diagnosis.computerVersion ?? computer.version ?? "—")
          if let relay = diagnosis.relayConnected {
            LabeledContent(t("mobile.pair.path.relay"), value: t(relay ? "mobile.pair.connected" : "mobile.pair.state.offline"))
          }
        }
        Section {
          // The connection's own words, as they go into the report
          Text(verbatim: diagnosis.detail)
            .textStyle(TypeScale.mono)
            .foregroundStyle(Palette.fg2)
            .textSelection(.enabled)
        }
        Section {
          Button {
            UIPasteboardBridge.copy(Self.report(computer, diagnosis))
            copied = true
          } label: {
            Text(web: t(copied ? "code.copied" : "common.copy"), paragraph: false)
          }
        }
      }
      .formPage()
      .navigationTitle(t("mobile.pair.details"))
      .navigationBarTitleDisplayModeInline()
      .toolbar {
        ToolbarItem(placement: .primaryAction) { CloseButton { dismiss() } }
      }
    }
    .presentationDetents([.medium, .large])
  }

  private static var appVersion: String {
    let info = Bundle.main.infoDictionary
    return "\(info?["CFBundleShortVersionString"] as? String ?? "0") (\(info?["CFBundleVersion"] as? String ?? "0"))"
  }

  /// The report as plain English text, the same in every language: it is read by whoever is asked for help
  static func report(_ computer: PairedComputer, _ d: ConnectionDiagnosis, app: String = appVersion) -> String {
    var stopped = "Stopped: \(d.stop.rawValue)"
    if let ms = d.tookMs { stopped += String(format: " after %.1f s", Double(ms) / 1000) }
    if let relay = d.relayConnected { stopped += relay ? ", relay connected" : ", relay not connected" }
    var theirs = "Computer: Acpira \(d.computerVersion ?? computer.version.map { "\($0) (last seen)" } ?? "unknown")"
    if let p = d.computerProtocol { theirs += ", protocol \(p)" }
    return [
      "Acpira app \(app), protocol \(SidecarProtocol.version)",
      theirs,
      stopped,
      "Error: \(d.detail)",
      "At: \(d.at.formatted(.iso8601))",
    ].joined(separator: "\n")
  }
}
