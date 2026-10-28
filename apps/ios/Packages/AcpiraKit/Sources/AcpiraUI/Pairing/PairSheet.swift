import AcpiraDesign
import AcpiraSync
import SwiftUI

// The add-computer sheet. Its form leads with the camera card, then the computers this network shows (picking one asks
// for the code on its card) and a pasted pairing link. A code scanned or a link pasted in the sheet pairs at once; a link
// opened from outside the app asks first. Pairing, connecting, connected and a refusal share one connect frame that
// replaces the form in place, and the sheet goes once the new computer answers. A scanned code is drawn in that frame as
// the computer's card draws it, and the paired moment plays on both screens at once (PairCode.swift).

/// What the sheet shows
enum PairPhase: Equatable {
  /// The form
  case choose
  /// A link from outside the app, waiting for a yes
  case confirm(link: String, name: String)
  /// The link on its way to the computer
  case pairing(link: String, name: String)
  /// Paired: the list follows it now, and the frame waits for its wire
  case joined(PairedComputer)
  /// Refused or unanswered, with the way on
  case failed(name: String?, message: String, retry: PairRetry)
}

enum PairRetry: Equatable {
  /// Back to the form, its camera scanning again
  case scan
  /// The same link once more (the computer did not answer)
  case link(String)
}

struct PairSheet: View {
  @Environment(\.pairing) private var pairing
  @Environment(\.initialPopup) private var initialPopup
  @Environment(\.motionEnabled) private var motion
  @Environment(\.dismiss) private var dismiss
  @State private var phase = PairPhase.choose
  @State private var path: [NearbyComputer] = []
  @State private var nearby: [NearbyComputer] = []
  @State private var browsing = true
  /// The code being paired with and its link: the very modules a scan read, else made from the link
  @State private var code: (link: String, rows: [String])?

  var body: some View {
    NavigationStack(path: $path) {
      ZStack {
        if phase == .choose {
          PairAddForm(nearby: nearby, browsing: browsing, scanning: path.isEmpty, onLink: { pair($0, scanned: $1) })
            .transition(.opacity)
        } else {
          PairStage(phase: phase, code: code?.rows, finish: { dismiss() }, onConnect: connectConfirmed, onRetry: retry)
            .transition(.opacity)
        }
      }
      .animation(motion ? Motion.ease : nil, value: phase)
      .navigationTitle(phase == .choose ? t("mobile.pair.add") : "")
      .navigationBarTitleDisplayModeInline()
      .navigationDestination(for: NearbyComputer.self) { computer in
        PairCodePage(computer: computer, finish: { dismiss() })
      }
      .toolbar { ToolbarItem(placement: .primaryAction) { Button(role: .close) { dismiss() } } }
    }
    .presentationBackground(Palette.formPage)
    // Browsing runs for the sheet's whole life, so coming back from the code page keeps the rows where they were
    .task {
      guard let pairing else { return }
      for await found in pairing.service.nearby() {
        withAnimation(motion ? Motion.ease : nil) { nearby = found }
        // `-popup pair.code…`: the first computer that turns up, picked
        if path.isEmpty, let first = found.first, initialPopup?.hasPrefix("pair.code") == true, !openedPopup {
          openedPopup = true
          path = [first]
        }
      }
      browsing = false
    }
    .task { takePending() }
    .onChange(of: pairing?.pendingLink) { _, _ in takePending() }
    .onDisappear { pairing?.addOpen = false }
  }

  @State private var openedPopup = false

  /// A link opened from outside while the sheet is up, or the one that raised it
  private func takePending() {
    guard let pairing, let link = pairing.pendingLink else { return }
    pairing.pendingLink = nil
    path = []
    do {
      phase = .confirm(link: link, name: try pairing.service.computerName(link: link))
      show(link)
      // `-popup pair.link.go|pair.link.expired`: the yes given at once
      if initialPopup == "pair.link.go" || initialPopup == "pair.link.expired" { connectConfirmed() }
    } catch {
      code = nil
      phase = .failed(name: nil, message: Self.message(error, name: nil), retry: .scan)
    }
  }

  /// The code to draw for a link: the rows a scan read, else the ones already shown for it, else made from its text
  private func show(_ link: String, scanned: [String]? = nil) {
    if let scanned {
      code = (link, scanned)
    } else if code?.link != link {
      code = PairCodeRows.make(link: link).map { (link, $0) }
    }
  }

