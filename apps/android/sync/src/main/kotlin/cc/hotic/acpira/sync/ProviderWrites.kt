package cc.hotic.acpira.sync

import cc.hotic.acpira.model.Provider
import cc.hotic.acpira.model.ProviderAction

/**
 * Model-source writes in order (packages/ui/settings/providerWrites.ts): a providers answer carries the whole list and
 * no request id, so one write is in flight at a time and the next goes once an answer lands. Consecutive row edits
 * (optimistic) of one source collapse into the newest; a form's write (a key, a new source) is never merged
 */
class ProviderWrites(private val send: (ProviderAction) -> Unit, private val read: () -> Unit) {
  private class Write(val action: ProviderAction, val optimistic: Boolean)

  private var active: Write? = null
  private val queue = ArrayDeque<Write>()
  private var reading = false

  val pending: Boolean get() = active != null || queue.isNotEmpty()

  /** Asks for the list, unless a read or a write is already on its way (its answer brings the list too) */
  fun refresh() {
    if (reading || pending) return
    reading = true
    read()
  }

  /** The row edits not answered yet, by source id: shown over the list until their writes settle */
  val drafts: Map<String, Provider>
    get() {
      val out = LinkedHashMap<String, Provider>()
      for (w in listOfNotNull(active) + queue) {
        val a = w.action
        if (w.optimistic && a is ProviderAction.Save) out[a.provider.id] = a.provider
      }
      return out
    }

  fun push(action: ProviderAction, optimistic: Boolean = false) {
    val write = Write(action, optimistic)
    if (active == null && !reading) {
      active = write
      send(action)
      return
    }
    val last = queue.lastOrNull()?.takeIf { it.optimistic }?.action as? ProviderAction.Save
    if (optimistic && action is ProviderAction.Save && last != null && last.provider.id == action.provider.id) {
      queue[queue.size - 1] = write
    } else queue.addLast(write)
  }

  /** A providers answer landed: the write it settles (none when it answered a read), and the next write goes */
  fun receive(): ProviderAction? {
    // A read already in flight when the first write was queued must not acknowledge that write
    val settled = if (reading) null else active?.action
    reading = false
    active = queue.removeFirstOrNull()
    active?.let { send(it.action) }
    return settled
  }
}
