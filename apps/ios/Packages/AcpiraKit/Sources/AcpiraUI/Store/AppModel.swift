import AcpiraDesign
import AcpiraFixtures
import AcpiraModels
import Foundation
import Observation
import SwiftUI

// The app's state. With `-engine PORT:TOKEN` it follows a live engine (EngineLink, AppModel+Engine.swift): the lists
// and views come from its pushes and every control posts the engine's action. Without it the app is a demo on the
// fixture sessions, with a local stand-in for the engine's session manager: every control acts on it (send, stop,
// queue, permissions, questions, fork, pin, rename, delete, modes, models, accounts), and a sent prompt gets a demo
// reply streamed the way `-replay` streams a fixture.

/// Where the navigation stack can go
public enum Route: Hashable {
  case chat(String)
  /// The new-session screen, preset to an agent and a project when it is opened from a session
  case new(agent: AgentId?, cwd: String?)
  /// A project's terminal on the engine's computer (Terminal/TerminalPage.swift)
  case terminal(cwd: String)
}

/// agentOrder.ts launchable: the agents a new session can start with, the disabled ones left out
func launchable(_ agents: [AgentInfo]) -> [AgentInfo] { agents.filter { $0.disabled != true } }

/// A short confirmation floating over the screen (copied, pinned, forked …)
struct Toast: Identifiable, Equatable {
  let id = UUID()
  var text: String
  var icon: LucideIcon = .check
}

@Observable
final class AppModel {
  var agents: [AgentInfo] = Fixtures.agents
  var accounts: [AccountInfo] = Fixtures.accounts
  var summaries: [SessionSummary]
  var views: [String: SessionView]
  var path: [Route] = []
  var toast: Toast?
  /// Folders added by hand, ahead of the ones sessions use
  var addedProjects: [String] = []
  /// Stand-ins for two engine fields that do not exist yet: projects pinned to the top of the by-project view (in pin
  /// order), and sessions pinned inside their project only (`SessionSummary.pinned` is the list's top pin)
  var pinnedProjects: [String] = []
  var projectPins: Set<String> = []
  /// Projects folded open in the by-project view
  var openProjects: Set<String> = []
  /// The settings sheet is up
  var settingsOpen = false
  /// A session's export waiting for the share sheet
  var shared: ExportFile?

  /// The live engine; nil in the demo
  @ObservationIgnored let link: EngineLink?
  /// The engine's terminals the phone opened, one per project (Terminal/TerminalSession.swift)
  @ObservationIgnored let terminals = TerminalHub()
  @ObservationIgnored private var runs: [String: Task<Void, Never>] = [:]
  @ObservationIgnored private let launched = Date()
  @ObservationIgnored private var serial = 0

  convenience init(options: LaunchOptions) {
    if let address = EngineAddress(options.engine) {
      self.init(link: EngineLink(address: address, language: options.locale ?? Locale.preferredLanguages.first ?? "en"), options: options)
    } else {
      self.init(demo: options)
    }
  }

  /// Follows a live engine: everything listed and shown comes from its pushes
  init(link: EngineLink, options: LaunchOptions) {
    self.link = link
    agents = []
    accounts = []
    summaries = []
    views = [:]
    path = options.session.map { [.chat($0)] } ?? (options.screen == "new" ? [.new(agent: nil, cwd: nil)] : [])
    settingsOpen = options.screen == "settings"
    connect()
  }

