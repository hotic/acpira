import AcpiraModels

// Session patches (packages/shared/sessionPatch.ts). The engine sends a page only the changed tail of a session view
// (rust/crates/acpira-shared/src/session_patch.rs); the page keeps the view the patch was computed against and splices.
// Views are values here, so the webview's reference reuse (packages/shared/reuse.ts) has no counterpart: SwiftUI diffs
// turns by id and value.

/// The view a patch leads to, or nil when `current` is not the view it was computed against (another session, another
/// rev, another window, too few turns or blocks): the page then asks for the whole view (`resync`)
public func applySessionPatch(_ current: SessionView?, _ p: SessionPatch) -> SessionView? {
  guard let current, current.id == p.id, current.rev == p.base, p.keep >= 0, current.turns.count >= p.keep else { return nil }
  // A patch counts its indices inside the window the engine has on record for the page and never moves that window
  // (only a chunk or a whole view does). One for another window means a `sessionEarlier` went missing on the way: its
  // indices would splice into the wrong turns. Not in sessionPatch.ts, whose pages take no windows
  guard p.view.earlier == current.earlier else { return nil }
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

// Windowed session views (packages/shared/sessionWindow.ts, rust/crates/acpira-shared/src/session_window.rs). The app
// says `window` in ready, so a view starts at its tail, `earlier` naming what is left out, and the thread asks for the
// part right ahead of what it holds (`loadEarlier`) as the reader nears its top. Patches need nothing new: the engine
// counts their indices inside the page's window.

/// The view with the chunk in front, or nil when the chunk was cut for another window or another session. The rev
/// stays: nothing in the session changed
public func applySessionEarlier(_ current: SessionView?, _ c: SessionEarlier) -> SessionView? {
  guard let current, current.id == c.id, (current.earlier ?? ViewEarlier(turns: 0, blocks: 0)) == c.from else { return nil }
  var next = current
  if let blocks = c.blocks, !blocks.isEmpty {
    // They are the blocks the held view's first turn was cut by, so that turn is an agent turn
    guard case .agent(var first)? = next.turns.first else { return nil }
    first.blocks = blocks + first.blocks
    next.turns[0] = .agent(first)
  }
  next.turns = c.turns + next.turns
  next.earlier = c.earlier
  return next
}
