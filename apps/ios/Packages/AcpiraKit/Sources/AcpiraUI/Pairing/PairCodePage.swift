import AcpiraDesign
import AcpiraSync
import SwiftUI

// The six digits a nearby computer's pairing card shows, entered after picking it in the sheet. One hidden field sits
// behind six boxes, so typing advances on its own, the one-time-code bar and a paste fill every box, and the sixth
// digit sends the code. A refused code clears the boxes and says why in a line that keeps its slot. Once paired, the
// page becomes the connect frame in place and the sheet goes when the computer answers.

struct PairCodePage: View {
  let computer: NearbyComputer
  /// Closes the whole sheet (this page's own dismiss would only pop it)
  var finish: () -> Void
  @Environment(\.pairing) private var pairing
  @Environment(\.initialPopup) private var initialPopup
  @Environment(\.motionEnabled) private var motion
  @State private var code = ""
  @State private var checking = false
  @State private var error: String?
  @State private var joined: PairedComputer?
  @FocusState private var focused: Bool

  static let length = 6

  var body: some View {
    ZStack {
      if let joined {
        PairStage(phase: .joined(joined), finish: finish).transition(.opacity)
      } else {
        entry.transition(.opacity)
      }
    }
    .animation(motion ? Motion.ease : nil, value: joined)
    .background(Palette.formPage)
    .navigationTitle(joined == nil ? t("mobile.pair.code.title") : "")
    .navigationSubtitle(joined == nil ? computer.name : "")
    .navigationBarTitleDisplayModeInline()
    .navigationBarBackButtonHidden(joined != nil)
    .toolbar { ToolbarItem(placement: .primaryAction) { Button(role: .close, action: finish) } }
    .task { await start() }
  }

  private var entry: some View {
    VStack(spacing: 0) {
      Text(web: t("mobile.pair.code.hint"), size: TypeScale.body.size)
        .textStyle(TypeScale.body)
        .foregroundStyle(Palette.fg2)
        .multilineTextAlignment(.center)
        .padding(.bottom, Metrics.page)
      ZStack {
        field
        boxes.allowsHitTesting(false)
      }
      .contentShape(Rectangle())
      .onTapGesture { focused = true }
      // Checking, or why the last code was refused; the slot stays, so nothing under it moves
      ZStack {
        if checking {
          ProgressView().controlSize(.small)
        } else if let error {
          Text(web: error, size: TypeScale.text3.size)
            .textStyle(TypeScale.text3)
            .foregroundStyle(Palette.danger)
            .multilineTextAlignment(.center)
        }
      }
      .frame(maxWidth: .infinity, minHeight: Metrics.pairStatusSlot, alignment: .top)
      .padding(.top, Metrics.gap)
      // The system's paste control reads the clipboard without asking (a plain button would raise the paste prompt)
      PasteButton(payloadType: String.self) { strings in
        let digits = String((strings.first ?? "").filter(\.isNumber).prefix(Self.length))
        Task { @MainActor in if !digits.isEmpty, !checking { code = digits } }
      }
      .labelStyle(.titleAndIcon)
      .buttonBorderShape(.capsule)
      // The code boxes' fill, on the page
      .pairPasteInk(fill: Palette.formRow)
      .disabled(checking)
      .padding(.top, Metrics.s2)
      Spacer(minLength: 0)
    }
    .padding(.top, Metrics.page * 2)
    .padding(.horizontal, Metrics.pad)
    .frame(maxWidth: .infinity)
  }

  private var field: some View {
    TextField("", text: $code)
      #if os(iOS)
      .keyboardType(.numberPad)
      .textContentType(.oneTimeCode)
      #endif
      .focused($focused)
      .foregroundStyle(.clear)
      .tint(.clear)
      // Kept nearly invisible rather than hidden: a hidden field takes no focus
      .opacity(Palette.pairHiddenField)
      .accessibilityLabel(Text(web: t("mobile.pair.code.title")))
      .onChange(of: code) { old, typed in
        // The field keeps focus while a code is checked, so the keyboard stays put; what is typed meanwhile is dropped
        if checking { if typed != old { code = old }; return }
        let digits = String(typed.filter(\.isNumber).prefix(Self.length))
        if digits != typed { code = digits; return }
        if !digits.isEmpty { error = nil }
        if digits.count == Self.length { submit(digits) }
      }
  }

