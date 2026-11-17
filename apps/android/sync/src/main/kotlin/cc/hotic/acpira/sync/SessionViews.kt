package cc.hotic.acpira.sync

import cc.hotic.acpira.model.AgentTurn
import cc.hotic.acpira.model.HostMsg
import cc.hotic.acpira.model.SessionEarlier
import cc.hotic.acpira.model.SessionPatch
import cc.hotic.acpira.model.SessionView
import cc.hotic.acpira.model.ViewEarlier

// Session patches (packages/shared/sessionPatch.ts) and the views the page keeps (packages/ui/sessionViews.ts). The
// engine sends a page only the changed tail of a session view (rust/crates/acpira-shared/src/session_patch.rs); the page
// keeps the view the patch was computed against and splices. Views are immutable values here, so the webview's
// reference reuse (packages/shared/reuse.ts) has no counterpart: Compose skips turns that compare equal. Same rules as
// apps/ios AcpiraSync/SessionPatches.swift and SessionViews.swift.
//
// The page says `window` in ready (packages/shared/sessionWindow.ts), so a view starts at the session's tail and the
// part ahead of it comes in chunks the page asks for. Patches need nothing for that: the engine counts their indices
// inside the window the page holds.

/**
 * The view a patch leads to, or null when [current] is not the view it was computed against (another session, another
 * rev, too few turns or blocks): the page then asks for the whole view (`resync`)
 */
fun applySessionPatch(current: SessionView?, p: SessionPatch): SessionView? {
  if (current == null || current.id != p.id || current.rev != p.base || p.keep < 0 || current.turns.size < p.keep) return null
  val tail = p.turns.toMutableList()
  val keepBlocks = p.keepBlocks
  if (keepBlocks != null) {
    val base = current.turns.getOrNull(p.keep) as? AgentTurn ?: return null
    val head = tail.firstOrNull() as? AgentTurn ?: return null
    if (keepBlocks < 0 || base.blocks.size < keepBlocks) return null
    tail[0] = head.copy(blocks = base.blocks.subList(0, keepBlocks) + p.blocks.orEmpty())
  }
  return p.view.copy(turns = current.turns.subList(0, p.keep) + tail)
}

/**
 * sessionWindow.ts applySessionEarlier: the view with the chunk in front, or null when the chunk was cut for another
 * window (a whole view landed since the page asked): the page drops it and asks again if it still wants more. The rev
 * stays, nothing in the session changed
 */
fun applySessionEarlier(current: SessionView?, c: SessionEarlier): SessionView? {
  if (current == null || current.id != c.id) return null
  if ((current.earlier ?: ViewEarlier()) != c.from) return null
  var held = current.turns
  val blocks = c.blocks
  if (!blocks.isNullOrEmpty()) {
    // The blocks belong to the turn the window was cut inside, which only an agent turn can be
    val first = held.firstOrNull() as? AgentTurn ?: return null
    held = listOf(first.copy(blocks = blocks + first.blocks)) + held.subList(1, held.size)
  }
  return current.copy(turns = c.turns + held, earlier = c.earlier)
}

/**
 * reuse.ts applySession: of two payloads of one session that raced, the newer rev wins; a view without a rev always
 * applies, so an unversioned source still updates
 */
internal fun newer(current: SessionView?, next: SessionView): SessionView {
  if (current != null && current.id == next.id) {
    val c = current.rev
    val n = next.rev
    if (c != null && n != null && n <= c) return current
  }
  return next
}

/**
 * Sessions whose last view the page keeps after switching away (sessionViews.ts VIEW_CACHE_ENTRIES). At least the
 * engine's SENT_VIEWS_MAX: it only patches against views it sent within its own bound
 */
const val VIEW_CACHE_ENTRIES = 12

/** What a session push does to the page */
sealed interface SessionPush {
  data class Show(val view: SessionView) : SessionPush
  /** Older than the view on screen, or a late patch of the session just left: nothing changes on screen */
  data object Stale : SessionPush
  /** The page does not hold the view the patch was computed against: ask for the whole view (`resync`) */
  data object Unknown : SessionPush
}

/**
 * What the page shows plus the last view of each recent session. A whole view (init, `session`) is always taken: the
 * engine sends one only when the page holds nothing it can patch, and revs are not comparable across epochs
 */
