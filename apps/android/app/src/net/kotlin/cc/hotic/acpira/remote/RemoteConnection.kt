package cc.hotic.acpira.remote

import cc.hotic.acpira.sync.ConnectionError
import cc.hotic.acpira.sync.PairedBlobs
import cc.hotic.acpira.sync.PairedConnection
import cc.hotic.acpira.sync.PairingError
import cc.hotic.acpira.sync.PathKind
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.flow.map
import kotlinx.coroutines.flow.takeWhile
import kotlinx.coroutines.launch
import uniffi.acpira_mobile.BlobProgress
import uniffi.acpira_mobile.Link
import uniffi.acpira_mobile.LinkEvent
import uniffi.acpira_mobile.LinkFailure
import uniffi.acpira_mobile.LinkPath
import uniffi.acpira_mobile.PairFailure

/**
 * A wire to a paired computer's engine over an acpira-mobile link (apps/ios AcpiraRemote/RemoteConnection.swift):
 * envelope lines both ways and the path the link travels on. One connection is one link; a lost link is not replayed,
 * the caller connects again and starts from hello
 */
class RemoteConnection internal constructor(
  private val link: Link,
  /** The computer's EndpointId */
  val computerId: String,
  private val scope: CoroutineScope,
  private val hooks: Hooks,
) : PairedConnection, PairedBlobs {
  /** The computer's name as it greeted this link */
  val computerName: String = link.welcome().name

  class Hooks(
    val path: (PathKind) -> Unit = {},
    val addresses: (relay: String?, addrs: List<String>) -> Unit = { _, _ -> },
    val ended: () -> Unit = {},
  )

  /** The current path, null until iroh selected one; [ENDED] once the link is over */
  private val pathNow = MutableStateFlow<Any?>(null)
  /** Why this side ended the link (a dead probe, the app going to the background), reported instead of a clean end */
  @Volatile private var failure: ConnectionError? = null
  @Volatile private var ended = false
  /** When the link last brought anything in (monotonic ms) */
  @Volatile private var heard = now()
  /** Lines in posting order: [send] cannot suspend, the link's write does */
  private val outgoing = Channel<String>(Channel.UNLIMITED)

  init {
    scope.launch { followPaths() }
    scope.launch {
      for (line in outgoing) {
        try {
          link.send(line)
        } catch (e: CancellationException) {
          throw e
        } catch (_: Exception) {
          // A failed write ends nothing by itself: the reader sees the link end
        }
      }
    }
  }

  override fun frames(): Flow<String> = callbackFlow {
    val pump = launch {
      try {
        while (true) {
          val event = link.recv() ?: break
          heard = now()
          when (event) {
            is LinkEvent.Envelope -> send(event.line)
            is LinkEvent.Addresses -> hooks.addresses(event.relay, event.addrs)
            // The engine never saw it, so no answer comes; nothing the phone sends today is refused
            is LinkEvent.Denied -> {}
          }
        }
        val f = failure
        if (f != null) close(f) else close()
      } catch (e: CancellationException) {
        throw e
      } catch (e: Exception) {
        val f = synchronized(this@RemoteConnection) { failure ?: connectionError(e).also { failure = it } }
        close(f)
      }
      endOnce()
    }
    awaitClose {
      pump.cancel()
      link.end()
    }
  }

  override fun send(text: String): Boolean = !ended && outgoing.trySend(text).isSuccess

  override fun close() {
    outgoing.close()
    link.end()
  }

  /** The path the link travels on now, once iroh selected one */
  val path: PathKind? get() = pathNow.value as? PathKind

  override fun paths(): Flow<PathKind> = pathNow.takeWhile { it !== ENDED }.filterNotNull().map { it as PathKind }

  override fun endReason(): ConnectionError? = failure

  override suspend fun blob(session: String, name: String): ByteArray = try {
    link.fetchBlob(session, name, BLOB_WITHIN_MS.toULong())
  } catch (e: CancellationException) {
    throw e
  } catch (e: Exception) {
    throw connectionError(e)
  }

  override suspend fun download(session: String, name: String, to: java.io.File, progress: (have: Long, total: Long) -> Unit): Long = try {
    val heard = object : BlobProgress {
      override fun progress(have: ULong, total: ULong) = progress(have.toLong(), total.toLong())
    }
    link.fetchBlobTo(session, name, to.path, DOWNLOAD_IDLE_MS.toULong(), heard).toLong()
  } catch (e: CancellationException) {
    throw e
  } catch (e: Exception) {
    throw connectionError(e)
  }

  /** A round trip through the gateway within the deadline; no answer throws [ConnectionError.Offline] */
  suspend fun probe(withinMs: Long): Long = try {
    link.ping(withinMs.toULong()).toLong()
  } catch (e: CancellationException) {
    throw e
  } catch (e: Exception) {
    throw connectionError(e)
  }

  /**
   * Whether the link still carries anything: the probe's answer, or any frame that came in while it waited. The pong
   * queues behind every envelope the gateway sent first, so a link busy delivering a large view may miss the deadline
   */
  suspend fun alive(withinMs: Long): Boolean {
    val asked = now()
    if (runCatching { probe(withinMs) }.isSuccess) return true
    return !ended && failure == null && heard > asked
  }

  /** Ends the link from this side, reporting [error] to the reader of [frames] */
  fun fail(error: ConnectionError) {
    synchronized(this) { if (failure == null) failure = error }
    close()
  }

  private suspend fun followPaths() {
    link.path()?.let { publish(pathKind(it)) }
    while (true) {
      val next = link.nextPath() ?: break
      publish(pathKind(next))
    }
    pathNow.value = ENDED
    // The link is over even when nobody reads frames()
    endOnce()
  }

  private fun endOnce() {
    val first = synchronized(this) { (!ended).also { ended = true } }
    if (first) {
      outgoing.close()
      hooks.ended()
    }
  }

  private fun publish(kind: PathKind) {
    if (pathNow.value == kind) return
    pathNow.value = kind
    hooks.path(kind)
  }

  private companion object {
    val ENDED = Any()
    /** A whole blob (an image the transcript shows) must arrive within this */
    const val BLOB_WITHIN_MS = 30_000L
    /**
     * How long a download may go without a byte before it stops (and resumes on the next try): a relay path can stall
     * for seconds while the link itself is fine
     */
    const val DOWNLOAD_IDLE_MS = 20_000L
    fun now() = System.nanoTime() / 1_000_000
  }
}

