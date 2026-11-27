import AcpiraDesign
import AcpiraFixtures
import AcpiraModels
import AcpiraSync
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
  /// One delegated child's transcript, read-only (Subagents/SubagentInspector.swift)
  case subagent(session: String, id: String)
  /// A session's subagents as a tree (Subagents/SubagentTree.swift)
  case subagents(session: String)

  /// The session the screen shows or reads from; nil for the new-session screen and a terminal
  var session: String? {
    switch self {
    case .chat(let id), .changes(let id): id
    case .file(let session, _, _), .subagent(let session, _), .subagents(let session): session
    case .new, .terminal: nil
    }
  }
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
  /// Each agent's remembered per-model parameters, from the same answer (modelShapes.ts): a model picked on the new
  /// session page brings its own effort and Fast from here, as only a live session's set_config_option would
  var agentShapes: [AgentId: ModelShapes] = [:]
  /// Each agent's slash commands from the same answer (its newest session's): the new session page has no session to
  /// advertise them, the desktop starts the CLI with its page and the phone only with the first prompt
  var agentCommands: [AgentId: [SlashCommand]] = [:]
  /// The engine's `controls` answers for a new session in a folder on an SSH host, by `hostControlsKey`: there the agent
  /// may start in the host's engine, with the host CLI's own config and models, so the agent-wide lists above do not fit
  var hostControls: [String: HostControls] = [:]
  /// Agents (or `hostControlsKey`s) whose empty `controls` answer was followed by a fresh probe on this wire, so it is
  /// asked once
  var probedControls: Set<String> = []
  var summaries: [SessionSummary]
  /// The session list exactly as the engine last sent it, the base its next `sessionsPatch` applies to (`summaries`
  /// carries local pending feedback on top)
  @ObservationIgnored var engineSummaries: [SessionSummary] = []
  /// The revision of `engineSummaries` (0 for a whole list); nil from a patch that did not fit until the whole list
  /// asked for lands, so the patches in flight meanwhile neither apply nor ask again
  @ObservationIgnored var engineListRev: Int? = 0
  /// List feedback stays until the list itself acknowledges it, even after a queued copy settles `outgoing`.
  var pendingSummaries: [String: PendingSummary] = [:]
  var views: [String: SessionView]
  /// Views carried over from the wire before (`resume`) that this wire has not shown yet, least recently shown first.
  /// They stay beside the feed's own until the cache bound needs their room, so a chat opened after a reconnect shows
  /// its last copy at once (syncing) instead of a spinner until the whole transcript arrives again
  @ObservationIgnored var carried: [String] = []
  /// Why a session picked on this wire never showed, by session: the engine's sentence (`sessionUnavailable`), or the
  /// app's own when the view arrived but did not decode. The chat's wait page shows it; a retry or the view clears it
  var unavailable: [String: String] = [:]
  /// Prompts sent to the engine that their session does not show yet, by session (Outgoing.swift)
  var outgoing: [String: [Outgoing]] = [:]
  /// Sessions whose earlier part the thread asked the engine for and has no answer to yet (`loadEarlier`), each with
  /// the wait that gives up on it: one ask under way per session. Observed: the thread's first row shows its spinner
  /// only while one is
  var loadingEarlier: [String: Task<Void, Never>] = [:]
  /// How many earlier parts a thread too short to scroll may still ask for by itself, by session (`fillEarlier`); none
  /// noted is the whole allowance
  @ObservationIgnored var earlierFills: [String: Int] = [:]
  var path: [Route] = []
  /// A session link opened before any paired computer's wire (the link launched the app): the first wire's model opens
  /// it (RootView.followPairing)
  @ObservationIgnored var linked: String?
  /// A paired build's model before any wire (`init(blank:)`): it knows no session yet, so a link waits rather than fails
  @ObservationIgnored private(set) var blank = false
  var toast: Toast?
  /// Folders added by hand, ahead of the ones sessions use
  var addedProjects: [String] = []
  /// Project folders in the user's order (SessionCategories.projects: moved to the top, added by hand) and the ones taken
  /// off the list (`removedProjects`), shared with every project shell on the machine
  var projectOrder: [String] = []
  var removedProjects: [String] = []
  /// The desktop app's sections ("Work", "Personal"), each holding whole projects in its own order; the by-project
  /// view lists them as the desktop does. Made and filled on the desktop only
  var sections: [SessionSection] = []
  /// Where chats' folders live on the computer (InitState.chats): a session whose folder sits directly inside is a chat,
  /// in no project. Nil in the demo and from an engine older than chats
  var chatRoot: String?
  /// The SSH hosts set up on the computer (HostMsg `connections`): each host folder's state and run mode. Empty until
  /// the engine answers, and for a gateway that keeps the list from devices; a host folder then shows its alias only
  var connections: [ConnectionView] = []
  /// helloOk.features of this wire's engine: additions it serves beyond the protocol version (`localDir`: the folder
  /// browser, `providers`: the built-in agent's model sources). Empty in the demo and from older engines
  var engineFeatures: Set<String> = []
  /// The built-in agent's model sources as the engine last sent them (AppModel+Providers.swift), nil until asked
  var providersView: ProvidersView?
  /// Why the last change to the sources failed, the engine's own words
  var providersError: String?
  /// Source changes in order, one at a time: the engine's answers carry no request id
  var providerWrites = ProviderWrites()
  /// Usage across every session as the engine last sent it (Settings → Usage), nil until asked
  var usageView: UsageView?
  /// The history search's content hits, for the query they answered
  var hits: SessionHits?
  /// Projects folded open in the by-project view
  var openProjects: Set<String> = []
  /// The settings sheet is up
  var settingsOpen = false
  /// A session's export waiting for the share sheet
  var shared: ExportFile?
  /// The observed subagents' transcripts and the one the phone follows (AppModel+Subagents.swift)
  var subagentFeed = SubagentFeed()
  /// This wire's init came in, so the lists are the engine's own
  var synced = false
  /// The screens and lists came over from the wire before to the same computer, until this one's init replaces them
  /// (AppModel+Engine.swift `resume`)
  var resumed = false
  /// The session the engine's view was last told this screen shows (`open`); nil once it was told the screen left it
  /// (AppModel+Engine.swift `syncWatching`)
  @ObservationIgnored var watching: String?
  /// Sessions whose latest view update came from a live stream step rather than an initial load or catch-up push
  @ObservationIgnored var liveSessions: Set<String> = []
  /// The app is in the background: the engine's view counts no session as watched until it comes back
  @ObservationIgnored var away = false
  /// This model's wire ended; the screens keep what it showed until the next wire's model takes them over
  var ended = false
  /// The real time the screens stopped following the engine live: the app went to the background or the wire ended.
  /// Running counters stand at it until the engine's view of the chat on screen comes again, so they never run on
  /// through the absence and then jump back to the engine's time (a turn that ended meanwhile)
  var clockHeld: Date?
  /// Each chat's unsent draft (AppModel+Composer.swift): the map is not observed, each draft is
  @ObservationIgnored var drafts: [String: ComposerDraft] = [:]
  /// The composer's `@` file search, answered by the engine's `files`
  @ObservationIgnored private(set) lazy var fileSearch = FileSearch { [weak self] query, seq in
    self?.post(.searchFiles(query: query, seq: seq)) ?? false
  }

  /// Session actions for this wire to send once its init is in (AppModel+Engine.swift `flushReplay`): the ones the
  /// wire before may not have delivered, and the ones posted before this wire's view could show their session
  @ObservationIgnored var replay: [WebviewMsg] = []
  /// The live engine; nil in the demo
  @ObservationIgnored let link: EngineLink?
  /// Told when the live engine's wire ends (a paired computer's connection reports it to the pairing model)
  @ObservationIgnored var onClosed: (() -> Void)?
  /// The paired computer whose engine this is (its id); nil for the demo and the development engine
  @ObservationIgnored var computer: String?
  /// The engine's terminals the phone opened, one per project (Terminal/TerminalSession.swift)
  @ObservationIgnored let terminals = TerminalHub()
  @ObservationIgnored var runs: [String: Task<Void, Never>] = [:]
  @ObservationIgnored private let launched = Date()
  @ObservationIgnored var serial = 0
  /// Reads waiting for the engine's answer, by the seq it echoes (AppModel+Files.swift)
  @ObservationIgnored var asking: [Int: CheckedContinuation<HostMsg?, Never>] = [:]
  @ObservationIgnored var askSeq = 0
  /// Model-source probes waiting for `providerProbed`, by id; source writes waiting for their settling answer, by write id
  @ObservationIgnored var probing: [String: CheckedContinuation<ProbeOutcome?, Never>] = [:]
  @ObservationIgnored var probeSeq = 0
  @ObservationIgnored var writeWaits: [Int: CheckedContinuation<String?, Never>] = [:]
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
    path += Self.subagentPath(options, after: path)
    settingsOpen = options.screen == "settings"
    connect()
  }

  /// Nothing to show: a paired build before any computer's wire, so no fixture stands in for the user's sessions. The
  /// connect page covers the list until a wire brings its own model (RootView.followPairing)
  init(blank _: LaunchOptions) {
    link = nil
    agents = []
    accounts = []
    summaries = []
    views = [:]
    blank = true
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
    // The fixtures' computer keeps chats where a project shell's engine does, so the demo can start them too
    chatRoot = "/Users/dev/.acpira/chats"
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
    path += Self.subagentPath(options, after: path)
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
      searchSeq += 1
      searchQuery = ""
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

  /// The controls a new session of `agent` starts with: those of its latest session while the app keeps its view, else
  /// the engine's answer to `controls` (its probe or its newest session, `loadControls`), else in the demo a fixture of
  /// that agent's or the showcase's. Only sessions opened on the phone are kept, so without the engine's answer a new
  /// session showed no model chip until its first prompt had made it. Protocol modes come with a session only, so the
  /// engine's answer has none
  func controls(for agent: AgentId, cwd: String = "") -> SessionControls {
    // A host folder: a kept session there, else the engine's answer for that folder, never another machine's list
    if let key = hostControlsKey(agent, cwd) {
      let kept = summaries.filter { $0.agent == agent && $0.cwd == cwd }.max { $0.updatedAt < $1.updatedAt }
        .flatMap { views[$0.id]?.controls }
      if let kept, !kept.options.isEmpty { return kept }
      return SessionControls(options: hostControls[key]?.options ?? [])
    }
    let latest = summaries.filter { $0.agent == agent }.max { $0.updatedAt < $1.updatedAt }
    if let kept = latest.flatMap({ views[$0.id]?.controls }), !kept.options.isEmpty { return kept }
    if let options = agentControls[agent], !options.isEmpty { return SessionControls(options: options) }
    let own = views.values.filter { $0.agent == agent && $0.id.hasPrefix("fx-") && !$0.controls.options.isEmpty }
      .min { $0.id < $1.id }?.controls
    let fallback = link == nil ? own ?? Fixtures.session("showcase")?.controls : nil
    return latest.flatMap { views[$0.id]?.controls } ?? fallback ?? SessionControls()
  }

  /// The slash commands a new session of `agent` offers before it runs: the engine's answer, else a kept view's of the
  /// agent (the demo's fixtures among them)
  func commands(for agent: AgentId, cwd: String = "") -> [SlashCommand] {
    if let key = hostControlsKey(agent, cwd) { return hostControls[key]?.commands ?? [] }
    if let known = agentCommands[agent], !known.isEmpty { return known }
    let latest = summaries.filter { $0.agent == agent }.sorted { $0.updatedAt > $1.updatedAt }
    return latest.lazy.compactMap { self.views[$0.id]?.commands }.first { !$0.isEmpty }
      ?? views.values.filter { $0.agent == agent && !$0.commands.isEmpty }.min { $0.id < $1.id }?.commands ?? []
  }

  /// The per-model parameters a model pick on the new session page reshapes with: the engine's answer, else those a
  /// kept view of the agent carries
  func shapes(for agent: AgentId, cwd: String = "") -> ModelShapes? {
    if let key = hostControlsKey(agent, cwd) { return hostControls[key]?.shapes }
    return agentShapes[agent] ?? views.values.first { $0.agent == agent && $0.modelShapes != nil }?.modelShapes
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
    // The demo's stand-in for the engine's new chat folder: its day and a random tail (projects.rs new_chat_folder)
    let chat = startsChat(cwd)
    let cwd = chat ? "\(chatRoot ?? "")/\(stamp.prefix(10))-\(UUID().uuidString.prefix(6).lowercased())" : cwd
    let account = accounts.first { $0.agent == agent }?.id
    let view = SessionView(id: id, agent: agent, title: title(from: text), cwd: cwd, accountId: account,
                           controls: controls, createdAt: stamp, updatedAt: stamp)
    views[id] = view
    summaries.insert(SessionSummary(id: id, title: view.title, agent: agent, cwd: cwd, updatedAt: stamp, accountId: account), at: 0)
    if !chat { addProject(cwd) }
    startTurn(id, text: text, attachments: attachments)
    return id
  }

  /// The first line of the first prompt, as the webview titles a session before the agent names it
  func title(from text: String) -> String {
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
      // The engine's copy names the stored blob: its thumbnail keeps the photo already on the phone
      media.adopt(attachments, session: id)
      let steer = steering(id)
      // Stamped here, so the bubble knows the clientId it can confirm once the engine shows the prompt
      let message = WebviewMsg.send(sessionId: id, text: trimmed, attachments: drafts.isEmpty ? nil : drafts, steer: steer ? true : nil).stamped()
      post(message)
      // Reads as sent until the engine shows it, a steer as already in the running turn; queued behind a running turn
      // without steering, the engine's queue row already says where it is
      if let view = views[id], !view.running || steer {
        outgoing[id, default: []].append(Outgoing(text: trimmed, attachments: attachments, after: view, steer: steer, clientId: message.clientId))
        if !view.running { sendingSummary(view, text: trimmed) }
      }
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

  /// Appends the prompt and streams the demo reply after it; `edited` marks a rewritten prompt (AppModel+Turns.swift)
  func startTurn(_ id: String, text: String, attachments: [Attachment], edited: Bool = false) {
    guard var view = views[id] else { return }
    var prompt = UserTurn(text: text, attachments: attachments.isEmpty ? nil : attachments)
    if edited { prompt.edited = true }
    view.turns.append(.user(prompt))
    view.running = false
    var full = view
    full.turns.append(.agent(DemoReply.turn(for: text, cwd: view.cwd, at: now)))
    let steps = Replay.steps(full)
    // The prompt and the empty running turn land at once, so a second prompt sent right after queues
    if let first = steps.first { apply(first, to: id) }
    play(id, Array(steps.dropFirst()))
  }

  /// Plays a turn's steps into the session, then lets the queue send its next prompt
  func play(_ id: String, _ steps: [SessionView], delay: Double = 0) {
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
    EntranceLedger.clear(prefix: "\(id):")
    // The replayed turn's clock starts with the stream, a second from now
    let steps = Replay.steps(view, startedAt: now.addingTimeInterval(1).timeIntervalSince1970 * 1000)
    if let first = steps.first { apply(first, to: id) }
    play(id, Array(steps.dropFirst()), delay: 1)
  }

  /// A stream step carries the transcript; the queue and controls stay as the reader left them
  func apply(_ step: SessionView, to id: String) {
    guard var view = views[id] else { return }
    liveSessions.insert(id)
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
  func touch(_ id: String, state: SessionState?) {
    guard let at = summaries.firstIndex(where: { $0.id == id }) else { return }
    summaries[at].state = state
    summaries[at].updatedAt = now.iso
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

  /// Pushes a screen onto the stack unless it (or any new-session screen) already sits on top, so a double-tap during
  /// the push transition never opens two pages
  func push(_ route: Route) {
    if path.last == route { return }
    if case .new = route, case .new? = path.last { return }
    path.append(route)
  }

  /// "Continue in a new session": the new-session screen preset to this agent and project, as the bar's new button
  func continueNew(_ id: String) {
    guard let view = views[id] else { return }
    push(.new(agent: view.agent, cwd: view.cwd))
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

  /// Archived sessions leave the list (its view menu's "archived" shows them); the toast's undo brings one back
  func setArchived(_ id: String, _ archived: Bool) {
    if !post(.archiveSession(id: id, archived: archived)), let at = summaries.firstIndex(where: { $0.id == id }) {
      summaries[at].archived = archived ? true : nil
    }
    guard archived else { return }
    let title = views[id]?.title ?? summaries.first { $0.id == id }?.title ?? ""
    show(t("session.archived", ["title": title]), icon: .archive,
         action: ToastAction(label: t("common.undo")) { [weak self] in self?.setArchived(id, false) })
  }

  /// The row menu's "mark as unread / read"
  func setUnread(_ id: String, _ unread: Bool) {
    if post(.setSessionUnread(id: id, unread: unread)) { return }
    guard let at = summaries.firstIndex(where: { $0.id == id }) else { return }
    if summaries[at].state == .background {
      summaries[at].unread = unread ? true : nil
    } else if unread, summaries[at].state == nil {
      summaries[at].state = .unread
    } else if !unread, summaries[at].state == .unread {
      summaries[at].state = nil
    }
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

  /// Fork from a turn: a new session holding the transcript up to and including that turn, opened over this one.
  /// `index` is the turn's place in the session, as the thread's rows carry it (`threadItems`) and forkSession counts it
  func fork(_ id: String, through index: Int) {
    guard let source = views[id] else { return }
    let turn = index - source.turnBase
    guard source.turns.indices.contains(turn) else { return }
    if link != nil {
      engineFork(id, through: index)
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

  /// Writes the export to a temporary file and hands it to the share sheet. The Markdown and the plain text are
  /// `sessionMarkdown`'s (with an engine, the desktop's own exports, so a list row's session the phone never opened
  /// exports too); the JSON is the view the phone holds, offered only while that is the whole session (`holdsWholeView`)
  func export(_ id: String, format: ExportFormat) {
    Task {
      let title = views[id]?.title ?? summaries.first { $0.id == id }?.title ?? ""
      let body = format == .json ? views[id].map(SessionExport.json) : await sessionMarkdown(id, format: format)
      guard let body, !body.isEmpty else {
        show(t("mobile.engineNeeded"), icon: .triangleAlert)
        return
      }
      let ext = switch format { case .json: "json"; case .text: "txt"; default: "md" }
      let url = FileManager.default.temporaryDirectory
        .appendingPathComponent("\(SessionExport.fileName(title)).\(ext)")
      do {
        try Data(body.utf8).write(to: url, options: .atomic)
        shared = ExportFile(url: url)
      } catch {
        show(error.localizedDescription, icon: .triangleAlert)
      }
    }
  }

  /// The phone holds the session's view (a JSON export needs it). Read from the storage behind `views` without observing
  /// it: a list row's menu asks, and observing `views` there would redraw every row on each streamed chunk
  func holdsView(_ id: String) -> Bool {
    _views[id] != nil
  }

  /// The phone holds the session's view from its first turn: a JSON export is that view, and one that starts at the
  /// session's tail (`SessionView.earlier`) would pass for the session. Unobserved, as `holdsView`
  func holdsWholeView(_ id: String) -> Bool {
    _views[id].map { $0.earlier == nil } ?? false
  }
}

/// A file waiting for the share sheet
struct ExportFile: Identifiable, Equatable {
  let id = UUID()
  var url: URL
}

// MARK: - Session actions in the environment

/// What the controls inside a chat call, bound to that chat's session. Defaults do nothing, so the full-length render
/// and previews draw the same views without a model. A screen makes its set once (`Memo`): a fresh set of closures
/// never compares equal, and handing one out on every rebuild redrew every visible view reading it
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
  /// Takes the sent prompt at that turn index into the composer for rewriting; send then goes out as `editTurn`
  var editPrompt: (Int) -> Void = { _ in }
  var sendEdit: (String, [Attachment]) -> Void = { _, _ in }
  var cancelEdit: () -> Void = {}
  /// A goal control (`set` carries the objective)
  var goal: (GoalAction, String?) -> Void = { _, _ in }
  /// Asks the agent to stop a background task by its id
  var stopTask: (String) -> Void = { _ in }
  /// Ends one running command by its row's id; rows offer it under `canStopCommands`
  var stopCommand: (String) -> Void = { _ in }
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
    .glassSurface(toast.action == nil ? .regular : .regular.interactive(), in: Capsule())
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
