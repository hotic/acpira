import AcpiraDesign
import AcpiraModels
import SwiftUI

// chat/Notice.tsx (Notice, ConnectingNotice) and chat/ExternalSessionNotice.tsx: what the session itself is doing when
// it is not ready. Signing in and unlocking the credential store happen on the computer the engine runs on (a
// terminal there, the keychain's password), so the phone says so and offers only what the engine can do by itself:
// a sign-in method that needs no terminal, a restart, taking the session over, or a fresh session.

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
        Icon(state.icon, size: Metrics.icon, strokeWidth: 1.75).foregroundStyle(Palette.fg3)
        Text(web: state.title)
          .textStyle(TypeScale.text2, weight: .semibold)
          .foregroundStyle(Palette.fgStrong)
          .lineLimit(1)
          .frame(maxWidth: .infinity, alignment: .leading)
        if state.showRetry {
          IconButton(.rotateCw, label: t("common.retry"), action: actions.restart)
            .padding(.vertical, -Metrics.s1)
            .padding(.trailing, -Metrics.s1_5)
        }
      }
      Text(web: state.text)
        .textStyle(TypeScale.text3)
        .foregroundStyle(Palette.fg2)
        .fixedSize(horizontal: false, vertical: true)
      if state.detail != nil || state.primary != nil || !state.others.isEmpty {
        CardActions(copy: state.detail == nil ? [] : [copied ? t("alert.copied") : t("alert.copy")]) {
          UIPasteboardBridge.copy(state.detail ?? "")
          copied = true
        } ways: {
          if state.others.count == 1, let other = state.others.first {
            Button(web: other.label) { run(other) }.buttonStyle(AcpButtonStyle(.secondary, fixed: true))
          } else if state.others.count > 1 {
            PopupMenu {
              ForEach(state.others, id: \.self) { way in Button(web: way.label) { run(way) } }
            } label: {
              Text(web: t("notice.moreWays"))
              Icon(.chevronDown, size: Metrics.chevron, strokeWidth: 1.75).foregroundStyle(Palette.fg3)
            }
            .buttonStyle(AcpButtonStyle(.secondary, fixed: true))
            .menuStyle(.button)
            .fixedSize()
          }
          if let primary = state.primary {
            Button(web: primary.label) { run(primary) }.buttonStyle(AcpButtonStyle(.primary, fixed: true))
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

/// A conversation driven from ChatGPT through the bridge: a receiver, not a disabled agent connection, so no sign-in,
/// model, send or stop. The connection instructions are copied to paste into ChatGPT
struct ExternalSessionNotice: View {
  let info: ExternalSessionInfo
  @State private var copied = false

  var body: some View {
    VStack(alignment: .leading, spacing: Metrics.gap) {
      HStack(alignment: .firstTextBaseline, spacing: Metrics.gap) {
        Text(web: "ChatGPT \(t("chatgpt.mirror"))")
          .textStyle(TypeScale.text2, weight: .semibold)
          .foregroundStyle(Palette.fgStrong)
          .frame(maxWidth: .infinity, alignment: .leading)
        Text(web: t("chatgpt.\(info.state.rawValue)"))
          .textStyle(TypeScale.text3)
          .foregroundStyle(Palette.fg2)
          .multilineTextAlignment(.trailing)
      }
      Text(web: t("chatgpt.boundary"))
        .textStyle(TypeScale.text3)
        .foregroundStyle(Palette.fg2)
        .fixedSize(horizontal: false, vertical: true)
      if let prompt = info.connectionPrompt {
        HStack(spacing: Metrics.gap) {
          Text(web: info.state == .unbound ? t("chatgpt.noEvent") : "\(t("chatgpt.lastEvent")) \(eventTime(info.lastEventAt))")
            .textStyle(TypeScale.text3)
            .foregroundStyle(Palette.fg3)
            .frame(maxWidth: .infinity, alignment: .leading)
          Button(web: t(copied ? "chatgpt.copyDone" : "chatgpt.copy")) {
            UIPasteboardBridge.copy(prompt)
            copied = true
          }
          .buttonStyle(AcpButtonStyle(.secondary, fixed: true))
        }
      }
    }
    .padding(Metrics.pad)
    .modifier(CardSurface())
    .accessibilityAddTraits(.updatesFrequently)
    .onChange(of: info.sourceKey) { copied = false }
  }

  /// The event's local time of day (toLocaleTimeString)
  private func eventTime(_ iso: String) -> String {
    let parsed = (try? Date(iso, strategy: .iso8601.year().month().day().time(includingFractionalSeconds: true)))
      ?? (try? Date(iso, strategy: .iso8601))
    return parsed?.formatted(date: .omitted, time: .standard) ?? iso
  }
}

/// What floats over the thread's lower edge rather than taking a dock slot, so neither shrinks the thread or pushes
/// its history up: the connecting row while a session starts, then the docked to-do card
struct ThreadDock: View {
  let session: SessionView
  var body: some View {
    VStack(spacing: 0) {
      if session.external == nil { ConnectingNotice(session: session) }
      PlanDock(session: session)
    }
  }
}

/// How long a session may be starting before the connecting row shows
private let connectingDelay = Duration.milliseconds(400)

/// ConnectingNotice: "Starting Codex…" once a start has taken a moment, so a warm agent does not flash it; only the
/// glyph loops. A zero-height anchor above the docked to-do card: the row rises from it at the composer's text inset
/// and takes no layout space
struct ConnectingNotice: View {
  let session: SessionView
  @Environment(\.chatAgent) private var agent
  @Environment(\.snapshot) private var snapshot
  @State private var late = false

  var body: some View {
    let starting = session.status == .starting
    Color.clear
      .frame(height: 0)
      .overlay(alignment: .bottom) {
        if starting && (late || snapshot) {
          Row {
            Orb()
          } content: {
            Text(web: t("notice.connecting", ["agent": agent?.name ?? session.agent])).lineLimit(1)
          }
          .foregroundStyle(Palette.fg2)
          // px-dock, then the row's own px-pad
          .padding(.horizontal, Metrics.pad * 2)
          .padding(.bottom, Metrics.gapHalf)
          .fixedSize(horizontal: false, vertical: true)
          .transition(.opacity)
          .accessibilityElement(children: .combine)
          .accessibilityAddTraits(.updatesFrequently)
        }
      }
      .allowsHitTesting(false)
      .task(id: starting) {
        late = false
        guard starting else { return }
        try? await Task.sleep(for: connectingDelay)
        withAnimation(Motion.ease) { late = true }
      }
  }
}