  private init(demo options: LaunchOptions) {
    link = nil
    let fixtures = Fixtures.sessionNames.compactMap { name in Fixtures.session(name).map { (name: name, view: $0) } }
    let showcase = fixtures.first { $0.name == "showcase" }?.view
    var views: [String: SessionView] = [:]
    let summaries = Fixtures.sessions
    // A listed session opens the fixture with its title, or the showcase dressed as that session
    for s in summaries {
      guard var view = fixtures.first(where: { $0.view.title == s.title })?.view ?? showcase else { continue }
      if view.agent != s.agent { view.accountId = s.accountId }
      view.id = s.id; view.title = s.title; view.agent = s.agent; view.cwd = s.cwd
      views[s.id] = view
    }
    // Fixtures no listed session points at stay reachable by `-fixture`
    for f in fixtures where !summaries.contains(where: { $0.title == f.view.title }) {
      var view = f.view
      view.id = "fx-\(f.name)"
      views[view.id] = view
    }
    self.summaries = summaries
    self.views = views
    // Demo data for the pin levels until the engine carries them, and one session with background work so the list
    // shows every state mark
    if let at = self.summaries.firstIndex(where: { $0.title == "上下文压缩" }) { self.summaries[at].state = .background }
    pinnedProjects = summaries.first.map { [$0.cwd] } ?? []
    projectPins = Set(summaries.filter { $0.title == "账号层测试补齐" }.map(\.id))
    // A pinned project starts open, so the by-project view shows open and closed groups
    openProjects = Set(pinnedProjects)
    path = switch options.screen {
    case "list", "settings": []
    case "new": [.new(agent: nil, cwd: nil)]
    default: [.chat(fixtureId(options.fixture))]
    }
    settingsOpen = options.screen == "settings"
  }

  /// The session a fixture name opens
  func fixtureId(_ name: String) -> String {
    guard let title = Fixtures.session(name)?.title else { return summaries.first?.id ?? "" }
    return summaries.first { $0.title == title }?.id ?? "fx-\(name)"
  }

  /// The app's clock: the real one with an engine, else the fixtures' clock running from launch (new sessions sort
  /// after the synthetic ones)
  var now: Date { Date().addingTimeInterval(clockOffset) }
  /// How far the fixtures' clock runs ahead of the real one
  var clockOffset: TimeInterval { link == nil ? Fixtures.now.timeIntervalSince(launched) : 0 }

  func agent(_ id: AgentId) -> AgentInfo { agents.first { $0.id == id } ?? AgentInfo(id: id, name: id) }

  func show(_ text: String, icon: LucideIcon = .check) { toast = Toast(text: text, icon: icon) }

  // MARK: - Projects

  /// Every known project, the latest first: folders added by hand, then the ones sessions work in
  var projects: [String] {
    var seen = Set<String>()
    let used = summaries.sorted { $0.updatedAt > $1.updatedAt }.map(\.cwd)
    return (addedProjects + used).filter { !$0.isEmpty && seen.insert($0).inserted }
  }

  /// The by-project view's projects: the pinned ones in pin order, then the rest, the latest first
  var listedProjects: [String] {
    let pinned = pinnedProjects.filter(projects.contains)
    return pinned + projects.filter { !pinned.contains($0) }
  }

  func togglePinProject(_ cwd: String) {
    let pinned = !pinnedProjects.contains(cwd)
    if pinned { pinnedProjects.append(cwd) } else { pinnedProjects.removeAll { $0 == cwd } }
    show(pinned ? t("mobile.pinned") : t("mobile.unpinned"), icon: pinned ? .pin : .pinOff)
  }

  func toggleOpen(_ cwd: String) {
    if openProjects.remove(cwd) == nil { openProjects.insert(cwd) }
  }

  func toggleProjectPin(_ id: String) {
    let pinned = projectPins.insert(id).inserted
    if !pinned { projectPins.remove(id) }
    show(pinned ? t("mobile.pinned") : t("mobile.unpinned"), icon: pinned ? .pin : .pinOff)
  }

  func addProject(_ path: String) {
    let trimmed = path.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !trimmed.isEmpty else { return }
    addedProjects.removeAll { $0 == trimmed }
    addedProjects.insert(trimmed, at: 0)
  }

  // MARK: - Sessions