  private func connectConfirmed() {
    guard case .confirm(let link, _) = phase else { return }
    pair(link)
  }

  private func retry() {
    guard case .failed(_, _, let retry) = phase else { return }
    switch retry {
    case .scan: phase = .choose
    case .link(let link): pair(link)
    }
  }

  /// Pairs through a scanned, pasted or confirmed link; a scan hands over the modules it read
  private func pair(_ link: String, scanned: [String]? = nil) {
    guard let pairing else { return }
    let name: String
    do { name = try pairing.service.computerName(link: link) } catch {
      code = nil
      phase = .failed(name: nil, message: Self.message(error, name: nil), retry: .scan)
      return
    }
    show(link, scanned: scanned)
    phase = .pairing(link: link, name: name)
    Task {
      do {
        let computer = try await pairing.service.pair(link: link)
        guard phase == .pairing(link: link, name: name) else { return }
        phase = .joined(computer)
        await pairing.paired(computer)
      } catch {
        guard phase == .pairing(link: link, name: name) else { return }
        let unanswered = (error as? PairingError) == .unreachable
        phase = .failed(name: name, message: Self.message(error, name: name), retry: unanswered ? .link(link) : .scan)
      }
    }
  }

  /// Why a scanned or opened link did not pair: a stale code asks for the new one the computer's card shows by now
  static func message(_ error: Error, name: String?) -> String {
    let named = ["name": name ?? ""]
    return switch error as? PairingError {
    case .expired?: t("mobile.pair.scan.expired")
    case .used?: t("mobile.pair.scan.used")
    case .closed? where name != nil: t("mobile.pair.closed", named)
    case .unreachable? where name != nil: t("mobile.pair.unreachable", named)
    case .impostor?: t("mobile.pair.impostor")
    case .invalidLink?: t("mobile.pair.invalidLink")
    default: t("mobile.pair.failed")
    }
  }
}

/// The sheet's form: the camera card, the computers nearby, a pasted link
struct PairAddForm: View {
  let nearby: [NearbyComputer]
  let browsing: Bool
  /// The form is on top: the camera runs only then
  let scanning: Bool
  /// A pairing link, with the code's rows when the camera read it
  var onLink: (String, [String]?) -> Void
  @State private var notice: String?
  @State private var noticeGen = 0

  var body: some View {
    Form {
      Section {
        PairCameraCard(active: scanning, onLink: onLink)
          .listRowInsets(EdgeInsets())
      }
      Section {
        if nearby.isEmpty {
          Text(web: t("mobile.pair.nearby.none"), size: TypeScale.text3.size)
            .textStyle(TypeScale.text3)
            .foregroundStyle(Palette.fg3)
        }
        ForEach(nearby) { computer in
          NavigationLink(value: computer) {
            Label {
              Text(verbatim: computer.name)
            } icon: {
              Image(systemName: computerSymbol(computer.model)).foregroundStyle(Palette.fg2)
            }
          }
        }
      } header: {
        HStack(spacing: Metrics.s2) {
          Text(web: t("mobile.pair.nearby"), paragraph: false)
          if browsing { ProgressView().controlSize(.mini) }
        }
      }
      Section {
        LabeledContent {
          // The system's paste control: it reads the clipboard on the tap without the paste prompt, and greys itself
          // out while the clipboard holds no text
          PasteButton(payloadType: String.self) { strings in
            let text = (strings.first ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
            Task { @MainActor in pasted(text) }
          }
          .labelStyle(.titleOnly)
          .buttonBorderShape(.capsule)
          .controlSize(.small)
          // The page's fill, on the row
          .pairPasteInk(fill: Palette.formPage)
        } label: {
          Label {
            Text(web: notice ?? t("mobile.pair.paste"), paragraph: false)
              .foregroundStyle(notice == nil ? Palette.fg1 : Palette.fg3)
              .contentTransition(.opacity)
          } icon: {
            Image(systemName: "link").foregroundStyle(Palette.fg2)
          }
        }
      }
    }
    .formPage()
  }

  private func pasted(_ text: String) {
    if PairingModel.isPairingLink(text) {
      onLink(text, nil)
      return
    }
    // Not a pairing link: the row says so for a moment, in place
    noticeGen += 1
    let gen = noticeGen
    withAnimation(Motion.quick) { notice = t("mobile.pair.paste.notLink") }
    Task {
      try? await Task.sleep(for: .seconds(Motion.pairNotice))
      if gen == noticeGen { withAnimation(Motion.quick) { notice = nil } }
    }
  }
}

/// The connect frame inside the sheet: the glyph (or the code being paired with), the computer's name, one sentence,
/// one action. Paired is the moment the computer answers the pairing, not its wire: the computer's card celebrates on
/// that same answer, so the two screens play it together while the wire comes up
struct PairStage: View {
  let phase: PairPhase
  /// The code's rows, drawn in the glyph's place
  var code: [String]?
  /// Closes the sheet
  var finish: () -> Void
  var onConnect: () -> Void = {}
  var onRetry: () -> Void = {}
  @Environment(\.pairing) private var pairing
  /// When the computer answered the pairing
  @State private var pairedAt: ContinuousClock.Instant?

