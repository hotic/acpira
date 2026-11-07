import AcpiraDesign
import AcpiraModels
import SwiftUI

// chat/Notice.tsx (Notice, ConnectingNotice): what the session itself is doing when it is not ready. Signing in and
// unlocking the credential store happen on the computer the engine runs on (a terminal there, the keychain's
// password), so the phone says so and offers only what the engine can do by itself: a sign-in method that needs no
// terminal, a restart, taking the session over, or a fresh session.

extension EnvironmentValues {
  /// The agent the open chat runs (name for the notices, the account layer and keychain state)
  @Entry var chatAgent: AgentInfo?
}

/// One way out of the state
enum NoticeWay: Hashable {
  case restart
  case takeOver
  case newSession
  /// A sign-in method the engine runs without a terminal: its id and its (localized) name
  case login(id: String, name: String)

  var label: String {
    switch self {
    case .restart: t("common.retry")
    case .takeOver: t("notice.takeOver")
    case .newSession: t("notice.continueNew")
    case .login(_, let name): name
    }
  }
}

/// The card for a session that is not ready: glyph, title and one line, the copyable agent error, the one primary
/// way, the other ways (one plain button, or several behind a menu), and retry as an icon when it is not the primary
struct NoticeState: Equatable {
  var icon: LucideIcon
  var title: String
  var text: String
  /// The agent's own error (English, whatever the locale) next to a localized explanation: behind "Copy details"
  var detail: String?
  var primary: NoticeWay?
  var others: [NoticeWay] = []
  var showRetry = false
}

/// Notice.tsx's state table for the phone; nil while the session is ready or starting
func noticeState(_ session: SessionView, agent: AgentInfo?) -> NoticeState? {
  let status = session.status
  guard status != .ready, status != .starting, status != .unknown else { return nil }
  let name = agent?.name ?? session.agent
  let error = session.error
  let takeOver = (status == .error || status == .readonly) && session.canTakeOver == true
  if status == .auth_required {
    if agent?.accounts == true && agent?.credentialsLocked == true {
      return NoticeState(icon: .keyRound, title: t("notice.locked.title", ["agent": name]),
                         text: t("mobile.notice.unlockOnComputer"), detail: error, showRetry: true)
    }
    // A method with a terminal opens it on the computer; the rest the engine runs itself, then reconnects
    let methods = (session.authMethods ?? []).filter { $0.terminal == nil }.map { method in
      let key = "notice.method.\(session.agent):\(method.id)"
      let known = t(key)
      return NoticeWay.login(id: method.id, name: known == key ? method.name : known)
    }
    return NoticeState(icon: .lockKeyhole, title: t("notice.login.title", ["agent": name]),
                       text: t("mobile.notice.loginOnComputer"), detail: error, primary: methods.first,
                       others: Array(methods.dropFirst()), showRetry: true)
  }
  if status == .readonly || status == .closed {
    let readonly = status == .readonly
    return NoticeState(icon: readonly ? .eye : .circlePause,
                       title: t(readonly ? "notice.readonly.title" : "notice.closed.title"),
                       text: readonly ? error ?? t("notice.readonly.text") : t("notice.closed.text"),
                       primary: takeOver ? .takeOver : .newSession, others: takeOver ? [.newSession] : [])
  }
  if takeOver {
    return NoticeState(icon: .triangleAlert, title: t("notice.takeOver.title"), text: t("notice.takeOver.text"),
                       detail: error, primary: .takeOver, showRetry: true)
  }
  return NoticeState(icon: .triangleAlert, title: t("notice.error.title"), text: error ?? t("notice.error.unknown"),
                     primary: .restart)
}

/// The session card above the composer, in the failure card's phone layout: what is wrong and retry on the first
/// line, the explanation under it, then the actions on their own row, the copyable detail first and the way on last
struct SessionNotice: View {
  let session: SessionView
  @Environment(\.chatAgent) private var agent
  @Environment(\.sessionActions) private var actions
  @State private var copied = false

  var body: some View {
    if let state = noticeState(session, agent: agent) {
      card(state)
    }
  }

