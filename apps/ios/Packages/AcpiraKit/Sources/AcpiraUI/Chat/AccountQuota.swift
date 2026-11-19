import AcpiraDesign
import AcpiraModels
import SwiftUI

// The account sheet's pieces: whether an agent has accounts to show, and the quota windows (chat/AccountPanel.tsx,
// QuotaBars.tsx).

/// True when the agent has something for the account menu: stored logins (Devin) or the CLI's own account (Grok, Kimi)
func hasAccountMenu(_ agent: AgentInfo) -> Bool { agent.accounts == true || agent.localAccount != nil }

// MARK: - Quota

/// What is wrong with a stored account's quota read, shown in place of its plan/source tag (AccountPanel.tsx savedState)
struct SavedAccountIssue: Equatable, Sendable {
  let text: String
  let warn: Bool
}

/// The issue tag for a saved account, if the keychain is locked or its last quota read failed
func savedAccountIssue(agent: AgentInfo, account: AccountInfo) -> SavedAccountIssue? {
  // A plugin's account signs in with what the plugin gives, never through the locked store
  if agent.credentialsLocked == true && account.plugin == nil {
    return SavedAccountIssue(text: t("quota.short.locked"), warn: true)
  }
  guard let issue = account.quotaIssue, issue != .unknown else { return nil }
  return SavedAccountIssue(text: t("quota.short.\(issue.rawValue)"), warn: issue == .expired)
}

/// QuotaBars.tsx: per window a metadata line over a slim tube whose fill turns warn under 30 % and danger under 10 %
/// (grey while stale). Windows that come in pairs under one label (e.g. a model group's 5-hour and weekly allowance)
/// share one aligned row at every width: the label once, then the two windows side by side
struct QuotaBars: View {
  let quota: AccountQuota
  /// The last read failed, so these are the last known figures: grey tubes and dimmed text
  var stale = false
  /// Matches desktop's `text-3`, the same step as the account row's plan and source tags
  var style = TypeScale.text4
  @Environment(\.now) private var now
  @State private var wide = false

  var body: some View {
    let pairs = QuotaTime.labelPairs(quota.windows)
    let balance = quota.onDemandBalanceUsd.flatMap { $0.isFinite ? $0 : nil }
    if !quota.windows.isEmpty || balance != nil {
      VStack(alignment: .leading, spacing: Metrics.s2) {
        if let pairs {
          Grid(alignment: .leading, horizontalSpacing: Metrics.gap, verticalSpacing: Metrics.s2) {
            ForEach(pairs, id: \.first.id) { pair in
              GridRow {
                Text(web: QuotaTime.windowLabel(pair.first.id), paragraph: false)
                  .foregroundStyle(stale ? Palette.fg3 : Palette.fg1)
                  .lineLimit(1)
                  .fixedSize()
                  .gridCellAnchor(.leading)
                QuotaBar(window: pair.first, now: now, stale: stale, bare: true, wide: wide)
                QuotaBar(window: pair.second, now: now, stale: stale, bare: true, wide: wide)
              }
            }
          }
        } else if wide && quota.windows.count > 1 {
          let rows = stride(from: 0, to: quota.windows.count, by: 2).map { i in
            (quota.windows[i], i + 1 < quota.windows.count ? quota.windows[i + 1] : nil)
          }
          Grid(alignment: .leading, horizontalSpacing: Metrics.gap, verticalSpacing: Metrics.s2) {
            ForEach(rows, id: \.0.id) { first, second in
              GridRow {
                QuotaBar(window: first, now: now, stale: stale, bare: false, wide: wide)
                if let second {
                  QuotaBar(window: second, now: now, stale: stale, bare: false, wide: wide)
                } else {
                  Color.clear.frame(height: 0)
                }
              }
            }
          }
        } else {
          VStack(alignment: .leading, spacing: Metrics.s2) {
            ForEach(quota.windows, id: \.id) { w in
              QuotaBar(window: w, now: now, stale: stale, bare: false, wide: wide)
            }
          }
        }
        if let balance {
          HStack(alignment: .firstTextBaseline, spacing: Metrics.s2) {
            Text(web: t("quota.onDemandBalance"), paragraph: false).lineLimit(1)
            Spacer(minLength: 0)
            Text(verbatim: balance.formatted(.currency(code: "USD")))
          }
        }
      }
      .textStyle(style)
      .foregroundStyle(stale ? Palette.fg3 : Palette.fg2)
      .monospacedDigit()
      .frame(maxWidth: .infinity, alignment: .leading)
      .onGeometryChange(for: Bool.self) { $0.size.width >= Metrics.quotaWide } action: { wide = $0 }
    }
  }
}

private struct QuotaBar: View {
  let window: QuotaWindow
  let now: Date
  var stale = false
  var bare = false
  var wide = false

