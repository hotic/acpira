package cc.hotic.acpira.remote

import android.content.Context
import android.os.Build
import android.provider.Settings
import cc.hotic.acpira.model.SidecarProtocol
import cc.hotic.acpira.sync.ConnectionError
import cc.hotic.acpira.sync.NearbyComputer
import cc.hotic.acpira.sync.PairedComputer
import cc.hotic.acpira.sync.PairedConnection
import cc.hotic.acpira.sync.PairingError
import cc.hotic.acpira.sync.PairingService
import cc.hotic.acpira.sync.PathKind
import cc.hotic.acpira.sync.Presence
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.isActive
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withTimeoutOrNull
import uniffi.acpira_mobile.ComputerAddr
import uniffi.acpira_mobile.DeviceType
import uniffi.acpira_mobile.Link
import uniffi.acpira_mobile.Node
import uniffi.acpira_mobile.RelayChoice
import uniffi.acpira_mobile.decodeLink
import uniffi.acpira_mobile.generateKey
import uniffi.acpira_mobile.startNode
import java.io.File
import java.util.UUID

/**
 * `PairingService` over acpira-mobile (apps/ios AcpiraRemote/RemotePairingService.swift): the identity key sealed by the
 * Keystore, the paired computers in the app's files, NSD for the computers nearby, and one iroh node shared by every
 * connection.
 *
 * Lifecycle: going to the background keeps every connection for [BACKGROUND_GRACE_MS], so a glance at another app comes
 * back to the same wire; past it the connections end as [ConnectionError.Suspended] and the node closes (Android may
 * freeze a backgrounded app's sockets, and an endpoint can come back silently dead). Coming back after that rebuilds the
 * node from the same key. A network change tells iroh at once and probes every live connection, ending the ones that
 * neither answer nor bring anything in; a network that drops away entirely ends them only once it stayed away
 */