internal fun pathKind(path: LinkPath): PathKind = when (path) {
  LinkPath.LAN -> PathKind.lan
  LinkPath.TAILSCALE -> PathKind.tailscale
  LinkPath.DIRECT -> PathKind.direct
  LinkPath.RELAY -> PathKind.relay
}

/** A `LinkFailure` as the screens tell it; anything else is [ConnectionError.Other] */
internal fun connectionError(e: Throwable): ConnectionError = when (e) {
  is ConnectionError -> e
  is LinkFailure.Offline -> ConnectionError.Offline
  is LinkFailure.Revoked -> ConnectionError.Revoked
  is LinkFailure.Incompatible -> ConnectionError.Incompatible(e.detail)
  is LinkFailure.Behind -> ConnectionError.Other("This device fell behind; reconnect")
  is LinkFailure.Engine -> ConnectionError.Other(e.detail)
  is LinkFailure.Other -> ConnectionError.Other(e.detail)
  else -> ConnectionError.Other(e.message ?: e.toString())
}

internal fun pairingError(e: Throwable): PairingError = when (e) {
  is PairingError -> e
  is PairFailure.Closed -> PairingError.Closed
  is PairFailure.Expired -> PairingError.Expired
  is PairFailure.Used -> PairingError.Used
  is PairFailure.Wrong -> PairingError.Wrong
  is PairFailure.InvalidLink -> PairingError.InvalidLink
  is PairFailure.Unreachable -> PairingError.Unreachable
  is PairFailure.Impostor -> PairingError.Impostor
  is PairFailure.Other -> PairingError.Other(e.detail)
  else -> PairingError.Other(e.message ?: e.toString())
}
