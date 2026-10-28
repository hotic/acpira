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
  /// The files a session changed, and one file of its project (read-only, its changes marked)
  case changes(String)
  case file(session: String, path: String, line: Int?)
  /// A project's terminal on the engine's computer (Terminal/TerminalPage.swift)
  case terminal(cwd: String)
}

/// agentOrder.ts launchable: the agents a new session can start with, the disabled ones left out
func launchable(_ agents: [AgentInfo]) -> [AgentInfo] { agents.filter { $0.disabled != true } }

/// A short confirmation floating over the screen (copied, pinned, forked …), with an action when the step can be taken
/// back (a deleted session, a project taken off the list)
struct Toast: Identifiable, Equatable {
  let id = UUID()
  var text: String
  var icon: LucideIcon = .check
  var action: ToastAction?

  static func == (a: Toast, b: Toast) -> Bool { a.id == b.id }
}

struct ToastAction {
  var label: String
  var run: () -> Void
}

@Observable
final class AppModel {
  var agents: [AgentInfo] = Fixtures.agents
  var accounts: [AccountInfo] = Fixtures.accounts
  /// The engine's settings (steering, the agents' order …); nil in the demo, which reads every default
  var settings: SettingsView?
  /// Each agent's config controls as the engine last reported them (`controls`): the settings' model lists
  var agentControls: [AgentId: [ConfigControl]] = [:]
  var summaries: [SessionSummary]
  var views: [String: SessionView]
  var path: [Route] = []
  var toast: Toast?
  /// Folders added by hand, ahead of the ones sessions use
  var addedProjects: [String] = []
  /// Project folders in the user's order (SessionCategories.projects: moved to the top, added by hand) and the ones taken
  /// off the list (`removedProjects`), shared with every project shell on the machine
  var projectOrder: [String] = []
  var removedProjects: [String] = []
  /// Stand-ins for two engine fields that do not exist yet: projects pinned to the top of the by-project view (in pin
  /// order), and sessions pinned inside their project only (`SessionSummary.pinned` is the list's top pin)
  var pinnedProjects: [String] = []
  var projectPins: Set<String> = []
  /// The desktop app's sections ("Work", "Personal"), each holding whole projects in its own order; the by-project
  /// view lists them as the desktop does. Made and filled on the desktop only
  var sections: [SessionSection] = []
  /// The history search's content hits, for the query they answered
  var hits: SessionHits?
  /// Projects folded open in the by-project view
  var openProjects: Set<String> = []
  /// The settings sheet is up
  var settingsOpen = false
  /// A session's export waiting for the share sheet
  var shared: ExportFile?
  /// Each chat's unsent draft (AppModel+Composer.swift): the map is not observed, each draft is
  @ObservationIgnored var drafts: [String: ComposerDraft] = [:]
  /// The composer's `@` file search, answered by the engine's `files`
  @ObservationIgnored private(set) lazy var fileSearch = FileSearch { [weak self] query, seq in
    self?.post(.searchFiles(query: query, seq: seq)) ?? false
  }

