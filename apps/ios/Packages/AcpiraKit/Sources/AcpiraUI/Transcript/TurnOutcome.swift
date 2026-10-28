import AcpiraModels
import Foundation

// The rest of chat/turnOutcome.ts (turnOutcome itself is in Logic/Folding.swift), shared/turnErrors.ts, and the
// display rules Turns.tsx applies to a turn before drawing it.

/// absorbedNotices: a turn that ended on its own prompt error (no AIR failure id behind it) already names the cause, and
/// the failure and session cards own the remedy. Its error notices are earlier symptoms of the same failure, so they
/// join the closing row as detail lines instead of standing as warnings with remedies of their own
func absorbedNotices(_ turn: AgentTurn) -> [NoticeBlock] {
  guard turn.stop == .error, let error = turn.error, error.failureId == nil else { return [] }
  return turn.blocks.compactMap { block in if case .notice(let n) = block, n.severity == "error" { n } else { nil } }
}

/// hasTurnContent: copy, fork and stats only mean something for a turn that produced more than notices and goal rows
func hasTurnContent(_ turn: AgentTurn) -> Bool {
  turn.blocks.contains { block in
    switch block { case .notice, .goal: false; default: true }
  }
}

/// The action row under a settled turn; a reply that failed before producing anything still offers the fork, so
/// another agent can take the message over
func showsTurnActions(_ turn: AgentTurn, running: Bool) -> Bool {
  !running && (hasTurnContent(turn) || turn.stop == .error)
}

/// revisedPlanEnd: Revise on a plan card ends the turn cancelled (claude-agent-acp 0.93.0) with nothing else to show.
/// The card already says the plan awaits changes, so that end reads as a normal one: no "stopped" row or stop glyph.
/// A stop pressed by hand leaves the plan unrejected
func revisedPlanEnd(_ turn: AgentTurn) -> Bool {
  turn.stop == .cancelled && turn.blocks.contains { block in
    if case .planDocument(let plan) = block { plan.status == "rejected" } else { false }
  }
}

/// The turn as the message draws it: a Revise end reads as settled, and absorbed notices leave the block list (they
/// render under the closing row), so an empty group cannot leave a stray gap
func displayTurn(_ turn: AgentTurn, absorbed: [NoticeBlock]) -> AgentTurn {
  var shown = turn
  if revisedPlanEnd(turn) { shown.stop = nil }
  if !absorbed.isEmpty {
    let ids = Set(absorbed.map(\.id))
    shown.blocks.removeAll { block in if case .notice(let n) = block { ids.contains(n.id) } else { false } }
  }
  return shown
}

/// A call whose observation was lost must not pass for running work (ToolCall.tsx, useToolSeconds.ts): it shows
/// settled, so nothing shimmers or ticks and an open command card does not follow its output, and `toolVerb` says its
/// result is unconfirmed
func observedBlock(_ block: AgentBlock) -> AgentBlock {
  guard case .toolCall(var call) = block, call.observation == .unknown,
        call.status == .pending || call.status == .in_progress else { return block }
  call.status = .cancelled
  return .toolCall(call)
}

/// isContextLengthError: the input cannot fit, whatever the provider's retryable flag says
func isContextLengthError(_ error: TurnError?) -> Bool {
  guard let error else { return false }
  if ["context_length_exceeded", "context_window_exceeded", "prompt_too_long"].contains(error.kind ?? "") { return true }
  let pattern = /(?i)\bprompt (?:to the model )?(?:was|is) too long\b|\bcontext (?:length|window) (?:exceeded|limit exceeded)\b|\bmaximum context length\b/
  return error.message.firstMatch(of: pattern) != nil
}