  private func card(_ state: NoticeState) -> some View {
    VStack(alignment: .leading, spacing: Metrics.s2) {
      HStack(spacing: Metrics.s2) {
        Icon(state.icon, size: Metrics.icon, strokeWidth: 1.75).foregroundStyle(Palette.popFg3)
        Text(web: state.title)
          .textStyle(TypeScale.text2, weight: .semibold)
          .foregroundStyle(Palette.fgStrong)
          .lineLimit(1)
          .frame(maxWidth: .infinity, alignment: .leading)
        if state.showRetry {
          CardCornerButton(icon: .rotateCw, label: t("common.retry"), action: actions.restart)
            .padding(.vertical, -Metrics.s2)
            .padding(.trailing, -Metrics.s1)
        }
      }
      Text(web: state.text)
        .textStyle(TypeScale.text3)
        .foregroundStyle(Palette.popFg2)
        .fixedSize(horizontal: false, vertical: true)
      if state.detail != nil || state.primary != nil || !state.others.isEmpty {
        CardActions(copy: state.detail == nil ? [] : [copied ? t("alert.copied") : t("alert.copy")]) {
          UIPasteboardBridge.copy(state.detail ?? "")
          copied = true
        } ways: {
          if state.others.count == 1, let other = state.others.first {
            Button(web: other.label) { run(other) }.modifier(CardButton(primary: false))
          } else if state.others.count > 1 {
            PopupMenu {
              ForEach(state.others, id: \.self) { way in Button(web: way.label) { run(way) } }
            } label: {
              Text(web: t("notice.moreWays"))
              Icon(.chevronDown, size: Metrics.chevron, strokeWidth: 1.75).foregroundStyle(Palette.popFg3)
            }
            .modifier(CardButton(primary: false))
            .menuStyle(.button)
            .fixedSize()
          }
          if let primary = state.primary {
            Button(web: primary.label) { run(primary) }.modifier(CardButton(primary: true))
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

  private func run(_ way: NoticeWay) {
    switch way {
    case .restart: actions.restart()
    case .takeOver: actions.takeOver()
    case .newSession: actions.newSession()
    case .login(let id, _): actions.login(id)
    }
  }
}

/// What floats over the thread's lower edge rather than taking a dock slot, so neither shrinks the thread or pushes
/// its history up: the docked to-do card
struct ThreadDock: View {
  let session: SessionView
  // Values rather than the session: a streamed chunk leaves them equal and the card is not rebuilt with it
  var body: some View {
    PlanDock(entries: dockPlan(session.turns, running: session.running), running: session.running,
             goal: session.goal, goalActions: session.goalActions)
  }
}

extension EnvironmentValues {
  /// The stage holding the reply the thread draws for a held prompt (`heldStage`): its working row says it in place
  /// of "Working"
  @Entry var liveStage: String?
}

/// How long the start may last before the pill shows
private let startingDelay = Duration.milliseconds(400)

/// ConnectingNotice at phone width: a glass pill in the navigation bar's empty middle while a session starts with no
/// reply to say it on, once the start has taken a moment, so a warm agent does not flash it. Neither place on the
/// thread held it: a row at the thread's end pushed the history up a line when it showed and dropped it back when the
/// start ended, and a pill floating above the composer covered the last reply's closing line and its actions. The bar
/// has room for "Starting…" alone; the composer's mark already names the agent. A held prompt makes the thread's view
/// running, and that reply's working row speaks instead (`heldStage`), so the words never show in two places.
/// iOS 26 lays the title slot out beside the back button, not centred
struct StartingTitle: ViewModifier {
  /// The session is starting with no reply to say it on (status starting, not running, not a mirror). A flag rather
  /// than the session, so a streamed chunk does not rebuild the bar's items
  let active: Bool
  /// The session's agent, named when the chat's agent is unknown
  let agent: AgentId
  /// A composer picker is out: the pill leaves the bar with its other items (`barAside`)
  var aside = false
  @Environment(\.chatAgent) private var chatAgent
  @Environment(\.snapshot) private var snapshot
  @State private var late = false

  func body(content: Content) -> some View {
    content
      #if os(iOS)
      // Its own glass, the height of the bar's buttons; the shared one would draw a second capsule around it
      .bareToolbarItem(placement: .title, shown: active && (late || snapshot) && !aside) {
        HStack(spacing: Metrics.s2) {
          Orb()
          Text(web: t("devices.starting"), paragraph: false).lineLimit(1)
        }
        .textStyle(TypeScale.text3, weight: .medium)
        .foregroundStyle(Palette.popFg2)
        .padding(.horizontal, Metrics.pad)
        .frame(height: Metrics.glassCtl)
        .glassSurface(.regular, in: Capsule())
        .fixedSize()
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(Text(web: t("notice.connecting", ["agent": chatAgent?.name ?? agent])))
        .accessibilityAddTraits(.updatesFrequently)
      }
      #endif
      .task(id: active) {
        late = false
        guard active else { return }
        try? await Task.sleep(for: startingDelay)
        withAnimation(Motion.ease) { late = true }
      }
  }
}
