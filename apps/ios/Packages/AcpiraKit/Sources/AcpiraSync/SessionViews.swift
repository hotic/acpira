import AcpiraModels

/// Sessions whose last view the page keeps after switching away (packages/ui/sessionViews.ts VIEW_CACHE_ENTRIES). At
/// least the engine's SENT_VIEWS_MAX: it only patches against views it sent within its own bound, so every view it may
/// patch against is still here
public let viewCacheEntries = 12

/// What a session push does to the page
public enum SessionPush: Equatable, Sendable {
  /// The view to show
  case show(SessionView)
  /// Older than the view on screen, or a late patch of the session just left: nothing changes on screen
  case stale
  /// The page does not hold the view the patch was computed against: ask for the whole view (`resync`)
  case unknown
}

/// What a `sessionEarlier` chunk does to the page
public enum EarlierPush: Equatable, Sendable {
  /// The view of the chunk's session, on screen or kept, with the chunk in front
  case took(SessionView)
  /// The page holds no view of that session, or a kept one the chunk did not fit (dropped): nothing changes on screen
  case stale
  /// The chunk does not fit the view on screen: ask for the whole view (`resync`)
  case unknown
}

/// What the page shows plus the last view of each recent session (packages/ui/sessionViews.ts, the reference for every
/// rule here). Switching back to a kept session arrives as a patch against the kept view, not the whole transcript.
/// A whole view (init, `session`) is always taken: the engine sends one only when the page holds nothing it can patch,
/// and revs are not comparable across epochs (a session followed from another engine counts from -2^40, a reopened one
/// from 0 again), so gating a whole view on rev would freeze the page on its old copy
public struct SessionViews: Sendable {
  /// Least recently shown first; the view on screen is in here too
  private var views: [String: SessionView] = [:]
  private var order: [String] = []
  /// The session just switched away from optimistically, until the engine's reply for the target lands
  private var leaving: String?
  /// The kept session `select` put on screen, until the engine's first reply for it lands. That reply carries what
  /// happened while the reader was away, which the thread shows in place instead of playing it as live output
  public private(set) var catchingUp: String?
  public private(set) var current: SessionView?

  public init() {}

  /// `init`: the engine starts its record of what the page holds over, so the page does too
  public mutating func reset(active: SessionView?) -> SessionView? {
    views = [:]
    order = []
    leaving = nil
    catchingUp = nil
    current = nil
    guard let active else { return nil }
    return show(active)
  }

  /// Optimistically switch to a kept session on a tap, so the screen changes before the engine answers; its patch (or
  /// whole view) that follows updates it in place. Nil when the session is not kept
  public mutating func select(_ id: String) -> SessionView? {
    if let current, current.id == id { return current }
    guard let kept = views[id] else { return nil }
    leaving = current?.id
    catchingUp = id
    current = kept
    return kept
  }

  /// A whole view, taken whatever its rev: the kept copy (on screen after an optimistic `select`, too) must not win on rev
  public mutating func whole(_ next: SessionView) -> SessionView {
    leaving = nil
    catchingUp = nil
    return show(next)
  }

  public mutating func patch(_ p: SessionPatch) -> SessionPush {
    // A late patch of the session the reader just left keeps its kept copy current without flipping the screen back
    if let leaving, p.id == leaving, current?.id != p.id {
      if let base = views[p.id], let updated = applySessionPatch(base, p) { views[p.id] = newer(base, updated) } else { drop(p.id) }
      return .stale
    }
    leaving = nil
    if p.id == catchingUp { catchingUp = nil }
    let onScreen = current?.id == p.id ? current : nil
    if let next = applySessionPatch(onScreen ?? views[p.id], p) { return .show(show(newer(onScreen, next))) }
    if let onScreen, let rev = onScreen.rev, rev >= (p.view.rev ?? 0) { return .stale }
    drop(p.id)
    return .unknown
  }

  /// The part right ahead of a view the page holds, on screen or kept (the reader may have switched away since the
  /// thread asked): that view with the chunk in front, where it stands. The engine moved its record of where the
  /// page's copy starts when it cut the chunk, so a chunk that does not fit a held view leaves the two apart, and every
  /// later patch and chunk would count from the wrong turn: the copy is dropped as unknown, as after a patch that did
  /// not fit (sessionWindow.ts only drops the chunk)
  public mutating func earlier(_ c: SessionEarlier) -> EarlierPush {
    let onScreen = current?.id == c.id
    guard let held = onScreen ? current : views[c.id] else { return .stale }
    guard let next = applySessionEarlier(held, c) else {
      drop(c.id)
      return onScreen ? .unknown : .stale
    }
    // In place: taking in an earlier part is no reason to count the session as shown more recently
    if views[c.id] != nil { views[c.id] = next }
    if onScreen { current = next }
    return .took(next)
  }

  /// Ids kept, least recently shown first
  public var kept: [String] { order }

  private mutating func show(_ view: SessionView) -> SessionView {
    order.removeAll { $0 == view.id }
    order.append(view.id)
    views[view.id] = view
    while order.count > viewCacheEntries { views[order.removeFirst()] = nil }
    current = view
    return view
  }

  private mutating func drop(_ id: String) {
    views[id] = nil
    order.removeAll { $0 == id }
  }
}

/// The page's handling of session pushes (packages/ui/App.tsx, its `init`, `session`, `noSession` and `sessionPatch`
/// cases, plus `sessionEarlier`): the view to show, and a `resync` asked once for a patch or chunk that does not fit,
/// until a whole view lands
public struct SessionFeed: Sendable {
  public enum Outcome: Equatable, Sendable {
    /// Show this view (nil: no session)
    case show(SessionView?)
    /// A view the page already holds, with the earlier part it asked for in front: nothing in the session changed,
    /// and the session on screen stays the one it was
    case earlier(SessionView)
    case unchanged
    /// Post `resync`: the engine sends the whole view again
    case resync
  }

  public private(set) var views = SessionViews()
  private var resyncAsked = false

  public init() {}

  public mutating func receive(_ message: HostMsg) -> Outcome {
    switch message {
    case .initState(let state):
      return .show(views.reset(active: state.active))
    case .session(let view):
      resyncAsked = false
      return .show(views.whole(view))
    case .noSession:
      return .show(nil)
    case .sessionPatch(let patch):
      switch views.patch(patch) {
      case .show(let view): return .show(view)
      case .stale: return .unchanged
      case .unknown: return askResync()
      }
    case .sessionEarlier(let chunk):
      switch views.earlier(chunk) {
      case .took(let view): return .earlier(view)
      case .stale: return .unchanged
      case .unknown: return askResync()
      }
    default:
      return .unchanged
    }
  }

  private mutating func askResync() -> Outcome {
    if resyncAsked { return .unchanged }
    resyncAsked = true
    return .resync
  }

  /// A tap on a session: its kept view at once, or nil to wait for the engine's
  public mutating func select(_ id: String) -> SessionView? {
    views.select(id)
  }
}