  /// The controls a new session of `agent` starts with: those of its latest session, else the showcase's
  func controls(for agent: AgentId) -> SessionControls {
    let latest = summaries.filter { $0.agent == agent }.max { $0.updatedAt < $1.updatedAt }
    let fallback = link == nil ? Fixtures.session("showcase")?.controls : nil
    return latest.flatMap { views[$0.id]?.controls } ?? fallback ?? SessionControls()
  }

  /// Starts a session with its first prompt and returns its id; nil with an engine, which pushes the session it made
  /// and the chat then takes the new-session screen's place
  @discardableResult
  func newSession(agent: AgentId, cwd: String, controls: SessionControls, text: String, attachments: [Attachment]) -> String? {
    if link != nil {
      engineNewSession(agent: agent, cwd: cwd, text: text)
      return nil
    }
    serial += 1
    let id = "local-\(serial)"
    let stamp = now.iso
    let account = accounts.first { $0.agent == agent }?.id
    let view = SessionView(id: id, agent: agent, title: title(from: text), cwd: cwd, accountId: account,
                           controls: controls, createdAt: stamp, updatedAt: stamp)
    views[id] = view
    summaries.insert(SessionSummary(id: id, title: view.title, agent: agent, cwd: cwd, updatedAt: stamp, accountId: account), at: 0)
    addProject(cwd)
    startTurn(id, text: text, attachments: attachments)
    return id
  }

  /// The first line of the first prompt, as the webview titles a session before the agent names it
  private func title(from text: String) -> String {
    let line = text.split(whereSeparator: \.isNewline).first.map(String.init)?.trimmingCharacters(in: .whitespaces) ?? ""
    guard !line.isEmpty else { return t("session.untitled") }
    return line.count > 40 ? String(line.prefix(40)) + "…" : line
  }

  /// Sends a prompt, or queues it while a turn runs
  func send(_ id: String, text: String, attachments: [Attachment]) {
    let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
    // Attachments go to the engine with P2's drafts; until then only the text does
    if link != nil {
      if !trimmed.isEmpty { post(.send(sessionId: id, text: trimmed)) }
      return
    }
    guard var view = views[id], !trimmed.isEmpty || !attachments.isEmpty else { return }
    if view.running {
      serial += 1
      view.queued = (view.queued ?? []) + [QueuedPrompt(id: "q-\(serial)", text: trimmed, attachments: attachments.isEmpty ? nil : attachments)]
      views[id] = view
      return
    }
    startTurn(id, text: trimmed, attachments: attachments)
  }

  /// Appends the prompt and streams the demo reply after it
  private func startTurn(_ id: String, text: String, attachments: [Attachment]) {
    guard var view = views[id] else { return }
    view.turns.append(.user(UserTurn(text: text, attachments: attachments.isEmpty ? nil : attachments)))
    view.running = false
    var full = view
    full.turns.append(.agent(DemoReply.turn(for: text, cwd: view.cwd, at: now)))
    let steps = Replay.steps(full)
    // The prompt and the empty running turn land at once, so a second prompt sent right after queues
    if let first = steps.first { apply(first, to: id) }
    play(id, Array(steps.dropFirst()))
  }

  /// Plays a turn's steps into the session, then lets the queue send its next prompt
  private func play(_ id: String, _ steps: [SessionView], delay: Double = 0) {
    runs[id]?.cancel()
    touch(id, state: .working)
    runs[id] = Task { [weak self] in
      if delay > 0 { try? await Task.sleep(for: .seconds(delay)) }
      for step in steps {
        guard !Task.isCancelled, let self else { return }
        self.apply(step, to: id)
        try? await Task.sleep(for: .seconds(Replay.interval))
      }
      guard !Task.isCancelled, let self else { return }
      self.finish(id)
    }
  }

