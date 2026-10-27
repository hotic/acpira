import AcpiraDesign
import AcpiraModels
import SwiftUI

// chat/Header.tsx's menus at phone width, each opening below its icon button and aligned to its end: the plus menu
// (new session with an agent), the account panel (chat/AccountPanel.tsx) and the session "…" menu (chat/SessionMenu.tsx).

extension EnvironmentValues {
  /// The time quota resets count down from: the real clock, or the fixtures' clock while the app shows fixtures
  @Entry var now: Date = .now
  /// Seconds the app's clock runs ahead of the real one: the fixtures' clock runs on from launch (`AppModel.now`), so a
  /// live counter reads the time the demo turns are stamped with; 0 on the real clock
  @Entry var clockOffset: TimeInterval = 0
}

/// agentOrder.ts launchable: the agents a new session can start with, the disabled ones left out
func launchable(_ agents: [AgentInfo]) -> [AgentInfo] { agents.filter { $0.disabled != true } }

/// True when the agent has something for the account menu: stored logins (Devin) or the CLI's own account (Grok, Kimi)
func hasAccountMenu(_ agent: AgentInfo) -> Bool { agent.external != true && (agent.accounts == true || agent.localAccount != nil) }

// MARK: - Plus

/// The plus menu: one row per enabled agent; an agent that is not installed stays listed, dimmed
struct NewSessionMenu: View {
  let agents: [AgentInfo]
  var onNew: (AgentId) -> Void
  @Environment(\.dismissPopup) private var dismiss

  var body: some View {
    PopupList {
      ForEach(launchable(agents)) { a in
        MenuOption(a.name, checked: false, checkSlot: false) {
          onNew(a.id)
          dismiss()
        } mark: {
          VendorMark(brand: a.id, name: a.name)
        }
        .disabled(a.available == false)
        .accessibilityHint(Text(web: a.available == false ? t("agent.notInstalled") : ""))
      }
    }
  }
}

// MARK: - Session "…"

/// SessionMenu.tsx from the header, with what the phone's header no longer shows as icons: the account, then pin and
/// rename, the exports, settings, and delete in the danger colour; groups split by a hairline. "Open in editor tab"
/// has no counterpart on the phone
struct SessionActionsMenu: View {
  var pinned: Bool
  var actions = HeaderActions()
  @Environment(\.dismissPopup) private var dismiss

  var body: some View {
    VStack(spacing: 0) {
      if let account = actions.account {
        item(.userRound, t("common.account"), account)
        MenuSeparator()
      }
      item(pinned ? .pinOff : .pin, pinned ? t("common.unpin") : t("common.pin"), actions.pin)
      item(.pencil, t("common.rename"), actions.rename)
      MenuSeparator()
      item(.fileText, t("session.export.markdown")) { actions.export(false) }
      item(.fileJson, t("session.export.json")) { actions.export(true) }
      MenuSeparator()
      item(.settings2, t("session.settings"), actions.settings)
      item(.trash2, t("common.delete"), actions.delete).environment(\.menuInk, Palette.danger)
    }
  }

  private func item(_ icon: LucideIcon, _ label: String, _ action: @escaping () -> Void) -> some View {
    MenuOption(label, checked: false, checkSlot: false) {
      // The popup closes first, so a sheet or alert the action raises is not stacked on it
      dismiss()
      action()
    } mark: {
      Icon(icon, size: Metrics.icon)
    }
  }
}

/// DropdownMenu.Separator: my-1 h-px bg-line
struct MenuSeparator: View {
  var body: some View {
    Rectangle().fill(Palette.menuLine).frame(height: Metrics.hairline).padding(.vertical, Metrics.s1)
  }
}

// MARK: - Account

/// AccountPanel.tsx: stored logins as a bar (agent name, ＋ to add one) over one row per account with its quota bars,
/// the current one raised and checked; an agent's own CLI account as one read-only identity with its quota
struct AccountPanel: View {
  let agent: AgentInfo
  let accounts: [AccountInfo]
  var accountId: String?
  var onSelect: (String) -> Void = { _ in }
  var onAdd: () -> Void = {}
  var onRemove: (String) -> Void = { _ in }
  @Environment(\.dismissPopup) private var dismiss

  var body: some View {
    VStack(alignment: .leading, spacing: 0) {
      if let local = agent.localAccount {
        PanelHeader(title: agent.name)
        LocalAccountBody(agent: agent, account: local)
      } else {
        PanelHeader(title: agent.name, action: .plus, actionLabel: t("composer.addAccount")) { onAdd(); dismiss() }
        if agent.credentialsLocked == true {
          Text(web: t("notice.locked.short"))
            .textStyle(TypeScale.text3)
            .foregroundStyle(Palette.popFg2)
            .padding(.horizontal, Metrics.s2)
            .padding(.vertical, Metrics.s1_5)
        }
        PopupList(current: accountId) {
          if accounts.isEmpty {
            AccountIdentity(title: t("settings.agent.accounts.none"), caption: t("composer.noAccounts"), wrapCaption: true)
              .padding(.horizontal, Metrics.s2)
              .padding(.vertical, Metrics.s1_5)
          }
          ForEach(accounts) { a in
            AccountRow(account: a, checked: a.id == accountId, checkSlot: accounts.contains { $0.id == accountId }) {
              onSelect(a.id)
              dismiss()
            }
            .contextMenu {
              Button(role: .destructive) { onRemove(a.id) } label: { Text(web: t("common.removeNamed", ["name": a.label])) }
            }
            .id(a.id)
          }
        }
      }
    }
  }
}

