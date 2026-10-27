import AcpiraModels

// Session patches (packages/shared/sessionPatch.ts). The engine sends a page only the changed tail of a session view
// (rust/crates/acpira-shared/src/session_patch.rs); the page keeps the view the patch was computed against and splices.
// Views are values here, so the webview's reference reuse (packages/shared/reuse.ts) has no counterpart: SwiftUI diffs
// turns by id and value.

/// The view a patch leads to, or nil when `current` is not the view it was computed against (another session, another
/// rev, too few turns or blocks): the page then asks for the whole view (`resync`)
public func applySessionPatch(_ current: SessionView?, _ p: SessionPatch) -> SessionView? {
  guard let current, current.id == p.id, current.rev == p.base, p.keep >= 0, current.turns.count >= p.keep else { return nil }
  var tail = p.turns
  if let keepBlocks = p.keepBlocks {
    guard keepBlocks >= 0, p.keep < current.turns.count, case .agent(let base) = current.turns[p.keep],
          case .agent(var head)? = tail.first, base.blocks.count >= keepBlocks else { return nil }
    head.blocks = Array(base.blocks[..<keepBlocks]) + (p.blocks ?? [])
    tail[0] = .agent(head)
  }
  var next = p.view
  next.turns = Array(current.turns[..<p.keep]) + tail
  return next
}

/// reuse.ts applySession: of two payloads of one session that raced, the newer rev wins; a view without a rev always
/// applies, so an unversioned source still updates
func newer(_ current: SessionView?, _ next: SessionView) -> SessionView {
  if let current, current.id == next.id, let c = current.rev, let n = next.rev, n <= c { return current }
  return next
}
