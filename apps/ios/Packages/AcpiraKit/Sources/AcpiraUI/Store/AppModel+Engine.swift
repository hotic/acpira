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
      show("Engine message not understood", icon: .triangleAlert)
    case .closed(let reason):
      EngineLink.log.notice("closed \(reason ?? "", privacy: .public)")
      show(reason.map { "Engine disconnected: \($0)" } ?? "Engine disconnected", icon: .unplug)
    }
  }

  private func receive(_ message: HostMsg) {
    guard let link else { return }
    switch message {
    case .initState(let state):
      if let base = state.blobBase { link.blobBase = base }
      agents = state.agents
      accounts = state.accounts
      summaries = state.sessions
      present(link.feed.receive(message))
    case .agents(let list):
      agents = list
    case .sessions(let list):
      summaries = list
    case .accounts(let list):
      accounts = list
    case .session, .sessionPatch, .noSession:
      present(link.feed.receive(message))
    case .terminalOutput, .terminalState:
      terminals.receive(message)
    case .imageBlob(let seq, let blob):
      link.staged(seq, blob: blob)
    default:
      break
    }
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

  /// The engine makes the session and shows it on this view; the first prompt follows, and its chat replaces the
  /// new-session screen
  /// agent: nil starts the engine's default agent
  func engineNewSession(agent: AgentId?, cwd: String, text: String) {
    guard let link else { return }
    let prompt = text.trimmingCharacters(in: .whitespacesAndNewlines)
    link.awaiting = (from: link.feed.views.current?.id, then: { [weak self] id in
      guard let self else { return }
      if !prompt.isEmpty { self.post(.send(sessionId: id, text: prompt)) }
      if case .new? = self.path.last { self.path[self.path.count - 1] = .chat(id) } else { self.path.append(.chat(id)) }
    })
    post(.newSession(agent: agent, cwd: cwd.isEmpty ? nil : cwd))
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