class RemotePairingService(
  private val context: Context,
  private val keys: IdentityStore = KeystoreIdentityStore(context),
  private val relays: RelayChoice = RelayChoice.N0,
) : PairingService {
  private val store = ComputerStore(File(context.noBackupFilesDir, "remote"))
  /** Every field below is read and written under [lock] */
  private val lock = Mutex()
  private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
  private var list: List<PairedComputer> = store.load()
  private var node: Node? = null
  private var starting: Deferred<Node>? = null
  /** Rebuild the node on the way back from the background (it was in use before) */
  private var rebuild = false
  @Volatile private var background = false
  /** When the app last went to the background; the close at the end of the grace belongs to that trip only */
  private var backgroundSince: Long? = null
  private var backgroundClose: Job? = null
  /** When the network last dropped away entirely, while it stays away */
  private var offlineSince: Long? = null
  private val live = HashMap<String, RemoteConnection>()
  /** Computers being dialed now: the gateway keeps one connection per device, so a probe must never dial one of them */
  private val dialing = HashMap<String, Int>()
  /** The path each computer was last reached on, for the probe deadline */
  private val lastPath = HashMap<String, PathKind>()
  @Volatile private var network: NetworkState? = null
  private var watch: NetworkWatch? = null

  override suspend fun computers(): List<PairedComputer> = lock.withLock { list.sortedByDescending { it.pairedAt } }

  override fun computerName(link: String): String = try {
    decodeLink(link).name
  } catch (_: Exception) {
    throw PairingError.InvalidLink
  }

  override suspend fun pair(link: String): PairedComputer {
    val node = pairingNode()
    try {
      val p = node.pairLink(link, deviceName(), deviceType(), appVersion(), PAIR_MS.toULong())
      return keep(PairedComputer(id = p.id, name = p.name, relay = p.relay, addrs = p.addrs))
    } catch (e: CancellationException) {
      throw e
    } catch (e: Exception) {
      val error = pairingError(e)
      if (error != PairingError.Unreachable || relays == RelayChoice.Off) throw error
      // No answer and no relay to carry it: the link names none, or this device never reached its own. Only a LAN
      // path was left
      val named = runCatching { decodeLink(link).relay }.getOrNull()
      if (named == null || !node.relayConnected()) throw PairingError.NoRelay
      throw error
    }
  }

  override suspend fun pair(nearby: NearbyComputer, code: String): PairedComputer {
    val node = pairingNode()
    try {
      val at = ComputerAddr(nearby.id, null, nearby.addrs)
      val p = node.pairCode(at, code, deviceName(), deviceType(), appVersion(), PAIR_MS.toULong())
      // The addresses the code exchange just reached it on go first: the likeliest to answer the first dial
      val addrs = nearby.addrs.filter { it in p.addrs } + p.addrs.filter { it !in nearby.addrs }
      return keep(PairedComputer(id = p.id, name = p.name, model = nearby.model, relay = p.relay, addrs = addrs))
    } catch (e: CancellationException) {
      throw e
    } catch (e: Exception) {
      throw pairingError(e)
    }
  }

  override fun nearby(): Flow<List<NearbyComputer>> = NearbyBrowser.stream(context)

  override suspend fun remove(id: String) {
    val closing = lock.withLock {
      list = list.filter { it.id != id }
      runCatching { store.save(list) }
      live.values.filter { it.computerId == id }
    }
    closing.forEach { it.close() }
  }

  override suspend fun connect(computer: PairedComputer): PairedConnection {
    if (network?.satisfied == false) throw ConnectionError.Other(NO_NETWORK)
    val node = try {
      liveNode()
    } catch (e: CancellationException) {
      throw e
    } catch (e: Exception) {
      throw connectionError(e)
    }
    // The stored entry carries the addresses later links reported; the caller's copy may be older
    val (known, within) = lock.withLock {
      dialing[computer.id] = (dialing[computer.id] ?: 0) + 1
      (list.firstOrNull { it.id == computer.id } ?: computer) to probeMs(computer.id)
    }
    val link: Link = try {
      node.connect(addr(known), SidecarProtocol.VERSION.toLong(), within.toULong())
    } catch (e: CancellationException) {
      throw e
    } catch (e: Exception) {
      // The short LAN deadline was a guess: this Wi-Fi may not be the computer's, so the next dial waits for the relay
      if (within < RELAY_PROBE_MS) lock.withLock { lastPath.remove(computer.id) }
      throw connectionError(e)
    } finally {
      lock.withLock { dialing[computer.id] = (dialing[computer.id] ?: 1) - 1 }
    }
    val id = computer.id
    val key = UUID.randomUUID().toString()
    val hooks = RemoteConnection.Hooks(
      path = { kind -> scope.launch { lock.withLock { lastPath[id] = kind } } },
      addresses = { relay, addrs -> scope.launch { moved(id, relay, addrs) } },
      ended = { scope.launch { lock.withLock { live.remove(key) } } },
    )
    val connection = RemoteConnection(link, id, scope, hooks)
    lock.withLock {
      live[key] = connection
      if (known.name != connection.computerName) {
        list = list.map { if (it.id == id) it.copy(name = connection.computerName) else it }
        runCatching { store.save(list) }
      }
    }
    return connection
  }

  /**
   * A live connection answers for its computer; otherwise a link is dialed and greeted within the probe deadline and
   * closed again. The gateway starts no engine for it: that waits for a hello, which a probe never sends
   */
  override suspend fun presence(computer: PairedComputer): Presence? {
    val (wire, busy, known, within) = lock.withLock {
      Quad(live.values.firstOrNull { it.computerId == computer.id }, (dialing[computer.id] ?: 0) > 0,
        list.firstOrNull { it.id == computer.id } ?: computer, probeMs(computer.id))
    }
    if (wire != null) return wire.path?.let { Presence.Online(it) }
    // Its own dial will tell; a probe now would replace that connection on the gateway
    if (busy) return null
    if (network?.satisfied == false) return Presence.Offline
    val node = runCatching { liveNode() }.getOrNull() ?: return null
    return try {
      val link = node.connect(addr(known), SidecarProtocol.VERSION.toLong(), within.toULong())
      try {
        // iroh names the selected path right after the handshake
        val path = link.path() ?: withTimeoutOrNull(PATH_WAIT_MS) { link.nextPath() }
        val kind = path?.let(::pathKind) ?: lock.withLock { lastPath[computer.id] } ?: PathKind.relay
        lock.withLock { lastPath[computer.id] = kind }
        Presence.Online(kind)
      } finally {
        link.end()
      }
    } catch (e: CancellationException) {
      throw e
    } catch (e: Exception) {
      when (connectionError(e)) {
        ConnectionError.Offline -> Presence.Offline
        ConnectionError.Revoked -> Presence.Revoked
        else -> null
      }
    }
  }

  // Lifecycle

  /** After the grace, unless the app came back meanwhile, every connection ends and the node closes */
  override fun enterBackground() {
    background = true
    val since = System.nanoTime()
    scope.launch {
      lock.withLock {
        backgroundSince = since
        backgroundClose?.cancel()
        backgroundClose = scope.launch {
          delay(BACKGROUND_GRACE_MS)
          val still = lock.withLock { background && backgroundSince == since }
          if (still) dropNode(rebuildLater = true)
        }
      }
    }
  }

  /**
   * Within the grace the node and its connections never closed: iroh hears the network may have moved and every
   * connection is probed. Later, a fresh node from the same key
   */
  override fun enterForeground() {
    background = false
    scope.launch {
      val (n, away) = lock.withLock {
        backgroundClose?.cancel()
        backgroundClose = null
        val away = backgroundSince?.let { (System.nanoTime() - it) / 1_000_000 }
        backgroundSince = null
        node to away
      }
      // Never left: Android starts the activity (a foreground notice) on launch too, while the first dial may be
      // starting the node; dropping it here cancelled that dial
      if (away == null) return@launch
      if (n != null && away < BACKGROUND_GRACE_MS) {
        n.networkChanged()
        probeLive()
        return@launch
      }
      val wanted = lock.withLock { rebuild } || n != null
      dropNode(rebuildLater = false)
      if (wanted) runCatching { liveNode() }
    }
  }

  /**
   * The deadline a connect or a probe gets: an answer on the LAN comes within milliseconds, so 3 s without one means
   * gone; anything else may be a relay or a punched hole still settling. Called under [lock]
   */
  private fun probeMs(id: String): Long =
    if (lastPath[id] == PathKind.lan && network?.local != false) LAN_PROBE_MS else RELAY_PROBE_MS

  // Internals

  private suspend fun dropNode(rebuildLater: Boolean) {
    val (closing, n) = lock.withLock {
      val closing = live.values.toList()
      live.clear()
      starting?.cancel()
      starting = null
      val n = node
      if (n != null) {
        node = null
        rebuild = rebuildLater
      }
      closing to n
    }
    closing.forEach { it.fail(ConnectionError.Suspended) }
    n?.shutdown()
  }

  private suspend fun pairingNode(): Node = try {
    liveNode()
  } catch (e: CancellationException) {
    throw e
  } catch (e: Exception) {
    throw PairingError.Other(e.message ?: e.toString())
  }

  private suspend fun liveNode(): Node {
    if (background) throw ConnectionError.Suspended
    val task = lock.withLock {
      watchSystem()
      node?.let { return it }
      starting ?: scope.async {
        val key = identityKey()
        startNode(key, relays)
      }.also { starting = it }
    }
    val n = try {
      task.await()
    } catch (e: CancellationException) {
      // The start was cancelled under this caller (the app went to the background): a suspended dial, not a silent end
      if (currentCoroutineContext().isActive) throw ConnectionError.Suspended
      throw e
    } catch (e: Exception) {
      lock.withLock { if (starting === task) starting = null }
      throw e
    }
    // Sent to the background while it started
    if (background) {
      lock.withLock { if (starting === task) starting = null }
      n.shutdown()
      throw ConnectionError.Suspended
    }
    return lock.withLock {
      if (starting === task) starting = null
      node ?: n.also { node = it }
    }
  }

  private fun identityKey(): ByteArray {
    keys.load()?.takeIf { it.size == 32 }?.let { return it }
    val key = generateKey()
    keys.save(key)
    return key
  }

  private suspend fun keep(computer: PairedComputer): PairedComputer = lock.withLock {
    list = list.filter { it.id != computer.id } + computer
    try {
      store.save(list)
    } catch (e: Exception) {
      throw PairingError.Other(e.message ?: e.toString())
    }
    computer
  }

  private suspend fun moved(id: String, relay: String?, addrs: List<String>) = lock.withLock {
    list = list.map { if (it.id == id) it.copy(relay = relay ?: it.relay, addrs = addrs.ifEmpty { it.addrs }) else it }
    runCatching { store.save(list) }
  }

  /** The default network's changes; started with the first node. Called under [lock] */
  private fun watchSystem() {
    if (watch != null) return
    watch = NetworkWatch(context) { state -> scope.launch { networkChanged(state) } }
  }

  private suspend fun networkChanged(state: NetworkState) {
    val before = network
    network = state
    // The first report is the network as it is, not a change
    if (before == null || before == state) return
    val n = lock.withLock { node }
    n?.networkChanged()
    if (!state.satisfied) {
      val since = System.nanoTime()
      lock.withLock { if (offlineSince == null) offlineSince = since }
      scope.launch {
        delay(OFFLINE_GRACE_MS)
        stayedOffline(since)
      }
      return
    }
    lock.withLock {
      offlineSince = null
      // The LAN a computer was reached on may not be this one: the next dial waits for the relay
      lastPath.entries.removeAll { it.value == PathKind.lan }
    }
    probeLive()
  }

  /** The network dropped away at [since] and has not come back: the connections end, and the screens say so */
  private suspend fun stayedOffline(since: Long) {
    val closing = lock.withLock {
      val at = offlineSince
      if (at == null || at > since || network?.satisfied != false) return
      live.values.toList()
    }
    closing.forEach { it.fail(ConnectionError.Other(NO_NETWORK)) }
  }

  /** Every connection asked whether it still carries anything, with the relay's deadline; a silent one ends as offline */
  private suspend fun probeLive() {
    val wires = lock.withLock { live.values.toList() }
    for (c in wires) scope.launch { if (!c.alive(RELAY_PROBE_MS)) c.fail(ConnectionError.Offline) }
  }

  private fun addr(c: PairedComputer) = ComputerAddr(c.id, c.relay, c.addrs)

  /** The name the computer's device list shows: the one set in the system's settings, else the model */
  private fun deviceName(): String =
    runCatching { Settings.Global.getString(context.contentResolver, Settings.Global.DEVICE_NAME) }.getOrNull()?.takeIf { it.isNotBlank() }
      ?: Build.MODEL

  private fun deviceType(): DeviceType =
    if (context.resources.configuration.smallestScreenWidthDp >= TABLET_DP) DeviceType.TABLET else DeviceType.PHONE

  private fun appVersion(): String = runCatching {
    context.packageManager.getPackageInfo(context.packageName, 0).versionName
  }.getOrNull() ?: "0"

  private data class Quad<A, B, C, D>(val a: A, val b: B, val c: C, val d: D)

  companion object {
    /** What a connection reports while the device has no network at all */
    const val NO_NETWORK = "no network"
    const val BACKGROUND_GRACE_MS = 20_000L
    /** A network that dropped away entirely may stay away this long before the connections end */
    const val OFFLINE_GRACE_MS = 10_000L
    const val LAN_PROBE_MS = 3_000L
    /** The probe deadline once the network moved: the way through the relay takes longer than a LAN answer */
    const val RELAY_PROBE_MS = 15_000L
    const val PAIR_MS = 20_000L
    const val PATH_WAIT_MS = 1_000L
    const val TABLET_DP = 600
  }
}
