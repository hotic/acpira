import AcpiraDesign
import AcpiraModels
import SwiftUI

// ContextUsage.tsx: the context window ring beside send, and its card. The card holds only what the agent reported
// (the figure, the window and the auto-compact threshold marked on the bar), says when that threshold applies, and
// offers compaction when the agent advertises `/compact`. What the desktop's tooltips say is the card's last line here.

/// The ring: an icon-sized arc on a faint track, in a control slot as tall as the bar
struct ContextRing: View {
  let usage: Usage
  var canCompact: Bool
  var compactAt: Int?
  var running: Bool
  var onCompact: () -> Void
  @State private var open = false
  @Environment(\.initialPopup) private var initialPopup
  @Environment(\.motionEnabled) private var motion

  var body: some View {
    let fill = UsageBreakdown.fill(usage)
    let stale = usage.compactedFrom != nil
    let pct = Int((usage.size > 0 ? min(1, Double(usage.used) / Double(usage.size)) : 0) * 100)
    let stroke = Metrics.icon / 8
    Button { open.toggle() } label: {
      ZStack {
        Circle().stroke(lineWidth: stroke).opacity(0.25)
        Circle().trim(from: 0, to: fill).stroke(style: StrokeStyle(lineWidth: stroke, lineCap: .round))
          .rotationEffect(.degrees(-90))
          .opacity(stale ? 0.5 : 1)
          .animation(motion ? Motion.ease : nil, value: fill)
      }
      // r = 6 in a 16-unit box, as the webview's svg
      .padding(Metrics.icon / 8)
      .frame(width: Metrics.icon, height: Metrics.icon)
    }
    .buttonStyle(IconButtonStyle(side: Metrics.ctlSm, open: open))
    .frame(height: Metrics.barCtl)
    .accessibilityLabel(Text(web: stale ? t("usage.compactedStale") : t("usage.usedPct", ["pct": pct])))
    .popover(isPresented: $open) {
      UsageCard(usage: usage, pct: pct, fill: fill, compactAt: compactAt, canCompact: canCompact, running: running) {
        onCompact()
        open = false
      }
      .presentationCompactAdaptation(.popover)
    }
    .task {
      // `-popup context`: the card open on launch (screenshots)
      guard initialPopup == "context" else { return }
      if motion { try? await Task.sleep(for: .seconds(1)) }
      open = true
    }
  }
}

/// UsagePanel: the title, the share used and the figures, the bar with the threshold's tick and number in its tone,
/// what that tone means, and compaction (held while a turn runs)
private struct UsageCard: View {
  let usage: Usage
  let pct: Int
  let fill: Double
  let compactAt: Int?
  let canCompact: Bool
  let running: Bool
  var onCompact: () -> Void
  @Environment(\.agentAccent) private var accent

  var body: some View {
    let stale = usage.compactedFrom != nil
    let panel = UsageBreakdown.panel(usage, compactAt: compactAt, canCompact: canCompact, running: running)
    VStack(alignment: .leading, spacing: Metrics.s2) {
      Text(web: t("usage.title"), paragraph: false)
        .textStyle(TypeScale.text2, weight: .medium)
        .foregroundStyle(Palette.fg1)
      HStack(alignment: .firstTextBaseline, spacing: Metrics.s2) {
        Text(web: stale ? t("usage.compactedStale") : t("usage.usedPctShort", ["pct": pct]), paragraph: false)
          .foregroundStyle(Palette.fg2)
        Spacer(minLength: 0)
        Text(web: stale ? t("usage.beforeCompact", ["n": number(usage.compactedFrom ?? 0)])
             : "\(number(usage.used)) / \(number(usage.size))", paragraph: false)
          .foregroundStyle(Palette.fg3)
      }
      .textStyle(TypeScale.text3)
      .monospacedDigit()
      HStack(spacing: Metrics.s2) {
        GeometryReader { box in
          ZStack(alignment: .leading) {
            Capsule().fill(Palette.active)
            Capsule().fill(Palette.fg2).opacity(stale ? 0.5 : 1).frame(width: box.size.width * fill)
            if let compactAt, usage.size > 0 {
              Rectangle().fill(tick(panel.tone))
                .frame(width: Metrics.s0_5)
                .offset(x: box.size.width * min(Double(compactAt) / Double(usage.size), 0.99))
            }
          }
          .clipShape(Capsule())
        }
        .frame(height: Metrics.s1_5)
        if let compactAt {
          Text(web: number(compactAt), paragraph: false)
            .textStyle(TypeScale.text3)
            .monospacedDigit()
            .foregroundStyle(ink(panel.tone))
            .opacity(panel.tone == .unsupported ? 0.6 : 1)
            .accessibilityLabel(Text(web: t("usage.budget", ["n": number(compactAt)])))
        }
      }
      if let note = stale ? t("usage.compactedStaleHint") : panel.note {
        Text(web: note)
          .textStyle(TypeScale.text3)
          .foregroundStyle(Palette.fg3)
          .fixedSize(horizontal: false, vertical: true)
      }
      if canCompact {
        Button(action: onCompact) {
          Icon(.shrink, size: Metrics.icon, strokeWidth: 1.75)
          Text(web: t("usage.compact"), paragraph: false)
        }
        .buttonStyle(AcpButtonStyle(.secondary, compact: true, fixed: true))
        .disabled(running)
        .opacity(running ? 0.4 : 1)
        .padding(.top, Metrics.s1)
      }
    }
    .padding(Metrics.s3)
    .frame(width: Metrics.popLg, alignment: .leading)
  }

  /// Numbers in the app's language, as the webview's Intl.NumberFormat(locale)
  private func number(_ n: Int) -> String { n.formatted(.number.locale(Locale(identifier: L10n.locale))) }

  private func tick(_ tone: UsageBreakdown.Panel.Tone) -> Color {
    switch tone {
    case .ok: Palette.fg1
    case .pending: accent
    case .limited: Palette.warn
    case .unsupported: Palette.fg3
    }
  }

  private func ink(_ tone: UsageBreakdown.Panel.Tone) -> Color {
    switch tone {
    case .ok, .unsupported: Palette.fg3
    case .pending: accent
    case .limited: Palette.warn
    }
  }
}
