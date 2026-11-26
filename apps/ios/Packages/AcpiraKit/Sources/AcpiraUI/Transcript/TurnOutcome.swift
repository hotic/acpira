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

/// hasTurnContent: copy, fork and stats need content beyond notices, goal rows and compaction status
func hasTurnContent(_ turn: AgentTurn) -> Bool {
  turn.blocks.contains { block in
    switch block { case .notice, .goal, .compaction: false; default: true }
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

/// Whether a user prompt or turn receipt names `/compact` (Shell.tsx `compacting`)
func isCompactTurn(_ turn: AgentTurn, prompt: UserTurn? = nil) -> Bool {
  if turn.command?.name == "compact" { return true }
  guard let text = prompt?.text.trimmingCharacters(in: .whitespacesAndNewlines) else { return false }
  return text == "/compact" || text.hasPrefix("/compact ") || text.hasPrefix("/compact\n") || text.hasPrefix("/compact\t")
}

/// Presentation-only mirror of `compactionDisplay.ts`: covers a running `/compact` before any `compaction_update`
/// lands (including peers like agy that emit nothing until `end_turn`) and legacy prose replies, so the turn renders
/// a single live "Compacting context" row instead of a generic "Working" head
func compactionForDisplay(_ turn: AgentTurn, running: Bool) -> AgentTurn {
  let text = turn.blocks.compactMap { if case .text(let b) = $0 { b.markdown } else { nil } }
    .joined().trimmingCharacters(in: .whitespacesAndNewlines)
  let structured = turn.blocks.compactMap { if case .compaction(let b) = $0 { b } else { nil } }
  let prefixes = [
    "Compacting context", "Context compaction started", "Compaction started",
    "Context compacted", "Compaction completed.", "Nothing to compact.",
  ]
  let known = text.firstMatch(of: CompactionPattern.known) != nil
    || (running && !text.isEmpty && prefixes.contains { $0.hasPrefix(text) })
  guard known || !structured.isEmpty || (running && turn.blocks.isEmpty) else { return turn }

  var status: CompactionStatus = structured.last?.status ?? .in_progress
  if structured.isEmpty {
    if text.firstMatch(of: CompactionPattern.failed) != nil { status = .failed }
    else if text.firstMatch(of: CompactionPattern.cancelled) != nil { status = .cancelled }
    else if text.firstMatch(of: CompactionPattern.completed) != nil { status = .completed }
    else if !running && turn.stop != .error && turn.stop != .cancelled { return turn }
  }
  if turn.stop == .error { status = .failed }
  if turn.stop == .cancelled { status = .cancelled }
  let error = status == .failed ? structured.last?.error : nil
  let block = CompactionBlock(structured.first?.id ?? "compact-display", status: status, error: error)
  var blocks = turn.blocks.filter { b in
    switch b {
    case .compaction: false
    case .text: !known
    default: true
    }
  }
  if known && status == .failed && !text.isEmpty { blocks.append(.text(TextBlock(markdown: text))) }
  let trailing: Bool = if case .compaction = turn.blocks.last { !structured.isEmpty && !blocks.isEmpty } else { false }
  var out = turn
  out.blocks = trailing ? blocks + [.compaction(block)] : [.compaction(block)] + blocks
  return out
}

private enum CompactionPattern {
  nonisolated(unsafe) static let known = /^(?:Compacting context(?:\.{3}|…|$)|Context compaction started\b|Compaction started\b|Context compacted\b|Compaction completed\.|Nothing to compact\.|(?:Force compaction|Compaction) failed:|\/compact failed:|Compaction cancel(?:ed|led)\.|Compaction is blocked by the current turn;)/
  nonisolated(unsafe) static let failed = /(?:Force compaction|Compaction) failed:|\/compact failed:|Compaction is blocked by the current turn;/
  nonisolated(unsafe) static let cancelled = /Compaction cancel(?:ed|led)\./
  nonisolated(unsafe) static let completed = /Context compacted|Compaction completed\.|Nothing to compact\./
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
  return error.message.firstMatch(of: ContextLengthPattern.message) != nil
}

/// Compiled once: a regex literal is compiled again each time it is evaluated
private enum ContextLengthPattern {
  nonisolated(unsafe) static let message = /(?i)\bprompt (?:to the model )?(?:was|is) too long\b|\bcontext (?:length|window) (?:exceeded|limit exceeded)\b|\bmaximum context length\b/
}
