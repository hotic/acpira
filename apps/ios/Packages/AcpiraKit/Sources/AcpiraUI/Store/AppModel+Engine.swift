import AcpiraDesign
import AcpiraModels
import AcpiraSync
import Foundation
import os

// AppModel against a live engine: the pushes it applies and the actions it posts. Mirrors what packages/ui/App.tsx does
// with the same messages; the demo's local stand-ins stay in AppModel.swift.

extension AppModel {
  var live: Bool { link != nil }

  /// What the screens show is the engine's own, pushed over a wire that is up: false from a wire's end until the next
  /// one's init replaces what the screens carried over (the demo is always current)
  var current: Bool { link == nil || (!ended && (!resumed || synced)) }

  /// The loader the chat's images ask: the engine's, else the demo's (fixture blobs from the bundle)
  var media: MediaLoader { link?.media ?? .demo }

  /// Posts an action to the engine; false in the demo, where the caller acts locally
  @discardableResult
  func post(_ message: WebviewMsg) -> Bool {
    guard let link else { return false }
    // A resumed wire's view shows no session until its init and the reopen after it, and the engine refuses a session
    // action before that: it waits in the replay
    if resumed, !synced, message.replayable {
      replay.append(message.stamped())
      return true
    }
    link.client.post(message)
    return true
  }

  /// Says ready (queued until the view is attached) and applies every event until the wire ends
  func connect() {
    guard let link else { return }
    link.client.post(.ready(patches: true))
    Task { [weak self] in
      await link.client.start()
      for await event in link.client.events {
        guard let self else { return }
        self.receive(event)
      }
    }
  }

  private func receive(_ event: EngineEvent) {
    switch event {
    case .connected:
      break
    case .host(let message):
      receive(message)
    case .platform(.toast(let level, let text, _)):
      show(text, icon: level == "error" ? .triangleAlert : level == "warning" ? .circleAlert : .info)
    case .platform:
      break
    case .dropped(let reason):
      EngineLink.log.error("dropped \(reason, privacy: .public)")
      show(t("mobile.engine.dropped"), icon: .triangleAlert)
    case .incompatible(let engine):
      // The older side is the one to update
      incompatible = true
      if onClosed == nil {
        show(t(engine < SidecarProtocol.version ? "mobile.engine.updateComputer" : "mobile.engine.updateApp"), icon: .triangleAlert)
      }
    case .closed(let reason):
      EngineLink.log.notice("closed \(reason ?? "", privacy: .public)")
      ended = true
      // A paired computer's wire has its own screens for an ended connection; the development engine gets a toast,
      // unless the version notice already said why
      if let onClosed { onClosed() } else if !incompatible {
        show(t("mobile.engine.disconnected"), icon: .unplug)
      }
    }
  }

  private func receive(_ message: HostMsg) {
    guard let link else { return }
    switch message {
    case .initState(let state):
      if let base = state.blobBase { link.blobBase = base }
      agents = state.agents
      accounts = state.accounts
      settings = state.settings
      summaries = state.sessions.map { pendingSummary($0, fresh: resumed) }
      if let categories = state.categories { follow(categories) }
      present(link.feed.receive(message))
      synced = true
      // The hosts' state for the host folders; the engine pushes it again on every change
      post(.connections)
      if resumed {
        reopen()
        flushReplay()
      }
    case .agents(let list):
      agents = list
    case .sessions(let list):
      summaries = list.map { pendingSummary($0) }
    case .categories(let categories):
      follow(categories)
    case .connections(let hosts):
      connections = hosts
    case .accounts(let list):
      accounts = list
    case .settings(let next, _):
      settings = next
    case let .controls(agent, controls, _):
      agentControls[agent] = controls
    case .accountActions(let actions):
      report(actions)
    case let .sessionHits(seq, list):
      // A reply to an older query is dropped rather than mixed in
      guard seq == searchSeq else { break }
      hits = SessionHits(query: searchQuery, snippets: Dictionary(list.map { ($0.id, $0.snippet) }, uniquingKeysWith: { a, _ in a }))
    case let .projectFile(seq, _, _, _), let .sessionChanges(seq, _), let .diffSource(seq, _), let .sessionMarkdown(seq, _, _):
      answer(seq, message)
    case .session, .sessionPatch, .noSession:
      present(link.feed.receive(message))
    case .terminalOutput, .terminalState:
      terminals.receive(message)
    case .imageBlob(let seq, let blob):
      link.staged(seq, blob: blob)
    case let .files(seq, files):
      fileSearch.receive(seq: seq, files: files)
    case let .editTurnResult(requestId, error):
      editAnswered(requestId, error: error)
    case let .subagent(sessionId, subagentId, rev, running, turns):
      subagentFeed.receive(session: sessionId, id: subagentId, transcript: SubagentTranscript(rev: rev, running: running, turns: turns))
    default:
      break
    }
  }

