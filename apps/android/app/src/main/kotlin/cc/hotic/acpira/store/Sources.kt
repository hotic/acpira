package cc.hotic.acpira.store

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import cc.hotic.acpira.model.HostMsg
import cc.hotic.acpira.model.ProbeOutcome
import cc.hotic.acpira.model.Provider
import cc.hotic.acpira.model.ProviderAction
import cc.hotic.acpira.model.ProviderModel
import cc.hotic.acpira.model.ProviderProbe
import cc.hotic.acpira.model.ProvidersView
import cc.hotic.acpira.model.WebviewMsg
import cc.hotic.acpira.sync.ProviderWrites
import kotlinx.serialization.json.JsonObject

/**
 * The computer's folders for the add-project picker (helloOk.features 'localDir'): the listing last asked for, by the
 * seq its answer echoes, so an answer to a folder left before it came is dropped
 */
class LocalFolders(private val post: (WebviewMsg) -> Boolean) {
  var listing by mutableStateOf<HostMsg.LocalDir?>(null)
    private set
  var loading by mutableStateOf(false)
    private set
  private var seq = 0

  /** Lists [path] (null: the user's home) */
  fun open(path: String?) {
    seq += 1
    loading = post(WebviewMsg.LocalDir(path, seq))
  }

  fun receive(m: HostMsg.LocalDir) {
    if (m.seq != seq) return
    listing = m
    loading = false
  }
}

/**
 * The built-in agent's model sources as the settings pages edit them (packages/ui App.tsx `providers` state with
 * ProvidersSection.tsx): the engine's last list with the row edits not answered yet laid over it, the answers to
 * probes by id, and the write each answer settled. [onSaved] runs once the last write landed without an error
 */
class ModelSources(private val post: (WebviewMsg) -> Boolean, private val onSaved: () -> Unit) {
  /** A write an answer acknowledged, and that answer's error (the write failed) */
  class Settled(val action: ProviderAction, val error: String?)

  var view by mutableStateOf<ProvidersView?>(null)
    private set
  /** The last answer's error: the action that produced it failed */
  var error by mutableStateOf<String?>(null)
    private set
  var settled by mutableStateOf<Settled?>(null)
    private set
  val probes = mutableStateMapOf<String, ProbeOutcome>()
  /** What a source's endpoint listed when its add-model page was asked for (or a new source was saved), by source id */
  val listed = mutableStateMapOf<String, List<ProviderModel>>()
  private var drafts by mutableStateOf<Map<String, Provider>>(emptyMap())
  private val writes = ProviderWrites({ post(WebviewMsg.ProviderAction(it)) }, { post(WebviewMsg.Providers) })
  private var probeSeq = 0

  /** The sources as shown: a pending row edit over the engine's entry (the entry keeps its `hasKey`) */
  val providers: List<Provider>
    get() = view?.providers.orEmpty().map { p -> drafts[p.id]?.let { d -> Provider(JsonObject(p.json + d.json)) } ?: p }

  fun refresh() = writes.refresh()

  /** optimistic: a row edit, shown at once and merged with the next edit of its source while queued */
  fun push(action: ProviderAction, optimistic: Boolean = false) {
    writes.push(action, optimistic)
    drafts = writes.drafts
  }

  /** Asks a question of the network; its answer lands in [probes] under the returned id */
  fun probe(probe: ProviderProbe): String {
    val id = "probe-${++probeSeq}"
    post(WebviewMsg.ProviderProbe(id, probe))
    return id
  }

  fun receive(m: HostMsg.Providers) {
    val done = writes.receive()
    drafts = writes.drafts
    view = m.view
    error = m.error
    if (done != null) {
      settled = Settled(done, m.error)
      if (!writes.pending && m.error == null) onSaved()
    }
  }

  fun receive(m: HostMsg.ProviderProbed) {
    probes[m.id] = m.outcome
  }
}