  /// `-replay`: the session's last agent turn streams again (the demo only: an engine's views are its own)
  func replay(_ id: String) {
    guard link == nil, let view = views[id] else { return }
    // The replayed turn's clock starts with the stream, a second from now
    let steps = Replay.steps(view, startedAt: now.addingTimeInterval(1).timeIntervalSince1970 * 1000)
    if let first = steps.first { apply(first, to: id) }
    play(id, Array(steps.dropFirst()), delay: 1)
  }

  /// A stream step carries the transcript; the queue and controls stay as the reader left them
  private func apply(_ step: SessionView, to id: String) {
    guard var view = views[id] else { return }
    view.turns = step.turns
    view.running = step.running
    views[id] = view
  }

  private func finish(_ id: String) {
    runs[id] = nil
    touch(id, state: nil)
    guard var view = views[id], var queue = view.queued, !queue.isEmpty else { return }
    let next = queue.removeFirst()
    view.queued = queue.isEmpty ? nil : queue
    views[id] = view
    startTurn(id, text: next.text, attachments: next.attachments ?? [])
  }

  /// Stop: the running turn ends cancelled, whatever was still streaming settles where it stood
  func stop(_ id: String) {
    if post(.stop(sessionId: id)) { return }
    runs[id]?.cancel()
    runs[id] = nil
    guard var view = views[id] else { return }
    if let at = view.turns.lastIndex(where: { if case .agent = $0 { true } else { false } }), case .agent(var turn) = view.turns[at],
       turn.stop == nil {
      turn.blocks = turn.blocks.map(Self.settled)
      turn.stop = .cancelled
      turn.activity = nil
      turn.endedAt = now.timeIntervalSince1970 * 1000
      view.turns[at] = .agent(turn)
    }
    view.running = false
    views[id] = view
    touch(id, state: nil)
  }

  private static func settled(_ block: AgentBlock) -> AgentBlock {
    switch block {
    case .text(var b): b.streaming = false; return .text(b)
    case .thought(var b): b.streaming = false; return .thought(b)
    case .toolCall(var b) where b.status == .in_progress || b.status == .pending: b.status = .cancelled; return .toolCall(b)
    default: return block
    }
  }

  /// The summary's time and state follow the session
  private func touch(_ id: String, state: SessionState?) {
    guard let at = summaries.firstIndex(where: { $0.id == id }) else { return }
    summaries[at].state = state
    summaries[at].updatedAt = now.iso
  }

  // MARK: - Cards in the transcript

  /// A permission answered: the card leaves the transcript, as the engine drops it once the agent has the outcome
  func choosePermission(_ id: String, block: String, option: String) {
    if post(.permission(sessionId: id, blockId: block, optionId: option)) { return }
    guard var view = views[id] else { return }
    view.turns = view.turns.map { turn in
      guard case .agent(var agent) = turn else { return turn }
      agent.blocks.removeAll { if case .permission(let p) = $0 { p.id == block } else { false } }
      return .agent(agent)
    }
    views[id] = view
  }

  /// A question answered or skipped: it closes, and its card leaves the dock
  func answer(_ id: String, question: String, answers: [String: QuestionAnswer], skipped: Bool) {
    if post(.answer(sessionId: id, blockId: question, answers: skipped ? [:] : answers, skip: skipped ? true : nil)) { return }
    guard var view = views[id] else { return }
    view.turns = view.turns.map { turn in
      guard case .agent(var agent) = turn else { return turn }
      agent.blocks = agent.blocks.map { block in
        guard case .question(var q) = block, q.id == question else { return block }
        q.outcome = skipped ? .skipped : .answered
        q.answers = skipped ? nil : answers
        return .question(q)
      }
      return .agent(agent)
    }
    views[id] = view
  }

  // MARK: - Queue

  /// Send now (or steer): the running turn stops and the queued prompt goes at once
  func sendQueued(_ id: String, item: String) {
    if post(.sendQueued(sessionId: id, id: item)) { return }
    guard let prompt = takeQueued(id, item) else { return }
    if views[id]?.running == true { stop(id) }
    startTurn(id, text: prompt.text, attachments: prompt.attachments ?? [])
  }