  var body: some View {
    PairHeroPlacement(box: code == nil ? Metrics.pairGlyphBox : Metrics.pairCode) {
      PairHero(symbol: symbol, ring: ring, code: code, paired: paired, name: name, line: line, path: path, action: action,
               dim: dim, onAction: act)
    }
    .background(Palette.formPage)
    .onChange(of: paired, initial: true) { _, now in pairedAt = now ? .now : nil }
    // Connected: the sheet goes once the paired moment has held as long as the card holds it, so both screens move on
    // together, and a beat after a wire that came up later. A wire that fails right after pairing leaves the sheet
    // too, and the list says what happened
    .task(id: wire) {
      switch wire {
      case .up:
        let held = pairedAt.map { $0.duration(to: .now) } ?? .zero
        try? await Task.sleep(for: max(.seconds(Motion.pairHold) - held, .seconds(Motion.pairDone)))
        if !Task.isCancelled { finish() }
      case .down: finish()
      case .waiting, .none: break
      }
    }
  }

  private var paired: Bool {
    if case .joined = phase { return true }
    return false
  }

  private enum Wire { case waiting, up, down }

  /// The path the newly paired computer's wire travels, once it is up; it follows the wire, so a relay that gives way to
  /// a direct path shows the change
  private var path: PathKind? {
    guard case .joined(let computer) = phase, let pairing, pairing.selected?.id == computer.id,
          case .online(let path) = pairing.link else { return nil }
    return path
  }

  /// The newly paired computer's wire, once the list follows it
  private var wire: Wire? {
    guard case .joined(let computer) = phase, let pairing else { return nil }
    guard pairing.selected?.id == computer.id else { return .waiting }
    return switch pairing.link {
    case .online: .up
    case .failed: .down
    case .idle, .connecting: .waiting
    }
  }

  private var symbol: String {
    if case .joined(let computer) = phase { return computerSymbol(computer.model) }
    // A link names the computer but not its kind
    return computerSymbol(nil)
  }

  private var name: String? {
    switch phase {
    case .choose: nil
    case .confirm(_, let name), .pairing(_, let name): name
    case .joined(let computer): computer.name
    case .failed(let name, _, _): name
    }
  }

  private var ring: PairHero.Ring {
    switch phase {
    case .pairing: .spin
    // The spinner stays with its sentence while the path's tag takes their place, so the line fades out whole
    case .joined: .spin
    default: .rest
    }
  }

  private var line: String {
    switch phase {
    case .choose: ""
    case .confirm: t("mobile.pair.confirm")
    case .pairing: t("mobile.pair.connecting")
    case .joined: t("mobile.pair.connecting")
    case .failed(_, let message, _): message
    }
  }

  private var action: String? {
    switch phase {
    case .confirm: t("mobile.pair.connect")
    case .failed(_, _, .scan): t("mobile.pair.scan.again")
    case .failed(_, _, .link): t("mobile.pair.retry")
    default: nil
    }
  }

  private var dim: Bool {
    if case .failed = phase { return true }
    return false
  }

  private func act() {
    switch phase {
    case .confirm: onConnect()
    case .failed: onRetry()
    default: break
    }
  }
}
