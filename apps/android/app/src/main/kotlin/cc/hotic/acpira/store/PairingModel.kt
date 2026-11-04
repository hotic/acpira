package cc.hotic.acpira.store

import android.content.Context
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.sync.ConnectionError
import cc.hotic.acpira.sync.PairedComputer
import cc.hotic.acpira.sync.PairedConnection
import cc.hotic.acpira.sync.PairingService
import cc.hotic.acpira.sync.PathKind
import cc.hotic.acpira.sync.Presence
import cc.hotic.acpira.sync.isPairingLink
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/** The followed computer's wire */
sealed interface PairLink {
  data object Idle : PairLink
  data object Connecting : PairLink
  data class Online(val path: PathKind) : PairLink
  data class Failed(val error: ConnectionError) : PairLink
}

/**
 * The phone's paired computers as the screens see them (apps/ios Pairing/PairingModel.swift): the list, the one the
 * session list follows, every computer's state, and the add-computer sheet. Everything goes through [PairingService];
 * the followed computer's wire goes to the app through [onConnection], and its end comes back with [connectionEnded].
 * Compose state, read and written on the main thread
 */
class PairingModel(val service: PairingService, context: Context, private val scope: CoroutineScope) {
  /** Paired computers, most recently paired first */
  var computers by mutableStateOf<List<PairedComputer>>(emptyList())
    private set
  /** The service answered once */
  var loaded by mutableStateOf(false)
    private set
  /** What a probe (or the followed computer's live wire) last said about each computer */
  val presence = mutableStateMapOf<String, Presence>()
  var link by mutableStateOf<PairLink>(PairLink.Idle)
    private set
  /** The followed computer, kept across launches */
  private var selectedId by mutableStateOf<String?>(null)
  /** The add-computer sheet over the list */
  var addOpen by mutableStateOf(false)
  /** A computer the sheet lists as nearby whatever NSD finds (debug builds' `nearby` launch extra) */
  var pinnedNearby: cc.hotic.acpira.sync.NearbyComputer? = null
  /** The first-run flow is up and pairs in place: the add sheet stays down meanwhile */
  var onboarding by mutableStateOf(false)
  /** An `acpira://pair/…` link waiting for confirmation in the sheet */
  var pendingLink by mutableStateOf<String?>(null)

  /** The followed computer's open wire */
  var connection: PairedConnection? = null
    private set
  var onConnection: ((PairedComputer, PairedConnection) -> Unit)? = null
  private var dialing: Job? = null
  private var watching: Job? = null
  /** The next dial after a wire that ended on its own */
  private var redial: Job? = null
  /** Wires in a row that failed or ended before they settled */
  private var failures = 0
  /** Dialing again after a wire that was up got lost, as opposed to a computer that never answered */
  private var reconnecting = false
  /** When the current wire came up (monotonic ms) */
  private var wiredAt: Long? = null
  private val prefs = context.getSharedPreferences("pairing", Context.MODE_PRIVATE)

  init {
    selectedId = prefs.getString(SELECTED, null)
  }

  /** The computer the session list follows: the chosen one, else the most recently paired */
  val selected: PairedComputer? get() = computers.firstOrNull { it.id == selectedId } ?: computers.firstOrNull()

  /** Reads the paired computers, dials the followed one and probes the rest */
  fun load() {
    scope.launch {
      computers = service.computers()
      loaded = true
      connectSelected()
      refreshPresence()
    }
  }

  /**
   * Every computer's state, probed side by side; the followed one keeps its live wire's word. It is not probed while its
   * own dial or wire is up: the gateway keeps one connection per device, so a probe after the dial replaced the wire
   */
  suspend fun refreshPresence() {
    val probing = computers.filter { !(it.id == selected?.id && wired) }
    val probed = probing.map { c -> scope.async { c.id to runCatching { service.presence(c) }.getOrNull() } }.awaitAll()
    for ((id, state) in probed) {
      if (state == null) continue
      if (id == selected?.id && link != PairLink.Idle && link != PairLink.Connecting) continue
      presence[id] = state
    }
  }

  /** The followed computer is being dialed or its wire is up */
  val wired: Boolean get() = link is PairLink.Connecting || link is PairLink.Online

  /** Follows another computer */
  fun select(id: String) {
    if (id == selected?.id || computers.none { it.id == id }) return
    selectedId = id
    prefs.edit().putString(SELECTED, id).apply()
    connectSelected()
  }

  /** Dials the followed computer, dropping any wire to the previous one */
  fun connectSelected() {
    failures = 0
    reconnecting = false
    dial(quiet = false)
  }

  /** quiet: keep the offline page up while it dials (the slow redial), so a computer still away changes nothing */
  private fun dial(quiet: Boolean) {
    redial?.cancel()
    redial = null
    dialing?.cancel()
    watching?.cancel()
    connection?.close()
    connection = null
    val computer = selected ?: run {
      link = PairLink.Idle
      return
    }
    if (!quiet) link = PairLink.Connecting
    android.util.Log.i(TAG, "dialing ${computer.id.take(10)} (quiet $quiet)")
    dialing = scope.launch {
      try {
        val wire = service.connect(computer)
        if (selected?.id != computer.id) {
          wire.close()
          return@launch
        }
        connection = wire
        wiredAt = now()
        onConnection?.invoke(computer, wire)
        watch(wire, computer)
      } catch (e: CancellationException) {
        throw e
      } catch (e: Exception) {
        if (selected?.id != computer.id) return@launch
        ended(e as? ConnectionError ?: ConnectionError.Other(e.message ?: e.toString()), computer.id)
      }
    }
  }