  /// The live engine; nil in the demo
  @ObservationIgnored let link: EngineLink?
  /// The engine's terminals the phone opened, one per project (Terminal/TerminalSession.swift)
  @ObservationIgnored let terminals = TerminalHub()
  @ObservationIgnored private var runs: [String: Task<Void, Never>] = [:]
  @ObservationIgnored private let launched = Date()
  @ObservationIgnored private var serial = 0
  /// Reads waiting for the engine's answer, by the seq it echoes (AppModel+Files.swift)
  @ObservationIgnored var asking: [Int: CheckedContinuation<HostMsg?, Never>] = [:]
  @ObservationIgnored var askSeq = 0
  /// The engine turned this app down for another protocol version; the close that follows says nothing more
  @ObservationIgnored var incompatible = false
  /// The last history search sent to the engine: its reply echoes the seq
  @ObservationIgnored var searchSeq = 0
  @ObservationIgnored var searchQuery = ""
  /// Each account action's last state, so a repeated push does not repeat its toast
  @ObservationIgnored var actionStates: [String: AccountActionStatus] = [:]
  /// Deleted sessions the toast's undo can bring back (the demo's stand-in for the engine's trash)
  @ObservationIgnored private var trash: [String: (summary: SessionSummary, view: SessionView?)] = [:]

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
    // One session with background work, so the list shows every state mark
    if let at = self.summaries.firstIndex(where: { $0.title == "上下文压缩" }) { self.summaries[at].state = .background }
    // The engine's defaults, so the settings that live on the computer can be tried in the demo
    settings = .defaults
    // The latest project starts open, so the by-project view shows open and closed groups
    openProjects = Set(summaries.first.map { [$0.cwd] } ?? [])
    let fixture = fixtureId(options.fixture)
    path = switch options.screen {
    case "list", "settings": []
    case "new": [.new(agent: nil, cwd: nil)]
    // The review pages over the fixture's chat: its changed files, or the first of them opened
    case "changes": [.chat(fixture), .changes(fixture)]
    case "file": [.chat(fixture), .changes(fixture)]
      + (Self.demoChanges(views[fixture]).first.map { [.file(session: fixture, path: $0.path, line: nil)] } ?? [])
    default: [.chat(fixture)]
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

  func show(_ text: String, icon: LucideIcon = .check, action: ToastAction? = nil) {
    toast = Toast(text: text, icon: icon, action: action)
  }

  // MARK: - Projects

  /// Every known project for the folder menus, the latest first: folders added by hand, the ones sessions work in, then
  /// the registered ones no session used yet
  var projects: [String] {
    var seen = Set(removedProjects)
    let used = summaries.sorted { $0.updatedAt > $1.updatedAt }.map(\.cwd)
    return (addedProjects + used + projectOrder).filter { !$0.isEmpty && seen.insert($0).inserted }
  }

  /// The by-project view's projects (sessionTree.ts buildSessionTree): the user's order first, then every other folder a
  /// session ran in, the latest first. Folders taken off the list stay off; their sessions still show in a search
  var listedProjects: [String] {
    var seen = Set(removedProjects)
    let used = summaries.sorted { $0.updatedAt > $1.updatedAt }.map(\.cwd)
    return (projectOrder + addedProjects + used).filter { !$0.isEmpty && seen.insert($0).inserted }
  }

  /// The project leads the by-project view: registered if it was not, then moved before the first one (the desktop
  /// drags it there)
  func moveProjectToTop(_ cwd: String) {
    guard listedProjects.first != cwd else { return }
    if live {
      if !projectOrder.contains(cwd) { post(.categoryOp(op: .addProject(cwd: cwd))) }
      post(.categoryOp(op: .reorderProject(cwd: cwd, before: projectOrder.first { $0 != cwd })))
      return
    }
    projectOrder.removeAll { $0 == cwd }
    projectOrder.insert(cwd, at: 0)
  }

  /// Takes a folder off the list (its sessions stay and still show in a search); the toast puts it back
  func removeProject(_ cwd: String) {
    if !post(.categoryOp(op: .removeProject(cwd: cwd))) {
      projectOrder.removeAll { $0 == cwd }
      if !removedProjects.contains(cwd) { removedProjects.append(cwd) }
    }
    show(t("mobile.project.removed", ["name": projectName(cwd)]), icon: .folder,
         action: ToastAction(label: t("common.undo")) { [weak self] in self?.addProject(cwd) })
  }

  func toggleOpen(_ cwd: String) {
    if openProjects.remove(cwd) == nil { openProjects.insert(cwd) }
  }

  func togglePinProject(_ cwd: String) {
    let pinned = !pinnedProjects.contains(cwd)
    if pinned { pinnedProjects.append(cwd) } else { pinnedProjects.removeAll { $0 == cwd } }
    show(pinned ? t("mobile.pinned") : t("mobile.unpinned"), icon: pinned ? .pin : .pinOff)
  }

  func toggleProjectPin(_ id: String) {
    let pinned = projectPins.insert(id).inserted
    if !pinned { projectPins.remove(id) }
    show(pinned ? t("mobile.pinned") : t("mobile.unpinned"), icon: pinned ? .pin : .pinOff)
  }

  /// A folder picked by hand: first in the folder menus, and back on the list if it was taken off (the engine registers
  /// it at the end of the user's order)
  func addProject(_ path: String) {
    let trimmed = path.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !trimmed.isEmpty else { return }
    addedProjects.removeAll { $0 == trimmed }
    addedProjects.insert(trimmed, at: 0)
    if !post(.categoryOp(op: .addProject(cwd: trimmed))) {
      removedProjects.removeAll { $0 == trimmed }
      if !projectOrder.contains(trimmed) { projectOrder.append(trimmed) }
    }
  }

  // MARK: - List

  /// Every unread mark goes, for every engine and window on the machine (they share them)
  func markAllRead() {
    if post(.markAllRead) { return }
    for i in summaries.indices {
      summaries[i].unread = nil
      if summaries[i].state == .unread { summaries[i].state = nil }
    }
  }

  /// The history search over saved conversations; its content hits land in `hits` (the engine answers `sessionHits`,
  /// the demo searches its own views)
  func search(_ query: String) {
    let terms = searchTerms(query)
    guard !terms.isEmpty else {
      hits = nil
      return
    }
    let q = terms.joined(separator: " ")
    if live {
      searchSeq += 1
      searchQuery = q
      post(.searchSessions(query: q, seq: searchSeq))
      return
    }
    var found: [String: String] = [:]
    for (id, view) in views {
      if let snippet = SessionSearch.snippet(view, terms: terms) { found[id] = snippet }
    }
    hits = SessionHits(query: q, snippets: found)
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
      engineNewSession(agent: agent, cwd: cwd, controls: controls, text: text, attachments: attachments)
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

  /// Sends a prompt, or queues it while a turn runs (with steering on, the engine joins it to the running turn); while a
  /// plan waits on its card the words revise it
  func send(_ id: String, text: String, attachments: [Attachment]) {
    let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
    guard !trimmed.isEmpty || !attachments.isEmpty else { return }
    if !trimmed.isEmpty, let review = views[id].flatMap({ pendingPlanReview($0.turns) }), let revise = reviseOption(review) {
      revisePlan(id, review: review, option: revise.id, text: trimmed, attachments: attachments)
      return
    }
    if link != nil {
      let drafts = Self.drafts(attachments)
      post(.send(sessionId: id, text: trimmed, attachments: drafts.isEmpty ? nil : drafts, steer: steering(id) ? true : nil))
      return
    }
    guard var view = views[id] else { return }
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
    // A plan's extra approvals reach here only to reject it (the allow ones build): the turn ends as on Revise
    if let plan = planPermission(view, block: block)?.planId {
      settlePlan(id, plan: plan, status: "rejected")
      return
    }
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

  /// Settings `steerQueued` on an agent that can steer, while a turn runs: a send joins the turn, and a queued row's send
  /// steers it in (Shell.tsx `steering`)
  func steering(_ id: String) -> Bool {
    settings?.steerQueued == true && views[id]?.canSteer == true && views[id]?.running == true
  }

  /// What the engine takes for the composer's attachments: photos as their bytes; a file picked on the phone as its bytes
  /// too (its path means nothing on the computer: the engine stages the bytes and links its copy), any other file (an @
  /// mention, a path on the computer) as the link it is; quotes as they are
  static func drafts(_ attachments: [Attachment]) -> [Draft] {
    attachments.compactMap { a -> Draft? in
      switch a {
      case let .image(blob?, mime, name): return .image(mimeType: mime, data: blob, name: name)
      case let .file(uri, name):
        guard let url = URL(string: uri), AttachmentFile.isCopy(url) else { return .file(uri: uri, name: name) }
        return (try? Data(contentsOf: url)).map { .file(uri: uri, name: name, data: $0.base64EncodedString()) }
      case let .quote(text, comment): return .quote(text: text, comment: comment)
      default: return nil
      }
    }
  }

  // MARK: - Queue

  /// Send now: the running turn stops and the queued prompt goes at once; with steering on it joins the turn instead
  func sendQueued(_ id: String, item: String) {
    if steering(id), post(.steerQueued(sessionId: id, id: item)) { return }
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

  /// Retry: the failed turn goes, its prompt runs again (`retryTurn`; `retry` restarts the session, see `restart`)
  func retry(_ id: String) {
    if post(.retryTurn(sessionId: id)) { return }
    guard var view = views[id], case .agent? = view.turns.last else { return }
    view.turns.removeLast()
    guard case .user(let prompt)? = view.turns.last else { return }
    view.turns.removeLast()
    views[id] = view
    startTurn(id, text: prompt.text, attachments: prompt.attachments ?? [])
  }

  /// Continue after a limit: a follow-up asking the agent to carry on
  func carryOn(_ id: String) { send(id, text: t("alert.continueText"), attachments: []) }

  /// /compact, offered when the context no longer fits; the demo plays the compaction as its own automatic turn
  func compact(_ id: String) {
    if post(.compact(sessionId: id)) { return }
    guard let view = views[id], !view.running else { return }
    serial += 1
    var prompt = UserTurn(text: "/compact")
    prompt.auto = true
    prompt.autoReason = "compact"
    let started = now.timeIntervalSince1970 * 1000
    let running = CompactionBlock("demo-compact-\(serial)", status: .in_progress)
    var done = running
    done.status = .completed
    func step(_ block: CompactionBlock, ended: Bool) -> SessionView {
      var step = view
      step.turns.append(.user(prompt))
      step.turns.append(.agent(AgentTurn(blocks: [.compaction(block)], startedAt: started,
                                         endedAt: ended ? started + Double(Replay.hold) * Replay.interval * 1000 : nil,
                                         stop: ended ? .end_turn : nil)))
      step.running = !ended
      return step
    }
    apply(step(running, ended: false), to: id)
    play(id, Array(repeating: step(running, ended: false), count: Replay.hold) + [step(done, ended: true)])
  }

  // MARK: - Plans

  /// Build a saved plan (the plan card's Build): `option` answers its approval (the allow-once one when nil), `model`
  /// is the executor picked on the card. The demo goes on with a short build after the card
  func buildPlan(_ id: String, plan: String, option: String?, model: ModelPick?) {
    if post(.buildPlan(sessionId: id, planId: plan, optionId: option, model: model)) { return }
    guard let view = views[id], !view.turns.isEmpty else { return }
    if let model { setConfig(id, model.configId, model.value) }
    settlePlan(id, plan: plan, status: "executing")
    continuePlan(id, plan: plan)
  }

  /// A send while a plan waits on its card: Revise answers the card and the words go out as the next prompt, so they
  /// reach the agent after the rejection instead of steering a turn that is blocked on the card (Shell.tsx)
  func revisePlan(_ id: String, review: PermissionBlock, option: String, text: String, attachments: [Attachment]) {
    if link != nil {
      post(.permission(sessionId: id, blockId: review.id, optionId: option))
      post(.send(sessionId: id, text: text))
      return
    }
    settlePlan(id, plan: review.planId ?? "", status: "rejected")
    startTurn(id, text: text, attachments: attachments)
  }

  /// The permission block `block` of the newest agent turn, when it is a plan's approval
  private func planPermission(_ view: SessionView, block: String) -> PermissionBlock? {
    guard case .agent(let turn)? = view.turns.last else { return nil }
    for case .permission(let p) in turn.blocks where p.id == block && p.planId != nil { return p }
    return nil
  }

  /// Demo: the plan's approval answered. Its permission leaves, the plan takes `status`, and a rejection ends the turn
  /// the way claude-agent-acp does (cancelled, nothing said)
  private func settlePlan(_ id: String, plan: String, status: String) {
    guard var view = views[id], let at = view.turns.lastIndex(where: { if case .agent = $0 { true } else { false } }),
          case .agent(var turn) = view.turns[at] else { return }
    turn.blocks = turn.blocks.compactMap { block in
      switch block {
      case .permission(let p) where p.planId == plan: return nil
      case .planDocument(var d) where d.id == plan: d.status = status; return .planDocument(d)
      default: return block
      }
    }
    if status == "rejected" {
      runs[id]?.cancel()
      runs[id] = nil
      turn.stop = .cancelled
      turn.endedAt = now.timeIntervalSince1970 * 1000
      view.running = false
      touch(id, state: nil)
    }
    view.turns[at] = .agent(turn)
    views[id] = view
  }

  /// Demo: the built plan's turn goes on after the card, an edit and a word on where the reply comes from, then ends
  private func continuePlan(_ id: String, plan: String) {
    guard let view = views[id], let at = view.turns.lastIndex(where: { if case .agent = $0 { true } else { false } }),
          case .agent(let turn) = view.turns[at] else { return }
    let started = now.timeIntervalSince1970 * 1000
    let added: [AgentBlock] = [
      .toolCall(ToolCallBlock(id: "demo-build-\(Int(started))", kind: .edit, verb: t("verb.edit"), target: "README.md",
                              targetMono: true, status: .completed)),
      .text(TextBlock(markdown: t("mobile.demo.build"))),
    ]
    func step(_ count: Int, ended: Bool) -> SessionView {
      var step = view
      var live = turn
      live.blocks += added.prefix(count)
      if ended {
        live.blocks = live.blocks.map { block in
          if case .planDocument(var d) = block, d.id == plan { d.status = "approved"; return .planDocument(d) }
          return block
        }
        live.stop = .end_turn
        live.endedAt = started + Double(added.count + 1) * Replay.interval * 1000
      }
      step.turns[at] = .agent(live)
      step.running = !ended
      return step
    }
    play(id, (0...added.count).map { step($0, ended: false) } + [step(added.count, ended: true)])
  }

  // MARK: - Session state

  /// The session's agent started again (the session card's retry): after an error, or a sign-in finished on the
  /// computer
  func restart(_ id: String) {
    if post(.retry(sessionId: id)) { return }
    settleStatus(id)
  }

  /// Sign in with a method the engine runs itself, without a terminal on the computer
  func login(_ id: String, method: String) {
    if post(.login(sessionId: id, methodId: method)) { return }
    settleStatus(id)
  }

  /// Take the session over from the Acpira instance that holds it, or ask the engine with its copy open to hand it over
  func takeOver(_ id: String) {
    if post(.takeOverSession(sessionId: id)) { return }
    settleStatus(id)
  }

  /// "Continue in a new session": the new-session screen preset to this agent and project, as the bar's new button
  func continueNew(_ id: String) {
    guard let view = views[id] else { return }
    path.append(.new(agent: view.agent, cwd: view.cwd))
  }

  /// An AIR failure notice's remedy: a real turn retries its prompt, a session-scoped failure (no turn started)
  /// restarts the session; sign-in belongs to the session card
  func failureAction(_ id: String, _ action: FailureAction) {
    switch action {
    case .retry:
      if case .agent(let turn)? = views[id]?.turns.last, turn.startedAt != nil { retry(id) } else { restart(id) }
    case .new_session: continueNew(id)
    case .login, .unknown: break
    }
  }

  /// Demo: whatever held the session back is resolved
  private func settleStatus(_ id: String) {
    guard var view = views[id] else { return }
    view.status = .ready
    view.error = nil
    view.authMethods = nil
    view.canTakeOver = nil
    views[id] = view
  }

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

  /// An agent's accounts came into view: the engine refreshes their quotas
  func refreshQuota(_ agent: AgentId) { post(.refreshQuota(agent: agent)) }

  /// Adds the login the agent's CLI holds on the computer; a new login needs its terminal, which is on the computer
  func addAccount(_ agent: AgentId) {
    if post(.addAccount(agent: agent, via: .import)) { return }
    show(t("mobile.engineNeeded"), icon: .unplug)
  }

  func removeAccount(_ account: String) {
    if post(.removeAccount(id: account)) { return }
    accounts.removeAll { $0.id == account }
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

  /// Deletes at once, as the desktop does; the toast's undo restores the session
  func delete(_ id: String) {
    runs[id]?.cancel()
    runs[id] = nil
    let summary = summaries.first { $0.id == id }
    let title = views[id]?.title ?? summary?.title ?? ""
    if !post(.deleteSession(id: id)), let summary { trash[id] = (summary, views[id]) }
    summaries.removeAll { $0.id == id }
    views[id] = nil
    path.removeAll { $0 == .chat(id) }
    show(t("session.deleted", ["title": title]), icon: .trash2,
         action: ToastAction(label: t("common.undo")) { [weak self] in self?.restore(id) })
  }

  func restore(_ id: String) {
    if post(.restoreSession(id: id)) { return }
    guard let kept = trash.removeValue(forKey: id) else { return }
    summaries.append(kept.summary)
    views[id] = kept.view
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
  /// A saved plan's Build: the plan, the approval option (nil: its allow-once one), the model to run it on
  var buildPlan: (String, String?, ModelPick?) -> Void = { _, _, _ in }
  var compact: () -> Void = {}
  /// "Continue in a new session"
  var newSession: () -> Void = {}
  /// The session card's retry: the agent starts again
  var restart: () -> Void = {}
  /// Sign in with a method the engine runs itself
  var login: (String) -> Void = { _ in }
  var takeOver: () -> Void = {}
  /// An AIR failure notice's remedy
  var failureAction: (FailureAction) -> Void = { _ in }
  /// Stages text quoted from the transcript in the chat's composer
  var quote: (String) -> Void = { _ in }
}

extension EnvironmentValues {
  @Entry var sessionActions = SessionActions()
}

// MARK: - Toast

/// The toast: a Liquid Glass capsule under the navigation bar with the action's glyph and a few words, as the system's
/// own bars and buttons around it
struct ToastView: View {
  let toast: Toast
  var dismiss: () -> Void = {}
  var body: some View {
    HStack(spacing: Metrics.s2) {
      Icon(toast.icon, size: Metrics.iconCtl, strokeWidth: 1.75)
      Text(web: toast.text, paragraph: false).lineLimit(1).truncationMode(.middle)
      if let action = toast.action {
        // The desktop toast's secondary button, as the capsule's last word after a hairline
        Rectangle().fill(Palette.lineStrong).frame(width: Metrics.hairline, height: Metrics.iconCtl)
        Button {
          action.run()
          dismiss()
        } label: {
          Text(web: action.label, paragraph: false).textStyle(TypeScale.text2, weight: .semibold)
            .frame(maxHeight: .infinity)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
      }
    }
    .textStyle(TypeScale.text2, weight: .medium)
    .foregroundStyle(Palette.fgStrong)
    .padding(.horizontal, Metrics.pad)
    .frame(height: Metrics.mCtl)
    .glassEffect(toast.action == nil ? .regular : .regular.interactive(), in: Capsule())
    // Translucent, it would show the system bar's glass buttons through it: it drops in under the bar instead
    .padding(.top, Metrics.navBar)
    .padding(.horizontal, Metrics.page)
    .accessibilityAddTraits(.updatesFrequently)
  }
}

extension View {
  /// Shows the model's toast over this view, dropping in from the top and leaving after Motion.toast (one with an undo
  /// stays longer, and only it takes taps)
  func toastLayer(_ model: AppModel) -> some View {
    overlay(alignment: .top) {
      ZStack {
        if let toast = model.toast {
          ToastView(toast: toast) { if model.toast?.id == toast.id { model.toast = nil } }
            .id(toast.id)
            .transition(.move(edge: .top).combined(with: .opacity))
            .task(id: toast.id) {
              try? await Task.sleep(for: .seconds(toast.action == nil ? Motion.toast : Motion.toastUndo))
              if model.toast?.id == toast.id { model.toast = nil }
            }
        }
      }
      .animation(Motion.open, value: model.toast)
      .allowsHitTesting(model.toast?.action != nil)
    }
  }
}