  var body: some View {
    let pct = min(100, max(0, Int((window.remaining * 100).rounded())))
    let label = QuotaTime.windowLabel(window.id)
    let reset = QuotaTime.resetText(window, now: now, bare: bare, wide: wide)
    VStack(alignment: .leading, spacing: Metrics.s1) {
      HStack(spacing: bare && !wide ? Metrics.s1 : Metrics.s2) {
        if !bare {
          Text(web: label, paragraph: false)
            .foregroundStyle(stale ? Palette.fg3 : Palette.fg1)
            .lineLimit(1)
            .fixedSize()
        }
        Text(web: reset, paragraph: false)
          .foregroundStyle(Palette.quotaReset)
          .lineLimit(1)
          .truncationMode(.tail)
          .layoutPriority(-1)
        Spacer(minLength: 0)
        Text(verbatim: "\(pct)%").fixedSize()
      }
      Capsule()
        .fill(Palette.popActive)
        .frame(height: Metrics.quotaTrack)
        .overlay(alignment: .leading) {
          GeometryReader { proxy in
            Capsule()
              .fill(stale ? Palette.quotaStale : window.remaining <= 0.1 ? Palette.quotaDanger : window.remaining <= 0.3 ? Palette.quotaWarn : Palette.quotaOk)
              .frame(width: proxy.size.width * CGFloat(pct) / 100)
          }
        }
        .clipShape(Capsule())
    }
    .frame(maxWidth: .infinity, alignment: .leading)
    .accessibilityElement(children: .ignore)
    .accessibilityLabel(Text(web: QuotaTime.accessibilityLabel(window, now: now)))
  }
}

/// Two windows sharing one label (e.g. a model group's 5-hour and weekly allowance)
struct QuotaPair: Equatable, Sendable {
  let first: QuotaWindow
  let second: QuotaWindow
}

/// The reset countdown's wording and window pairing (QuotaBars.tsx): up to two units, compact units run together in CJK
enum QuotaTime {
  /// Shown in a bare narrow cell when a window has no upcoming reset ("Not started" does not fit half a narrow row)
  static let noReset = "—"

  static func parse(_ iso: String) -> Date? { Date(iso: iso) }

  /// Known window ids translated, anything else shown as sent
  static func windowLabel(_ id: String) -> String {
    let key = "quota.window.\(id)"
    let text = t(key)
    return text == key ? id : text
  }

  /// The windows two by two when every such pair shares its label; nil for any other list (QuotaBars.tsx labelPairs)
  static func labelPairs(_ windows: [QuotaWindow]) -> [QuotaPair]? {
    guard !windows.isEmpty, windows.count.isMultiple(of: 2) else { return nil }
    var pairs: [QuotaPair] = []
    pairs.reserveCapacity(windows.count / 2)
    for i in stride(from: 0, to: windows.count, by: 2) {
      let a = windows[i], b = windows[i + 1]
      guard windowLabel(a.id) == windowLabel(b.id) else { return nil }
      pairs.append(QuotaPair(first: a, second: b))
    }
    return pairs
  }

  static func until(_ date: Date, now: Date, compact: Bool = false, largestOnly: Bool = false) -> String {
    let minutes = Int((max(0, date.timeIntervalSince(now)) / 60).rounded(.up))
    func unit(_ name: String, _ n: Int) -> String { t(compact ? "quota.time.short.\(name)" : "quota.time.\(name)", ["n": n]) }
    let separator = compact && ["zh-CN", "zh-TW", "ja"].contains(L10n.locale) ? "" : " "
    if minutes < 60 { return unit("minutes", minutes) }
    let hours = minutes / 60
    if hours < 24 {
      return [unit("hours", hours), !largestOnly && minutes % 60 > 0 ? unit("minutes", minutes % 60) : nil]
        .compactMap { $0 }.joined(separator: separator)
    }
    return [unit("days", hours / 24), !largestOnly && hours % 24 > 0 ? unit("hours", hours % 24) : nil]
      .compactMap { $0 }.joined(separator: separator)
  }

  static func resetLabel(_ window: QuotaWindow, now: Date) -> String {
    guard let resets = window.resetsAt.flatMap(parse) else {
      return t(window.remaining >= 1 ? "quota.resetIdle" : "quota.resetUnknown")
    }
    if resets <= now { return t("quota.resetPending") }
    return t("quota.reset", ["time": until(resets, now: now)])
  }

  /// Metadata line reset text: a bare narrow cell keeps only the largest unit or an em-dash
  static func resetText(_ window: QuotaWindow, now: Date, bare: Bool, wide: Bool) -> String {
    let resets = window.resetsAt.flatMap(parse)
    let ahead = resets.map { $0 > now } ?? false
    if bare && !wide {
      return ahead ? until(resets!, now: now, compact: true, largestOnly: true) : noReset
    }
    return ahead ? until(resets!, now: now, compact: true) : resetLabel(window, now: now)
  }

  static func accessibilityLabel(_ window: QuotaWindow, now: Date) -> String {
    let pct = min(100, max(0, Int((window.remaining * 100).rounded())))
    return "\(t("quota.aria", ["window": windowLabel(window.id), "pct": pct]))\(t("common.metaSep"))\(resetLabel(window, now: now))"
  }
}