  /// The project order and sections every project shell on the machine shares
  private func follow(_ categories: SessionCategories) {
    projectOrder = categories.projects ?? []
    removedProjects = categories.removedProjects ?? []
    sections = categories.sections ?? []
  }

  private func present(_ outcome: SessionFeed.Outcome) {
    guard let link else { return }
    switch outcome {
    case .show(let view?):
      // Only the views the feed keeps stay, so a long transcript is not held past its cache bound
      let kept = Set(link.feed.views.kept)
      views = views.filter { kept.contains($0.key) }
      views[view.id] = view
      // A prompt still on its way gives up its place once the session shows the engine's copy; the ones sent after it
      // keep theirs until their own copies arrive
      if let sent = outgoing[view.id] {
        let left = sent.left(after: view)
        if left != sent { outgoing[view.id] = left.isEmpty ? nil : left }
      }
      settleSummary(view)
      if let awaiting = link.awaiting, view.id != awaiting.from {
        link.awaiting = nil
        awaiting.then(view.id)
      }
    case .show(nil), .unchanged:
      break
    case .resync:
      link.client.post(.resync)
    }
  }

  /// A new wire to the computer the screens follow (coming back from the background, a retry): they stay where they
  /// were, showing the last lists and views until this wire's init replaces them, and the drafts stay too. Before, a
  /// fresh model popped every return to an empty list until init landed. A terminal's page goes, since its shell was
  /// attached to the old wire
  func resume(from previous: AppModel) {
    agents = previous.agents
    accounts = previous.accounts
    settings = previous.settings
    agentControls = previous.agentControls
    summaries = previous.summaries
    views = previous.views
    // The replay below sends them again, so they keep reading as sent, and this wire's loader keeps their photos too
    outgoing = previous.outgoing
    pendingSummaries = previous.pendingSummaries
    for (id, sent) in outgoing { sent.forEach { media.adopt($0.attachments, session: id) } }
    path = Array(previous.path.prefix { if case .terminal = $0 { false } else { true } })
    addedProjects = previous.addedProjects
    projectOrder = previous.projectOrder
    removedProjects = previous.removedProjects
    sections = previous.sections
    hits = previous.hits
    openProjects = previous.openProjects
    settingsOpen = previous.settingsOpen
    shared = previous.shared
    drafts = previous.drafts
    // What the wire before may not have delivered (a prompt sent as the network moved), and anything it had still
    // waiting for its own init; the engine runs each clientId once, so a resend that did arrive changes nothing
    replay = previous.replay + (previous.link?.client.unsent() ?? [])
    away = previous.away
    resumed = true
  }

  /// After a resumed wire's init: its view starts on no session, so the one on screen is opened again (a chat opens
  /// itself on appear, which a screen carried over never repeats). In the background it waits for the way back
  private func reopen() {
    guard !away, let id = onScreenSession else { return }
    open(id)
  }

  /// The chat the stack shows, or the nearest one under the page on top
  private var onScreenSession: String? { path.reversed().lazy.compactMap(\.session).first }

  /// Sends the held session actions in order, after the reopen. An action for a session off screen switches the view
  /// to it first and back after, since the engine acts only on the session the view shows
  private func flushReplay() {
    let held = replay
    replay = []
    let shown = watching
    for message in held {
      guard let target = message.sessionTarget, target != shown else {
        post(message)
        continue
      }
      post(.selectSession(id: target))
      post(message)
      // Back to what the screen shows; with no chat on screen the view must not stay on the target as if watching it
      post(shown.map { .selectSession(id: $0) } ?? .deselectSession(id: target))
    }
  }

  /// The chat the engine's view should count as watched: the one on screen (or under the page on top), none while the
  /// list is on top or the app is in the background
  private var watchable: String? { away ? nil : onScreenSession }

