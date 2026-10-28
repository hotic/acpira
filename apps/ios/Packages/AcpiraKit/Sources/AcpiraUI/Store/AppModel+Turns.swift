import AcpiraDesign
import AcpiraModels
import Foundation

// What the transcript's own actions do: rewriting a sent prompt (HistoryMessage.tsx and App.tsx `editTurn`), the
// session goal's controls (Goal.tsx, `goal`) and stopping a background task (ToolCall.tsx, `stopAsyncTask`). With an
// engine each posts its message; the demo plays the engine's answer locally.

extension AppModel {
  // MARK: - Prompt edits

  /// Takes the sent prompt at `index` into the chat's composer: its text and attachments, and the selections it went
  /// out with as the composer's local snapshot. What the composer held waits until the edit is sent or dropped
  func beginEdit(_ id: String, turn index: Int) {
    guard let view = views[id], canEditPrompts(view), view.turns.indices.contains(index),
          case .user(let prompt) = view.turns[index], prompt.auto != true else { return }
    let draft = draft(for: id)
    // Taking in another prompt keeps the draft from before the first
    let kept = draft.edit.map { ($0.keptText, $0.keptStaged) } ?? (draft.text, draft.staged)
    draft.edit = TurnEdit(turnIndex: index, turnCount: view.turns.count, original: prompt,
                          controls: openTurnControls(view.controls, prompt.settings, shapes: view.modelShapes),
                          keptText: kept.0, keptStaged: kept.1)
    draft.text = prompt.text
    draft.staged = prompt.attachments ?? []
  }

  /// `-popup edit`: the newest prompt the user typed, taken in for editing (screenshots)
  func beginEditLast(_ id: String) {
    guard let turns = views[id]?.turns,
          let at = turns.lastIndex(where: { if case .user(let u) = $0 { u.auto != true } else { false } }) else { return }
    beginEdit(id, turn: at)
  }

  /// The edit dropped: the composer gets back what it held
  func cancelEdit(_ id: String) {
    let draft = draft(for: id)
    guard let edit = draft.edit else { return }
    draft.edit = nil
    draft.text = edit.keptText
    draft.staged = edit.keptStaged
  }

  /// Send from the editing composer: the turn and everything after it give way to the rewrite. The engine answers with
  /// `editTurnResult`; until then the composer stays empty, and a refusal puts the rewrite back. The demo drops the
  /// later turns and replays its reply, as a send does
  func sendEdit(_ id: String, text: String, attachments: [Attachment]) {
    let draft = draft(for: id)
    guard let edit = draft.edit else {
      send(id, text: text, attachments: attachments)
      return
    }
    draft.edit = nil
    if link != nil {
      let request = UUID().uuidString
      draft.pending = (request, edit, text, attachments)
      post(.editTurn(requestId: request, edit: editTurnRequest(sessionId: id, edit: edit, text: text, staged: attachments)))
      return
    }
    guard var view = views[id], edit.turnIndex <= view.turns.count else { return }
    view.turns = Array(view.turns.prefix(edit.turnIndex))
    // The engine applies the edit's selections to the live session before the prompt goes out
    view.controls = edit.controls
    views[id] = view
    startTurn(id, text: text.trimmingCharacters(in: .whitespacesAndNewlines), attachments: attachments, edited: true)
    // The composer clears itself after this returns, so the earlier draft comes back on the next pass
    Task { @MainActor [weak self] in self?.restoreKept(id, edit) }
  }

  /// The engine's answer to an edit: a refusal names why in a toast and puts the rewrite back (drafts survive a
  /// rejected edit), an accepted one gives the composer its earlier draft. Either only into an untouched composer
  func editAnswered(_ request: String, error: String?) {
    guard let match = drafts.first(where: { $0.value.pending?.request == request }),
          let pending = match.value.pending else { return }
    let (id, draft) = (match.key, match.value)
    draft.pending = nil
    if let error {
      show(error, icon: .triangleAlert)
      guard draft.text.isEmpty && draft.staged.isEmpty && draft.edit == nil else { return }
      draft.edit = pending.edit
      draft.text = pending.text
      draft.staged = pending.staged
    } else {
      restoreKept(id, pending.edit)
    }
  }

  private func restoreKept(_ id: String, _ edit: TurnEdit) {
    let draft = draft(for: id)
    guard draft.text.isEmpty && draft.staged.isEmpty && draft.edit == nil else { return }
    draft.text = edit.keptText
    draft.staged = edit.keptStaged
  }

  // MARK: - Goal

  /// A goal control. The engine sends the turn-starting ones as a `/goal …` prompt; the demo plays the agent's new
  /// snapshot and its milestone row on the last agent turn (normalize.rs apply_goal)
  func goal(_ id: String, _ action: GoalAction, objective: String? = nil) {
    if post(.goal(sessionId: id, action: action, objective: objective)) { return }
    guard var view = views[id] else { return }
    var goal = view.goal
    let event: GoalEvent
    switch action {
    case .set:
      guard let objective = objective?.trimmingCharacters(in: .whitespacesAndNewlines), !objective.isEmpty else { return }
      goal = SessionGoal(objective: objective, status: .active, timeUsedSeconds: 0)
      event = .set
    case .pause: goal?.status = .paused; event = .paused
    case .resume: goal?.status = .active; event = .resumed
    case .clear: event = .cleared
    case .unknown: return
    }
    let snapshot = goal ?? view.goal
    let row = AgentBlock.goal(GoalBlock(event: event, objective: snapshot?.objective, iterations: snapshot?.iterations,
                                        tokensUsed: snapshot?.tokensUsed, timeUsedSeconds: snapshot?.timeUsedSeconds))
    if case .agent(var last)? = view.turns.last {
      last.blocks.append(row)
      view.turns[view.turns.count - 1] = .agent(last)
    } else {
      view.turns.append(.agent(AgentTurn(blocks: [row], stop: .end_turn)))
    }
    view.goal = action == .clear ? nil : goal
    views[id] = view
  }

  // MARK: - Background tasks

  /// Stop on a background task's row. The engine marks the stop asked at once and the adapter's state update settles
  /// the row; a refusal comes back as the engine's error toast with the row live again. The demo stops it a beat later
  func stopTask(_ id: String, task: String) {
    if post(.stopAsyncTask(sessionId: id, taskId: task)) { return }
    updateTask(id, task) { $0.stopRequested = true }
    Task { @MainActor [weak self] in
      // Long enough to read "Stopping…"
      try? await Task.sleep(for: .seconds(1))
      self?.updateTask(id, task) { info in
        info.stopRequested = nil
        info.state = "stopped"
      }
    }
  }

  /// Rewrites the task on every row that carries it; a stopped task's row reads cancelled
  private func updateTask(_ id: String, _ task: String, _ change: (inout AsyncTaskInfo) -> Void) {
    guard var view = views[id] else { return }
    view.turns = view.turns.map { turn in
      guard case .agent(var agent) = turn else { return turn }
      agent.blocks = agent.blocks.map { block in
        guard case .toolCall(var call) = block, var info = call.asyncTask, info.id == task else { return block }
        change(&info)
        call.asyncTask = info
        if info.state == "stopped" { call.status = .cancelled }
        return .toolCall(call)
      }
      return .agent(agent)
    }
    views[id] = view
  }
}
