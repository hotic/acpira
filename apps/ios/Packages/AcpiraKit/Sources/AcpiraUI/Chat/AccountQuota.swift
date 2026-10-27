import AcpiraDesign
import AcpiraModels
import SwiftUI

// The account sheet's pieces: whether an agent has accounts to show, and the quota windows (chat/AccountPanel.tsx,
// QuotaBars.tsx).

/// True when the agent has something for the account menu: stored logins (Devin) or the CLI's own account (Grok, Kimi)
func hasAccountMenu(_ agent: AgentInfo) -> Bool { agent.external != true && (agent.accounts == true || agent.localAccount != nil) }

// MARK: - Quota

/// QuotaBars.tsx: per window a line (name, the time to its reset as a badge, what is left) over a 7 pt tube whose
/// fill turns warn under 30 % and danger under 10 %
struct QuotaBars: View {
  let quota: AccountQuota
  /// The text's step and ink: the popup's caption by default, a step up in the system account sheet
  var style = TypeScale.text3
  var ink = Palette.popFg2
  @Environment(\.now) private var now

  var body: some View {
    VStack(alignment: .leading, spacing: Metrics.gap) {
      ForEach(quota.windows, id: \.id) { w in QuotaBar(window: w, now: now) }
      if let balance = quota.onDemandBalanceUsd, balance.isFinite {
        HStack(alignment: .firstTextBaseline, spacing: Metrics.s2) {
          Text(web: t("quota.onDemandBalance"), paragraph: false).lineLimit(1)
          Spacer(minLength: 0)
          Text(verbatim: balance.formatted(.currency(code: "USD")))
        }
      }
    }
    .textStyle(style)
    .foregroundStyle(ink)
    .monospacedDigit()
    .padding(.top, Metrics.s2)
  }
}

private struct QuotaBar: View {
  let window: QuotaWindow
  let now: Date

  var body: some View {
    let pct = Int((window.remaining * 100).rounded())
    let resets = window.resetsAt.flatMap(QuotaTime.parse)
    VStack(alignment: .leading, spacing: Metrics.s1) {
      HStack(spacing: Metrics.s2) {
        Text(web: QuotaTime.windowLabel(window.id), paragraph: false).fixedSize()
        Text(web: resets.map { $0 > now ? QuotaTime.until($0, now: now, compact: true) : t("quota.resetPending") }
               ?? t(window.remaining >= 1 ? "quota.resetIdle" : "quota.resetUnknown"), paragraph: false)
          .lineLimit(1)
          .padding(.horizontal, Metrics.s1)
          .background(RoundedRectangle(cornerRadius: Metrics.rSm).fill(Palette.popActive))
          .layoutPriority(-1)
        Spacer(minLength: 0)
        Text(web: t("quota.left", ["pct": pct]), paragraph: false).fixedSize()
      }
      Capsule()
        .fill(Palette.popActive)
        .frame(height: Metrics.quotaTrack)
        .overlay(alignment: .leading) {
          GeometryReader { proxy in
            Capsule()
              .fill(window.remaining <= 0.1 ? Palette.quotaDanger : window.remaining <= 0.3 ? Palette.quotaWarn : Palette.quotaOk)
              .frame(width: proxy.size.width * CGFloat(pct) / 100)
          }
        }
        .clipShape(Capsule())
    }
    .accessibilityElement(children: .ignore)
    .accessibilityLabel(Text(web: t("quota.aria", ["window": QuotaTime.windowLabel(window.id), "pct": pct])))
  }
}

/// The reset countdown's wording (QuotaBars.tsx fmtUntil): up to two units, compact units run together in CJK
enum QuotaTime {
  static func parse(_ iso: String) -> Date? { Date(iso: iso) }

  /// Known window ids translated, anything else shown as sent
  static func windowLabel(_ id: String) -> String {
    let key = "quota.window.\(id)"
    let text = t(key)
    return text == key ? id : text
  }

  static func until(_ date: Date, now: Date, compact: Bool = false) -> String {
    let minutes = Int((max(0, date.timeIntervalSince(now)) / 60).rounded(.up))
    func unit(_ name: String, _ n: Int) -> String { t(compact ? "quota.time.short.\(name)" : "quota.time.\(name)", ["n": n]) }
    let separator = compact && ["zh-CN", "zh-TW", "ja"].contains(L10n.locale) ? "" : " "
    if minutes < 60 { return unit("minutes", minutes) }
    let hours = minutes / 60
    if hours < 24 { return [unit("hours", hours), minutes % 60 > 0 ? unit("minutes", minutes % 60) : nil].compactMap { $0 }.joined(separator: separator) }
    return [unit("days", hours / 24), hours % 24 > 0 ? unit("hours", hours % 24) : nil].compactMap { $0 }.joined(separator: separator)
  }
}
