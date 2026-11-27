import AcpiraDesign
import AcpiraFixtures
import AcpiraModels
import AcpiraSync
import Foundation
import Observation
import SwiftUI

// What the transcript's cards and the dock do to a session: permission and question cards, the queue, the short
// stops' retry / continue / compact, and building or revising a plan

extension AppModel {
  // MARK: - Cards in the transcript

  /// A permission answered: the card leaves the transcript, as the engine drops it once the agent has the outcome
  func choosePermission(_ id: String, block: String, option: String) {
    if post(.permission(sessionId: id, blockId: block, optionId: option)) { return }
    if settleChildCard(id, block: block) { return }
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
    if settleChildCard(id, block: question) { return }
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
      case let .text(blob, name, context):
        guard let text = PastedText.inline(blob) else { return nil }
        return .text(name: name, text: text, context: context)
      case let .file(uri, name):
        guard let url = URL(string: uri), AttachmentFile.isCopy(url) else { return .file(uri: uri, name: name) }
        guard let bytes = try? Data(contentsOf: url) else { return nil }
        // The tile of the sent prompt finds the video under the engine's blob name without fetching it back
        if Attachment.isVideo(name: name) { VideoCache.adopt(bytes, name: name) }
        return .file(uri: uri, name: name, data: bytes.base64EncodedString())
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

  /// Edit: takes a prompt out of the queue for the composer. The engine's queue row names the blobs it stored, which a
  /// send cannot hand back (`drafts` takes local bytes only), so they are read back first; while one cannot be read the
  /// prompt stays queued and a toast says so. nil: nothing to edit (also when the prompt went out during the read)
  func editQueued(_ id: String, item: String) async -> QueuedPrompt? {
    guard var prompt = views[id]?.queued?.first(where: { $0.id == item }) else { return nil }
    if let attachments = prompt.attachments {
      let media = media
      do {
        prompt.attachments = try await Self.localCopies(attachments) { try await media.bytes(session: id, blob: $0) }
      } catch {
        show(t("attach.loadFailed", ["name": Self.failedName(attachments)]), icon: .triangleAlert)
        return nil
      }
    }
    return takeQueued(id, item) == nil ? nil : prompt
  }

  /// The attachments with every stored text or photo replaced by its bytes, base64 like a fresh paste or pick
  static func localCopies(_ attachments: [Attachment], read: (String) async throws -> Data) async throws -> [Attachment] {
    var out: [Attachment] = []
    for a in attachments {
      switch a {
      case let .text(blob?, name, context) where Self.isStored(blob):
        out.append(.text(blob: try await read(blob).base64EncodedString(), name: name, context: context))
      case let .image(blob?, mimeType, name) where Self.isStored(blob):
        out.append(.image(blob: try await read(blob).base64EncodedString(), mimeType: mimeType, name: name))
      default: out.append(a)
      }
    }
    return out
  }

  /// A blob the engine stored is named `<hash>.<ext>`; base64 never holds a dot (`PastedText.inline`)
  private static func isStored(_ blob: String) -> Bool { blob.contains(".") }

  /// The attachment the failure toast names: the first one still stored
  private static func failedName(_ attachments: [Attachment]) -> String {
    for a in attachments {
      if case let .text(blob?, name, _) = a, isStored(blob) { return name }
      if case let .image(blob?, _, name) = a, isStored(blob) { return name ?? t("common.image") }
    }
    return ""
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
}
