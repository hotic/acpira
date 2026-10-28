import AcpiraDesign
import AcpiraModels
import AcpiraSync
import Foundation
import os

// AppModel against a live engine: the pushes it applies and the actions it posts. Mirrors what packages/ui/App.tsx does
// with the same messages; the demo's local stand-ins stay in AppModel.swift.

extension AppModel {
  var live: Bool { link != nil }

  /// The loader the chat's images ask: the engine's, else the demo's (fixture blobs from the bundle)
  var media: MediaLoader { link?.media ?? .demo }

  /// Posts an action to the engine; false in the demo, where the caller acts locally
  @discardableResult
  func post(_ message: WebviewMsg) -> Bool {
    guard let link else { return false }
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
      show(t(engine < SidecarProtocol.version ? "mobile.engine.updateComputer" : "mobile.engine.updateApp"), icon: .triangleAlert)
    case .closed(let reason):
      EngineLink.log.notice("closed \(reason ?? "", privacy: .public)")
      // A toast, unless the version notice already said why
      if !incompatible { show(t("mobile.engine.disconnected"), icon: .unplug) }
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
      summaries = state.sessions
      if let categories = state.categories { follow(categories) }
      present(link.feed.receive(message))
    case .agents(let list):
      agents = list
    case .sessions(let list):
      summaries = list
    case .categories(let categories):
      follow(categories)
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

  /// A chat came on screen: its kept view at once, and the engine switches the view to it
  func open(_ id: String) {
    guard let link else { return }
    if let kept = link.feed.select(id) { views[id] = kept }
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
    post(.newSession(agent: agent, cwd: cwd.isEmpty ? nil : cwd))
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