/// One stored login: the identity row (avatar, name, plan, check) and its quota bars below. The current row takes the
/// option-selected fill; the remove button is a long press on the phone (the webview shows it on hover)
private struct AccountRow: View {
  let account: AccountInfo
  var checked: Bool
  var checkSlot: Bool
  var action: () -> Void

  var body: some View {
    Button(action: action) {
      VStack(alignment: .leading, spacing: 0) {
        HStack(spacing: Metrics.s2) {
          AccountIdentity(title: account.label, caption: account.detail)
          if checked {
            Icon(.check, size: Metrics.icon, strokeWidth: 2).foregroundStyle(Palette.fg1)
          } else if checkSlot {
            Color.clear.frame(width: Metrics.icon, height: Metrics.icon)
          }
        }
        // pr-6: room for the remove button the webview overlays here
        .padding(.trailing, Metrics.s6)
        if let quota = account.quota { QuotaBars(quota: quota) }
      }
      .padding(.horizontal, Metrics.s2)
      .padding(.vertical, Metrics.s1_5)
      .background(RoundedRectangle(cornerRadius: Metrics.rMd).fill(checked ? Palette.popActive : .clear))
    }
    .buttonStyle(MenuOptionStyle())
    .accessibilityAddTraits(checked ? .isSelected : [])
  }
}

/// The CLI's own account: its identity, the quota bars and a footnote on what they count
struct LocalAccountBody: View {
  let agent: AgentInfo
  let account: LocalAccountInfo

  var body: some View {
    // Before a login is found the label is only the product name the bar already shows
    let known = account.label != agent.name || account.detail != nil
    VStack(alignment: .leading, spacing: Metrics.gap) {
      if known {
        AccountIdentity(title: account.label, detail: account.detail, caption: t("quota.officialAccount"))
        if let quota = account.quota {
          QuotaBars(quota: quota)
        } else {
          Text(web: t("quota.status.\(account.status)")).textStyle(TypeScale.text3).foregroundStyle(Palette.popFg2).padding(.top, Metrics.s1)
        }
      } else {
        AccountIdentity(title: account.status == "login_required" ? t("composer.notLoggedIn") : t("quota.officialAccount"),
                        caption: t("quota.status.\(account.status)"), wrapCaption: true)
      }
      if account.quota != nil {
        Text(web: t("quota.local.desc"))
          .textStyle(TypeScale.text3)
          .foregroundStyle(Palette.popFg3)
          .fixedSize(horizontal: false, vertical: true)
          .padding(.top, Metrics.s1_5)
          .overlay(alignment: .top) { Rectangle().fill(Palette.menuLine).frame(height: Metrics.hairline) }
      }
    }
    .padding(.horizontal, Metrics.s2)
    .padding(.vertical, Metrics.s1_5)
  }
}

/// AccountLabel.tsx AccountIdentity: a round avatar, the name line (with the plan beside it), a caption under it
struct AccountIdentity: View {
  var title: String
  var detail: String?
  var caption: String?
  var wrapCaption = false

  var body: some View {
    HStack(spacing: Metrics.gap) {
      Circle()
        .fill(Palette.popHover)
        .frame(width: Metrics.ctl, height: Metrics.ctl)
        .overlay { Icon(.userRound, size: Metrics.icon, strokeWidth: 1.75).foregroundStyle(Palette.popFg3) }
      VStack(alignment: .leading, spacing: 0) {
        HStack(alignment: .firstTextBaseline, spacing: Metrics.s2) {
          Text(web: title, paragraph: false).lineLimit(1).truncationMode(.tail)
          if let detail {
            Text(web: detail, paragraph: false).textStyle(TypeScale.text3).foregroundStyle(Palette.popFg2).fixedSize()
          }
        }
        if let caption {
          Text(web: caption, paragraph: wrapCaption)
            .textStyle(TypeScale.text3)
            .foregroundStyle(Palette.popFg3)
            .lineLimit(wrapCaption ? nil : 1)
            .truncationMode(.tail)
            .fixedSize(horizontal: false, vertical: wrapCaption)
        }
      }
      .frame(maxWidth: .infinity, alignment: .leading)
    }
  }
}

/// Panel.tsx PanelHeader: a --ctl bar over the options with a hairline under it, the title on the left and one
/// square action at the right
struct PanelHeader: View {
  var title: String
  var action: LucideIcon?
  var actionLabel = ""
  var onAction: () -> Void = {}

  var body: some View {
    HStack(spacing: Metrics.s1) {
      Text(web: title, paragraph: false)
        .lineLimit(1)
        .padding(.horizontal, Metrics.s2)
        .frame(height: Metrics.ctl)
      Spacer(minLength: 0)
      if let action {
        Button(action: onAction) { Icon(action, size: Metrics.icon, strokeWidth: 1.75) }
          .buttonStyle(IconButtonStyle(side: Metrics.ctl, ink: Palette.popFg3))
          .accessibilityLabel(Text(web: actionLabel))
      }
    }
    .padding(.bottom, Metrics.s1 + Metrics.hairline)
    .overlay(alignment: .bottom) { Rectangle().fill(Palette.menuLine).frame(height: Metrics.hairline) }
    .padding(.bottom, Metrics.s1)
  }
}

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
