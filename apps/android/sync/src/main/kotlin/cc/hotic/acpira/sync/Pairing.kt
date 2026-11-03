package cc.hotic.acpira.sync

import kotlinx.coroutines.flow.Flow
import kotlinx.serialization.Serializable

// The phone's side of paired remote access (apps/ios AcpiraSync/Pairing.swift): the computers it paired with, finding
// one on this network, pairing with a link or a code, and a wire to a paired computer's engine. The pairing screens talk
// to `PairingService` only; the iroh-backed implementation (the app's `net` source set over acpira-mobile) conforms.

/** A computer this phone paired with, kept by the service across launches */
@Serializable
data class PairedComputer(
  /** The computer's EndpointId: its identity, pinned at pairing */
  val id: String,
  /** What the computer calls itself; it may rename itself, and the service updates this on the next connection */
  val name: String,
  /** The computer's `hw.model` when it advertised one (picks the icon) */
  val model: String? = null,
  /** Where the computer was last reachable, handed to iroh on every connect */
  val relay: String? = null,
  val addrs: List<String> = emptyList(),
  /** Epoch milliseconds */
  val pairedAt: Long = System.currentTimeMillis(),
)

/**
 * A computer with its pairing window open on this network. The gateway advertises `_acpira-pair._udp` only while a
 * window is open: instance name = the computer's name, TXT `v=1`, `id=<EndpointId>`, `a=<ip:port,...>`, `m=<hw.model>`
 */
data class NearbyComputer(val id: String, val name: String, val model: String? = null, val addrs: List<String> = emptyList())

/** How a connection travels right now (Rust `acpira_shared::devices::PathKind`) */
enum class PathKind { lan, tailscale, direct, relay }

/** Why pairing did not happen */
sealed class PairingError(message: String) : Exception(message) {
  /** The computer refused: no window open, the code expired, it was used already, the code is wrong */
  data object Closed : PairingError("closed")
  data object Expired : PairingError("expired")
  data object Used : PairingError("used")
  data object Wrong : PairingError("wrong")
  /** Not an `acpira://pair/…` link this version reads */
  data object InvalidLink : PairingError("invalid link")
  /** The computer did not answer in time */
  data object Unreachable : PairingError("unreachable")
  /** No answer and no relay could carry the pairing: only the same network joins the two then */
  data object NoRelay : PairingError("no relay")
  /** The computer answered but could not prove it knows the code: not the computer it claims to be */
  data object Impostor : PairingError("impostor")
  class Other(message: String) : PairingError(message)
}

/** Why a connection to a paired computer ended or never started */
sealed class ConnectionError(message: String) : Exception(message) {
  /** Nothing answered: asleep, off, or remote access turned off */
  data object Offline : ConnectionError("offline")
  /** The computer removed this phone */
  data object Revoked : ConnectionError("revoked")
  /** The two sides speak different protocol versions; the message says which side to update */
  class Incompatible(message: String) : ConnectionError(message)
  /** The app went to the background and closed the wire itself; coming back dials again, so nothing failed */
  data object Suspended : ConnectionError("suspended")
  class Other(message: String) : ConnectionError(message)

  /** A removal or another protocol version will not mend itself; anything else may on the next dial */
  val transient: Boolean get() = this is Offline || this is Other
}

/** A wire to a paired computer's engine: envelopes like any other transport, plus the path it travels on */
interface PairedConnection : EngineTransport {
  /** The current path first, then every change, until the connection ends */
  fun paths(): Flow<PathKind>
  /** Why the connection ended, once it has; null while open or when closed from this side */
  fun endReason(): ConnectionError? = null
}

/** A paired computer's state as a probe saw it */
sealed interface Presence {
  data class Online(val path: PathKind) : Presence
  /** Nothing answered: asleep, off, or remote access turned off */
  data object Offline : Presence
  /** The computer removed this phone */
  data object Revoked : Presence
}

interface PairingService {
  /** Paired computers, most recently paired first */
  suspend fun computers(): List<PairedComputer>
  /** The computer a link names, for the confirmation step before pairing; throws [PairingError.InvalidLink] */
  fun computerName(link: String): String
  /** Pair through a scanned QR code or a pasted link; throws [PairingError] */
  suspend fun pair(link: String): PairedComputer
  /** Pair with a computer found on this network and the 6-digit code its pairing card shows */
  suspend fun pair(nearby: NearbyComputer, code: String): PairedComputer
  /** Computers with their pairing window open on this network, updated as they come and go; browsing stops with the collector */
  fun nearby(): Flow<List<NearbyComputer>>
  /** Forget a computer on this side (the computer keeps its entry until its owner removes the phone) */
  suspend fun remove(id: String)
  /** Open a wire to the computer's engine; throws [ConnectionError] */
  suspend fun connect(computer: PairedComputer): PairedConnection
  /** Whether a paired computer answers right now and over which path, without opening an engine wire. Null: not known */
  suspend fun presence(computer: PairedComputer): Presence? = null
  /** The app went to the background / came back (the service closes its node after a grace, rebuilds it on return) */
  fun enterBackground() {}
  fun enterForeground() {}
}

/** `acpira://pair/…`, as a scan, a paste or a link opened from outside hands it over */
fun isPairingLink(text: String): Boolean = text.trim().lowercase().startsWith("acpira://pair")