  /// Takes a prompt out of the queue (edit puts its text back in the composer, remove drops it)
  @discardableResult
  func takeQueued(_ id: String, _ item: String) -> QueuedPrompt? {
    if link != nil {
      let prompt = views[id]?.queued?.first { $0.id == item }
      post(.dequeue(sessionId: id, id: item))
      return prompt
    }
    guard var view = views[id], let at = view.queued?.firstIndex(where: { $0.id == item }) else { return nil }
    let prompt = view.queued?.remove(at: at)
    if view.queued?.isEmpty == true { view.queued = nil }
    views[id] = view
    return prompt
  }

  // MARK: - Short stops

  /// Retry: the failed turn goes, its prompt runs again
  func retry(_ id: String) {
    if post(.retry(sessionId: id)) { return }
    guard var view = views[id], case .agent? = view.turns.last else { return }
    view.turns.removeLast()
    guard case .user(let prompt)? = view.turns.last else { return }
    view.turns.removeLast()
    views[id] = view
    startTurn(id, text: prompt.text, attachments: prompt.attachments ?? [])
  }

  /// Continue after a limit: a follow-up asking the agent to carry on
  func carryOn(_ id: String) { send(id, text: t("alert.continue"), attachments: []) }

  // MARK: - Controls

  func setMode(_ id: String, _ mode: String) {
    if post(.setMode(sessionId: id, id: mode)) { return }
    views[id]?.controls.modeId = mode
  }

  func setConfig(_ id: String, _ control: String, _ value: String) {
    if post(.setConfig(sessionId: id, configId: control, value: value)) { return }
    guard let at = views[id]?.controls.options.firstIndex(where: { $0.id == control }) else { return }
    views[id]?.controls.options[at].value = value
  }

  func selectAccount(_ id: String, _ account: String) {
    if post(.selectAccount(sessionId: id, id: account)) { return }
    views[id]?.accountId = account
    if let at = summaries.firstIndex(where: { $0.id == id }) { summaries[at].accountId = account }
    if let label = accounts.first(where: { $0.id == account })?.label { show(t("mobile.account.switched", ["name": label]), icon: .userRound) }
  }

  // MARK: - Session actions

  func togglePin(_ id: String) {
    guard let at = summaries.firstIndex(where: { $0.id == id }) else { return }
    let pinned = summaries[at].pinned != true
    if !post(.pinSession(id: id, pinned: pinned)) { summaries[at].pinned = pinned ? true : nil }
    show(pinned ? t("mobile.pinned") : t("mobile.unpinned"), icon: pinned ? .pin : .pinOff)
  }

  func rename(_ id: String, to title: String) {
    let trimmed = title.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !trimmed.isEmpty else { return }
    if post(.renameSession(id: id, title: trimmed)) { return }
    views[id]?.title = trimmed
    if let at = summaries.firstIndex(where: { $0.id == id }) { summaries[at].title = trimmed }
  }

  func delete(_ id: String) {
    runs[id]?.cancel()
    runs[id] = nil
    let title = views[id]?.title ?? summaries.first { $0.id == id }?.title ?? ""
    post(.deleteSession(id: id))
    summaries.removeAll { $0.id == id }
    projectPins.remove(id)
    views[id] = nil
    path.removeAll { $0 == .chat(id) }
    show(t("session.deleted", ["title": title]), icon: .trash2)
  }

  /// Fork from a turn: a new session holding the transcript up to and including that turn, opened over this one
  func fork(_ id: String, through turn: Int) {
    guard let source = views[id], source.turns.indices.contains(turn) else { return }
    if link != nil {
      engineFork(id, through: turn)
      return
    }
    serial += 1
    let forkId = "local-\(serial)"
    let stamp = now.iso
    var view = source
    view.id = forkId
    view.title = t("session.forkTitle", ["title": source.title])
    view.turns = Array(source.turns[...turn])
    view.running = false
    view.queued = nil
    view.createdAt = stamp
    view.updatedAt = stamp
    views[forkId] = view
    summaries.insert(SessionSummary(id: forkId, title: view.title, agent: view.agent, cwd: view.cwd, updatedAt: stamp,
                                    accountId: view.accountId), at: 0)
    path.append(.chat(forkId))
    show(t("mobile.forked"), icon: .gitBranch)
  }

