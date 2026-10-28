import AcpiraDesign
import AcpiraModels
import SwiftUI

/// Alert.tsx: a turn that stopped short. What happened and ✕ on the first line, the agent's words under it, then the
/// actions on their own row: the copyable detail as a quiet button at the start, the way on as the one filled button
/// at the end, where a thumb reaches. The way on is retry for an error (plus "continue in a new session" when the
/// failure names it), carry on for a limit, nothing for a refusal; a context that grew too long offers compaction
/// instead of a retry that would fail the same way
struct AlertCard: View {
  let turn: AgentTurn
  var onDismiss: () -> Void
  var onRetry: () -> Void = {}
  var onContinue: () -> Void = {}
  /// Only when the agent advertises `/compact`
  var onCompact: (() -> Void)?
  /// An AIR failure's `new_session` remedy
  var onNewSession: (() -> Void)?
  @State private var copied = false

  var body: some View {
    let stop = turn.stop ?? .error
    let err = turn.error
    let contextTooLong = stop == .error && isContextLengthError(err)
    let detail = [err?.code.map(String.init) ?? "", err?.kind ?? ""].filter { !$0.isEmpty }.joined(separator: t("common.metaSep"))
    let copyable = [err?.message ?? "", detail].filter { !$0.isEmpty }.joined(separator: "\n")
    let message: String? = if contextTooLong {
      t(onCompact != nil ? "alert.contextLength.text" : "alert.contextLength.unsupported")
    } else {
      switch stop {
      case .error: (err?.message).flatMap { $0.isEmpty ? nil : $0 } ?? t("alert.error.unknown")
      case .refusal: t("alert.refusal.text")
      case .max_tokens: t("alert.maxTokens.text")
      case .max_turn_requests: t("alert.maxTurns.text")
      default: nil
      }
    }
    let title = if contextTooLong {
      t("alert.contextLength.title")
    } else {
      switch stop {
      case .refusal: t("alert.refusal.title")
      case .max_tokens: t("alert.maxTokens.title")
      case .max_turn_requests: t("alert.maxTurns.title")
      default: t("alert.error.title")
      }
    }
    phone(stop: stop, contextTooLong: contextTooLong, title: title, message: message, copyable: copyable, detail: detail)
  }

  private func phone(stop: TurnStop, contextTooLong: Bool, title: String, message: String?, copyable: String,
                     detail: String) -> some View {
    let actions = turn.error?.actions
    let retry = stop == .error && !contextTooLong && (actions == nil || actions?.contains(.retry) == true)
    let fresh = stop == .error && !contextTooLong && actions?.contains(.new_session) == true ? onNewSession : nil
    let carry = stop == .max_tokens || stop == .max_turn_requests
    let compact = contextTooLong ? onCompact : nil
    return VStack(alignment: .leading, spacing: Metrics.s2) {
      HStack(spacing: Metrics.s2) {
        Icon(.triangleAlert, size: Metrics.icon, strokeWidth: 1.75).foregroundStyle(Palette.warn)
        Text(web: title)
          .textStyle(TypeScale.text2, weight: .semibold)
          .foregroundStyle(Palette.fgStrong)
          .frame(maxWidth: .infinity, alignment: .leading)
        IconButton(.x, label: t("common.close"), action: onDismiss)
          .padding(.vertical, -Metrics.s1)
          .padding(.trailing, -Metrics.s1_5)
      }
      if let message {
        Text(web: message)
          .textStyle(TypeScale.text3)
          .foregroundStyle(Palette.fg2)
          .fixedSize(horizontal: false, vertical: true)
      }
      // The code and kind ride on "Copy details" (Cursor's "Copy Request (id)") while they fit
      if !copyable.isEmpty || retry || fresh != nil || carry || compact != nil {
        CardActions(copy: copyable.isEmpty ? [] : [copied ? t("alert.copied") : detail.isEmpty ? t("alert.copy") : t("alert.copyWith", ["detail": detail]),
                                                   copied ? t("alert.copied") : t("alert.copy")]) {
          UIPasteboardBridge.copy(copyable)
          copied = true
        } ways: {
          if let fresh {
            Button(web: t("notice.continueNew"), action: fresh).buttonStyle(AcpButtonStyle(.secondary, fixed: true))
          }
          if retry {
            Button(web: t("common.retry"), action: onRetry).buttonStyle(AcpButtonStyle(.primary, fixed: true))
          }
          if carry {
            Button(web: t("alert.continue"), action: onContinue).buttonStyle(AcpButtonStyle(.primary, fixed: true))
          }
          if let compact {
            Button(web: t("alert.contextLength.compact"), action: compact).buttonStyle(AcpButtonStyle(.primary, fixed: true))
          }
        }
        .padding(.top, Metrics.s1)
      }
    }
    .padding(.horizontal, Metrics.pad)
    .padding(.vertical, Metrics.s3)
    .modifier(CardSurface())
    .task(id: copied) {
      guard copied else { return }
      try? await Task.sleep(for: .milliseconds(1500))
      copied = false
    }
  }
}

/// A dock card's action row: the quiet copy button at the start, the ways on at the end where a thumb reaches. The
/// copy label falls back to its shorter forms before the row breaks (`copy`: longest first, empty for no copy
/// button); when no form fits one row (long labels, larger text) the ways drop under the copy button, still at the end
struct CardActions<Ways: View>: View {
  var copy: [String]
  var onCopy: () -> Void
  @ViewBuilder var ways: Ways

  init(copy: [String], onCopy: @escaping () -> Void, @ViewBuilder ways: () -> Ways) {
    self.copy = copy; self.onCopy = onCopy; self.ways = ways()
  }

  var body: some View {
    if let shortest = copy.last {
      // The copy label on the ways' row, longest first, then the ways on a row of their own. One ForEach with unique ids
      // and a plain last candidate: two ForEach children (their offsets repeating) tripped SwiftUI's SizeFittingState
      // precondition, a crash at launch
      let candidates = copy.map { (label: $0, stacked: false) } + copy.dropLast().map { (label: $0, stacked: true) }
      ViewThatFits(in: .horizontal) {
        ForEach(Array(candidates.enumerated()), id: \.offset) { _, c in
          if c.stacked { stacked(c.label) } else { row(c.label) }
        }
        stacked(shortest)
      }
    } else {
      HStack(spacing: Metrics.s2) {
        Spacer(minLength: 0)
        ways
      }
    }
  }

  private func row(_ label: String) -> some View {
    HStack(spacing: Metrics.s2) {
      copyButton(label)
      Spacer(minLength: 0)
      ways
    }
  }

  private func stacked(_ label: String) -> some View {
    VStack(alignment: .trailing, spacing: Metrics.s2) {
      HStack(spacing: 0) {
        copyButton(label)
        Spacer(minLength: 0)
      }
      HStack(spacing: Metrics.s2) { ways }
    }
  }

  private func copyButton(_ label: String) -> some View {
    Button(action: onCopy) { Text(web: label) }
      .buttonStyle(AcpButtonStyle(.secondary, fixed: true))
  }
}
