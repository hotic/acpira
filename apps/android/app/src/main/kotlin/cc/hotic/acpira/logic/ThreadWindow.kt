package cc.hotic.acpira.logic

import cc.hotic.acpira.model.SessionView

// The chat's thread over a windowed session view (packages/shared/sessionWindow.ts). The list holds one item per turn,
// keyed by the turn's index in the session, with a lead item ahead of them while part of the session is left out. A
// chunk of the earlier part (`sessionEarlier`) puts turns ahead of the held ones, and blocks ahead of the first held
// turn's own when the window was cut inside it: that turn keeps its key and grows at its top.

/** Where the thread's view starts: the session's turns left out ahead of it, and the blocks left out of its first turn */
data class ThreadStart(val turns: Int, val blocks: Int) {
  /** Items ahead of the turns: the lead, while anything is left out */
  val lead: Int get() = if (turns > 0 || blocks > 0) 1 else 0

  /** This view starts ahead of [other]: a chunk went in front of it */
  fun aheadOf(other: ThreadStart): Boolean = turns < other.turns || (turns == other.turns && blocks < other.blocks)

  companion object {
    fun of(view: SessionView): ThreadStart = ThreadStart(view.earlier?.turns ?: 0, view.earlier?.blocks ?: 0)
  }
}

/** One item of the thread as it was last laid out: its key (a turn's session index, or a name), index, offset and size */
data class ThreadItem(val key: Any, val index: Int, val offset: Int, val size: Int)

/** What keeps the reader's place when a chunk lands ahead of what the thread shows ([holdPlace]) */
data class ThreadHold(
  /**
   * The item (its index in the new list) to put back at this scroll offset before the list is measured again; null
   * when the list's own key tracking keeps it
   */
  val request: Pair<Int, Int>?,
  /**
   * The turn (its key) that grew at its top with nothing under it on screen to hold on to, and where its end stood:
   * its end goes back there once the list has measured it
   */
  val grown: Pair<Int, Int>?,
)

/**
 * Past this many items put ahead, the list no longer finds its first item by key (LazyColumn maps keys to indices only
 * around the first visible one, a hundred items either way), so the place is asked for by index instead
 */
private const val KEYED_REACH = 64

/**
 * holdPlace: what to do so the items in [visible] (the layout before the chunk) stay where they are once the thread
 * starts at [new] instead of [old]; null when nothing went in front. The place is held by an item whose top did not
 * move: any turn but the one the old window was cut inside. A scroll request stops a fling, so it is left out where the
 * list keeps the place by itself: the first item on screen is such a turn, and few items went ahead of it
 */
fun holdPlace(old: ThreadStart, new: ThreadStart, visible: List<ThreadItem>): ThreadHold? {
  if (!new.aheadOf(old)) return null
  val first = visible.firstOrNull() ?: return null
  val cut = old.turns.takeIf { old.blocks > 0 }
  val grown = visible.firstOrNull { it.key == cut }
  val still = visible.firstOrNull { it.key is Int && it.key != cut }
  val hold = still ?: grown ?: return null
  // How far the held turns moved down the list: the turns put ahead, less the lead when it left
  val shift = old.turns - new.turns + new.lead - old.lead
  val byKey = hold === first && shift <= KEYED_REACH
  return ThreadHold(
    request = if (byKey) null else (hold.index + shift) to -hold.offset,
    grown = if (still == null) grown?.let { it.key as Int to it.offset + it.size } else null,
  )
}

/** Parts a thread too short to scroll asks for on its own after it opens ([EarlierReach]) */
const val EARLIER_FILLS = 4

/**
 * When the part ahead of a windowed thread is asked for, the same rule on every phone app. A finished turn's steps sit
 * in a closed fold, so the parts of a long cut turn add no height as they land: asking wherever the layout stands near
 * the start would fetch the whole turn, part after part, without the reader asking. So only the reader's own scrolling
 * asks, with one exception: a thread that opens too short to scroll fills itself, a few parts at most. One request is
 * in flight at a time; that is the caller's to keep (AppModel.loadEarlier)
 */
class EarlierReach {
  private var filling = true
  private var filled = 0
  private var pulled = false

  /** The view is first shown, or a fresh window of it arrived (a whole view): it may fill itself again */
  fun opened() {
    filling = true
    filled = 0
  }

  /**
   * The thread is laid out from a new start (first shown, a fresh window, a part landed), and [scrolls] or not: true
   * to ask for the next part on its own. Filling ends for good once the thread scrolls or [EARLIER_FILLS] parts were
   * asked for; a part that lands later never asks for the next
   */
  fun laidOut(scrolls: Boolean): Boolean {
    if (!filling) return false
    if (scrolls || filled == EARLIER_FILLS) {
      filling = false
      return false
    }
    filled++
    return true
  }

  /**
   * The reader's own scroll with the thread's start within reach: true to ask. [moved] is how far it took the thread,
   * positive towards the start, and [left] what the thread could not take. Every move towards the start asks, so
   * scrolling on that way keeps loading; one away from it never does, though the start is still in view. A [drag]
   * pulling on a thread that did not move (its top edge, or a thread still too short to scroll) asks once until the
   * finger lifts ([released]); a fling run into the top edge is no pull
   */
  fun reached(moved: Float, left: Float, drag: Boolean): Boolean = when {
    moved > 0f -> true
    moved < 0f -> false
    left > 0f && drag && !pulled -> {
      pulled = true
      true
    }
    else -> false
  }

  /** The finger lifted: the next pull asks again */
  fun released() {
    pulled = false
  }
}