  /// The stack changed or the app went to the background: a chat that left the screen is no longer watched from this
  /// view, so a turn ending there gets its unread dot. The view used to keep the last chat opened, under the list and in
  /// the background alike. A chat coming on screen selects itself (`open`); `back` selects the one on screen again on
  /// the way back from the background, whose view may have moved on meanwhile
  func syncWatching(back: Bool = false) {
    guard let link, !resumed || synced else { return }
    let want = watchable
    if let was = watching, was != want {
      watching = nil
      link.client.post(.deselectSession(id: was))
    }
    guard back, let want, watching == nil else { return }
    open(want)
    // The engine drops the observed child with its session: the inspector on screen follows it again
    if let again = subagentFeed.renewal(session: want) { link.client.post(again) }
  }

  /// The app went to the background (true) or came back
  func setAway(_ now: Bool) {
    guard now != away else { return }
    away = now
    syncWatching(back: !now)
  }

  /// The session's view came on this wire: a view carried over from the wire before waits for the engine to show it
  /// again (an observe sent before that is never answered)
  func shown(_ id: String) -> Bool {
    views[id] != nil && (link.map { $0.feed.views.kept.contains(id) } ?? true)
  }

  /// A chat came on screen: its kept view at once, and the engine switches the view to it
  func open(_ id: String) {
    guard let link else { return }
    if let kept = link.feed.select(id) { views[id] = kept }
    watching = id
    link.client.post(.selectSession(id: id))
  }

  /// The engine makes the session and shows it on this view; the controls picked on the new-session screen are set on
  /// it, the first prompt follows, and its chat replaces the new-session screen
  /// agent: nil starts the engine's default agent; `opened` runs once its chat is on screen (`-start` with `-subagent`)
  func engineNewSession(agent: AgentId?, cwd: String, controls: SessionControls? = nil, text: String,
                        attachments: [Attachment] = [], opened: (() -> Void)? = nil) {
    guard let link else { return }
    link.awaiting = (from: link.feed.views.current?.id, then: { [weak self] id in
      guard let self else { return }
      if let controls, let made = self.views[id]?.controls { self.apply(controls, over: made, to: id) }
      self.send(id, text: text, attachments: attachments)
      if case .new? = self.path.last { self.path[self.path.count - 1] = .chat(id) } else { self.path.append(.chat(id)) }
      opened?()
    })
    // A host folder under local run takes the built-in agent, whatever the page picked (also a folder picked after the
    // page opened) or the engine would default to
    let start = startAgent(agent ?? "", in: cwd)
    post(.newSession(agent: start.isEmpty ? nil : start, cwd: cwd.isEmpty ? nil : cwd))
  }

  /// The picks a new session's screen made where they differ from what the engine started it with
  private func apply(_ picked: SessionControls, over made: SessionControls, to id: String) {
    if let mode = picked.modeId, mode != made.modeId, made.modes.contains(where: { $0.id == mode }) {
      post(.setMode(sessionId: id, id: mode))
    }
    for option in picked.options {
      guard let value = option.value, let now = made.options.first(where: { $0.id == option.id }), now.value != value else { continue }
      post(.setConfig(sessionId: id, configId: option.id, value: value))
    }
  }

  /// An account import that ended without a login: a toast says why (the engine toasts a success itself). Only a change
  /// of state speaks, as every push repeats the actions the engine still keeps
  private func report(_ actions: [AccountAction]) {
    for action in actions where action.via == .import {
      let key = "\(action.agent)/\(action.via.rawValue)"
      guard actionStates.updateValue(action.status, forKey: key) != action.status else { continue }
      let name = agent(action.agent).name
      switch action.status {
      case .missing: show(t("mobile.account.missing", ["agent": name]), icon: .circleAlert)
      case .error: show(action.error ?? t("mobile.account.failed", ["agent": name]), icon: .triangleAlert)
      default: break
      }
    }
  }

  /// The engine forks and shows the fork on this view; its chat opens over the source
  func engineFork(_ id: String, through turn: Int) {
    guard let link else { return }
    link.awaiting = (from: id, then: { [weak self] fork in
      self?.path.append(.chat(fork))
      self?.show(t("mobile.forked"), icon: .gitBranch)
    })
    post(.forkSession(sessionId: id, turnIndex: turn))
  }
}