  private var boxes: some View {
    HStack(spacing: Metrics.s2) {
      ForEach(0..<Self.length, id: \.self) { i in
        box(i).padding(.leading, i == Self.length / 2 ? Metrics.pairCodeSplit - Metrics.s2 : 0)
      }
    }
    .accessibilityHidden(true)
  }

  private func box(_ i: Int) -> some View {
    let digits = Array(code)
    // The box the next digit goes in, lit only while the field has focus
    let next = focused && !checking && i == digits.count
    return ZStack {
      RoundedRectangle(cornerRadius: Metrics.rLg, style: .continuous).fill(Palette.formRow)
      RoundedRectangle(cornerRadius: Metrics.rLg, style: .continuous)
        .strokeBorder(next ? Palette.fg1 : Palette.line, lineWidth: Metrics.pairCodeLine)
      if i < digits.count {
        Text(verbatim: String(digits[i]))
          .font(TypeScale.pairDigit.font(.semibold)).monospacedDigit()
          .foregroundStyle(checking ? Palette.fg2 : Palette.fgStrong)
      } else if next {
        PairCaret()
      }
    }
    .frame(width: Metrics.pairCodeBox, height: Metrics.pairCodeBoxH)
  }

  private func submit(_ digits: String) {
    guard !checking, let pairing else { return }
    checking = true
    error = nil
    Task {
      do {
        let paired = try await pairing.service.pair(nearby: computer, code: digits)
        checking = false
        focused = false
        joined = paired
        await pairing.paired(paired)
      } catch {
        checking = false
        self.error = Self.message(error, name: computer.name)
        code = ""
      }
    }
  }

  /// Why a code was refused, in the page's own words: a code that expired or was used asks for the new one the card
  /// shows by now
  static func message(_ error: Error, name: String) -> String {
    switch error as? PairingError {
    case .wrong?: t("mobile.pair.code.wrong")
    case .expired?: t("mobile.pair.code.expired")
    case .used?: t("mobile.pair.code.used")
    case .closed?: t("mobile.pair.closed", ["name": name])
    case .unreachable?: t("mobile.pair.unreachable", ["name": name])
    case .impostor?: t("mobile.pair.impostor")
    case .invalidLink?, .other?, nil: t("mobile.pair.failed")
    }
  }

  /// Focus once the push settles (focusing mid-push drags the keyboard in with the page), then the launch's demo
  private func start() async {
    try? await Task.sleep(for: .milliseconds(150))
    focused = true
    // `-popup pair.code.type|wrong|expired|demo`: a few digits typed, a refused code, or a whole entry played
    switch initialPopup {
    case "pair.code.type": code = "384"
    case "pair.code.wrong": code = "384971"
    case "pair.code.expired": code = "000000"
    case "pair.code.demo":
      try? await Task.sleep(for: .seconds(1))
      for wrong in ["3", "8", "4", "9", "7", "1"] { code += wrong; try? await Task.sleep(for: .milliseconds(260)) }
      while checking { try? await Task.sleep(for: .milliseconds(100)) }
      try? await Task.sleep(for: .seconds(1.4))
      for digit in ["3", "8", "4", "9", "1", "7"] { code += digit; try? await Task.sleep(for: .milliseconds(260)) }
    default: break
    }
  }
}

extension View {
  /// The system paste control as a quiet capsule: `fill` a step off the surface it sits on, body ink. The control keeps
  /// its own label, which the system localises
  func pairPasteInk(fill: Color) -> some View {
    tint(fill).foregroundStyle(Palette.fg1)
  }
}

/// The blinking caret in the box the next digit goes in; it blinks with motion off too, as a text field's does
struct PairCaret: View {
  var body: some View {
    TimelineView(.periodic(from: .now, by: Motion.pairBlink)) { context in
      let on = Int(context.date.timeIntervalSinceReferenceDate / Motion.pairBlink).isMultiple(of: 2)
      Rectangle().fill(Palette.fg1)
        .frame(width: Metrics.pairCaret, height: Metrics.pairCaretH)
        .opacity(on ? 1 : 0)
    }
  }
}