  /** The wire's path as it changes; the flow ends with the connection */
  private fun watch(wire: PairedConnection, computer: PairedComputer) {
    watching = scope.launch {
      wire.paths().collect { path ->
        link = PairLink.Online(path)
        presence[computer.id] = Presence.Online(path)
      }
      // Only the current wire: the app model may have reported its end already
      if (selected?.id != computer.id || connection !== wire) return@launch
      // The paths can end before the frames report why (a background close is not the computer going offline)
      ended(wire.endReason() ?: ConnectionError.Offline, computer.id)
    }
  }

  /**
   * The app model reports how the followed computer's wire ended. Only the current wire counts: a wire closed by a
   * switch or a new pairing ends after the next one is dialed
   */
  fun connectionEnded(error: ConnectionError, wire: PairedConnection) {
    val id = selected?.id ?: return
    if (connection !== wire) return
    watching?.cancel()
    ended(error, id)
  }

  private fun ended(error: ConnectionError, id: String) {
    android.util.Log.i(TAG, "wire to $id ended: ${error.message} (failures $failures, reconnecting $reconnecting)")
    connection = null
    wiredAt?.let { up ->
      // A wire that was up and is lost: the quick dials, counted afresh once it had settled
      reconnecting = true
      if (now() - up >= SETTLED_MS) failures = 0
    }
    wiredAt = null
    // The app's own background close: coming back dials again, so nothing failed
    if (error == ConnectionError.Suspended) {
      reconnecting = false
      link = PairLink.Idle
      return
    }
    // A lost wire is dialed again at once, then with growing waits, and the screens keep what they showed. A computer
    // that did not answer in the first place shows as offline at once and is dialed quietly. A removal or another
    // protocol version will not mend itself
    if (error.transient) {
      failures += 1
      if (reconnecting && failures <= QUICK_REDIALS_MS.size) {
        link = PairLink.Connecting
        scheduleRedial(QUICK_REDIALS_MS[failures - 1], quiet = false, id)
        return
      }
      reconnecting = false
      scheduleRedial(if (failures <= FIRST_QUIET_REDIALS) FIRST_QUIET_REDIAL_MS else SLOW_REDIAL_MS, quiet = true, id)
    } else {
      reconnecting = false
    }
    link = PairLink.Failed(error)
    when (error) {
      ConnectionError.Offline -> presence[id] = Presence.Offline
      ConnectionError.Revoked -> presence[id] = Presence.Revoked
      else -> presence.remove(id)
    }
  }

  private fun scheduleRedial(waitMs: Long, quiet: Boolean, id: String) {
    redial?.cancel()
    redial = scope.launch {
      delay(waitMs)
      // Another computer followed, a Retry tap or a new wire meanwhile
      if (selected?.id != id || connection != null) return@launch
      dial(quiet)
    }
  }

  /** A pairing finished: the list follows the new computer at once, so its wire is up by the time the sheet goes */
  suspend fun paired(computer: PairedComputer) {
    computers = service.computers().let { list -> if (list.any { it.id == computer.id }) list else listOf(computer) + list }
    selectedId = computer.id
    prefs.edit().putString(SELECTED, computer.id).apply()
    connectSelected()
  }

  /** Forgets a computer here; the list then follows the next one, or shows the empty state */
  suspend fun remove(id: String) {
    val wasSelected = selected?.id == id
    service.remove(id)
    computers = computers.filter { it.id != id }
    presence.remove(id)
    if (wasSelected) {
      selectedId = computers.firstOrNull()?.id
      prefs.edit().putString(SELECTED, selectedId).apply()
      connectSelected()
    }
  }

  /** `acpira://pair/…` opened from outside (a scanned code in the camera app, a tapped link): the sheet asks first */
  fun open(link: String): Boolean {
    if (!isPairingLink(link)) return false
    pendingLink = link.trim()
    addOpen = true
    return true
  }

  /** A computer's state in one or two words (the menu's subtitle, the settings' value); null while nothing is known */
  fun status(computer: PairedComputer): String? {
    if (computer.id == selected?.id) {
      when (val l = link) {
        is PairLink.Online -> return pathLabel(l.path)
        is PairLink.Failed -> when (l.error) {
          ConnectionError.Offline -> return t("mobile.pair.state.offline")
          ConnectionError.Revoked -> return t("mobile.pair.state.revoked")
          else -> {}
        }
        else -> {}
      }
    }
    return when (val p = presence[computer.id]) {
      is Presence.Online -> pathLabel(p.path)
      Presence.Offline -> t("mobile.pair.state.offline")
      Presence.Revoked -> t("mobile.pair.state.revoked")
      null -> null
    }
  }

  /** Whether the computer answers (null: not known), for the settings' dot */
  fun online(computer: PairedComputer): Boolean? {
    if (computer.id == selected?.id && link is PairLink.Online) return true
    return when (presence[computer.id]) {
      is Presence.Online -> true
      Presence.Offline, Presence.Revoked -> false
      null -> null
    }
  }

  private fun now() = System.nanoTime() / 1_000_000

  companion object {
    private const val SELECTED = "selected"
    private const val TAG = "acpira-pair"
    /** The waits before dialing again after a wire was lost; the screens stay as they were meanwhile */
    val QUICK_REDIALS_MS = listOf(300L, 1_000L, 2_000L, 4_000L, 8_000L)
    /** Past those the offline page shows with its Retry, and a quiet dial every so often brings the computer back */
    const val SLOW_REDIAL_MS = 15_000L
    const val FIRST_QUIET_REDIAL_MS = 3_000L
    const val FIRST_QUIET_REDIALS = 2
    /** A wire up this long counts as a success, so the next loss starts the quick dials over */
    const val SETTLED_MS = 10_000L
  }
}

/** The path's plain name: 局域网 / Tailscale / 直连 / 中继 */
fun pathLabel(path: PathKind): String = t("mobile.pair.path.${path.name}")