  /// Writes the export to a temporary file and hands it to the share sheet
  func export(_ id: String, json: Bool) {
    guard let view = views[id] else { return }
    let name = "\(SessionExport.fileName(view.title)).\(json ? "json" : "md")"
    let body = json ? SessionExport.json(view) : SessionExport.markdown(view, agentName: agent(view.agent).name, exportedAt: now)
    let url = FileManager.default.temporaryDirectory.appendingPathComponent(name)
    do {
      try Data(body.utf8).write(to: url, options: .atomic)
      shared = ExportFile(url: url)
    } catch {
      show(error.localizedDescription, icon: .triangleAlert)
    }
  }
}

/// A file waiting for the share sheet
struct ExportFile: Identifiable, Equatable {
  let id = UUID()
  var url: URL
}

// MARK: - Session actions in the environment

/// What the controls inside a chat call, bound to that chat's session. Defaults do nothing, so the full-length render
/// and previews draw the same views without a model
struct SessionActions {
  var send: (String, [Attachment]) -> Void = { _, _ in }
  var stop: () -> Void = {}
  var permission: (String, String) -> Void = { _, _ in }
  var answer: (String, [String: QuestionAnswer], Bool) -> Void = { _, _, _ in }
  var fork: (Int) -> Void = { _ in }
  var toast: (String, LucideIcon) -> Void = { _, _ in }
  var sendQueued: (String) -> Void = { _ in }
  var editQueued: (String) -> Void = { _ in }
  var removeQueued: (String) -> Void = { _ in }
  var retry: () -> Void = {}
  var carryOn: () -> Void = {}
  var setMode: (String) -> Void = { _ in }
  var setConfig: (String, String) -> Void = { _, _ in }
}

extension EnvironmentValues {
  @Entry var sessionActions = SessionActions()
}

// MARK: - Toast

/// The toast: a Liquid Glass capsule under the navigation bar with the action's glyph and a few words, as the system's
/// own bars and buttons around it
struct ToastView: View {
  let toast: Toast
  var body: some View {
    HStack(spacing: Metrics.s2) {
      Icon(toast.icon, size: Metrics.iconCtl, strokeWidth: 1.75)
      Text(web: toast.text, paragraph: false).lineLimit(1).truncationMode(.middle)
    }
    .textStyle(TypeScale.text2, weight: .medium)
    .foregroundStyle(Palette.fgStrong)
    .padding(.horizontal, Metrics.pad)
    .frame(height: Metrics.mCtl)
    .glassEffect(.regular, in: Capsule())
    // Translucent, it would show the system bar's glass buttons through it: it drops in under the bar instead
    .padding(.top, Metrics.navBar)
    .padding(.horizontal, Metrics.page)
    .accessibilityAddTraits(.updatesFrequently)
  }
}

extension View {
  /// Shows the model's toast over this view, dropping in from the top and leaving after Motion.toast
  func toastLayer(_ model: AppModel) -> some View {
    overlay(alignment: .top) {
      ZStack {
        if let toast = model.toast {
          ToastView(toast: toast)
            .id(toast.id)
            .transition(.move(edge: .top).combined(with: .opacity))
            .task(id: toast.id) {
              try? await Task.sleep(for: .seconds(Motion.toast))
              if model.toast?.id == toast.id { model.toast = nil }
            }
        }
      }
      .animation(Motion.open, value: model.toast)
      .allowsHitTesting(false)
    }
  }
}