class SessionViews {
  private val views = HashMap<String, SessionView>()
  /** Least recently shown first; the view on screen is in here too */
  private val order = ArrayList<String>()
  /** The session just switched away from optimistically, until the engine's reply for the target lands */
  private var leaving: String? = null
  /** The kept session `select` put on screen, until the engine's first reply for it lands */
  var catchingUp: String? = null
    private set
  var current: SessionView? = null
    private set

  /** `init`: the engine starts its record of what the page holds over, so the page does too */
  fun reset(active: SessionView?): SessionView? {
    views.clear(); order.clear(); leaving = null; catchingUp = null; current = null
    return active?.let { show(it) }
  }

  /** Optimistically switch to a kept session on a tap; null when the session is not kept */
  fun select(id: String): SessionView? {
    current?.let { if (it.id == id) return it }
    val kept = views[id] ?: return null
    leaving = current?.id
    catchingUp = id
    current = kept
    return kept
  }

  /** A whole view, taken whatever its rev */
  fun whole(next: SessionView): SessionView {
    leaving = null
    catchingUp = null
    return show(next)
  }

  fun patch(p: SessionPatch): SessionPush {
    // A late patch of the session the reader just left keeps its kept copy current without flipping the screen back
    if (leaving != null && p.id == leaving && current?.id != p.id) {
      val base = views[p.id]
      val updated = applySessionPatch(base, p)
      if (base != null && updated != null) views[p.id] = newer(base, updated) else drop(p.id)
      return SessionPush.Stale
    }
    leaving = null
    if (p.id == catchingUp) catchingUp = null
    val onScreen = current?.takeIf { it.id == p.id }
    applySessionPatch(onScreen ?: views[p.id], p)?.let { return SessionPush.Show(show(newer(onScreen, it))) }
    val rev = onScreen?.rev
    if (rev != null && rev >= (p.view.rev ?: 0L)) return SessionPush.Stale
    drop(p.id)
    return SessionPush.Unknown
  }

  /**
   * A chunk of the part ahead of a view (`sessionEarlier`), put in front of it wherever the page holds it: on screen, or
   * kept from a session shown before. One that does not fit the held window is dropped. It is no new showing of the
   * session, so the cache order stays
   */
  fun earlier(c: SessionEarlier): SessionPush {
    val onScreen = current?.takeIf { it.id == c.id }
    val next = applySessionEarlier(onScreen ?: views[c.id], c) ?: return SessionPush.Stale
    if (c.id in views) views[c.id] = next
    if (onScreen == null) return SessionPush.Stale
    current = next
    return SessionPush.Show(next)
  }

  /** Ids kept, least recently shown first */
  val kept: List<String> get() = order.toList()

  private fun show(view: SessionView): SessionView {
    order.remove(view.id)
    order.add(view.id)
    views[view.id] = view
    while (order.size > VIEW_CACHE_ENTRIES) views.remove(order.removeAt(0))
    current = view
    return view
  }

  private fun drop(id: String) {
    views.remove(id)
    order.remove(id)
  }
}

/**
 * The page's handling of session pushes (packages/ui/App.tsx `init`, `session`, `noSession`, `sessionPatch`, and
 * `sessionEarlier`): the view to show, and a `resync` asked once for a patch that does not fit, until a whole view lands
 */
class SessionFeed {
  sealed interface Outcome {
    /** Show this view (null: no session) */
    data class Show(val view: SessionView?) : Outcome
    data object Unchanged : Outcome
    /** Post `resync`: the engine sends the whole view again */
    data object Resync : Outcome
  }

  val views = SessionViews()
  private var resyncAsked = false

  fun receive(message: HostMsg): Outcome = when (message) {
    is HostMsg.Init -> Outcome.Show(views.reset(message.state.active))
    is HostMsg.Session -> {
      resyncAsked = false
      Outcome.Show(views.whole(message.session))
    }
    HostMsg.NoSession -> Outcome.Show(null)
    is HostMsg.Patch -> when (val push = views.patch(message.patch)) {
      is SessionPush.Show -> Outcome.Show(push.view)
      SessionPush.Stale -> Outcome.Unchanged
      SessionPush.Unknown -> if (resyncAsked) Outcome.Unchanged else {
        resyncAsked = true
        Outcome.Resync
      }
    }
    is HostMsg.Earlier -> when (val push = views.earlier(message.chunk)) {
      is SessionPush.Show -> Outcome.Show(push.view)
      else -> Outcome.Unchanged
    }
    else -> Outcome.Unchanged
  }

  /** A tap on a session: its kept view at once, or null to wait for the engine's */
  fun select(id: String): SessionView? = views.select(id)
}
