package cc.hotic.acpira.remote

import android.content.Context
import android.net.ConnectivityManager
import android.net.LinkProperties
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.net.nsd.NsdManager
import android.net.nsd.NsdServiceInfo
import android.os.Build
import cc.hotic.acpira.sync.NearbyComputer
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.callbackFlow
import java.util.concurrent.ConcurrentHashMap

/** What the service needs to know about this device's network (apps/ios NetworkWatch.swift `NetworkState`) */
data class NetworkState(
  /** Any route out at all */
  val satisfied: Boolean,
  /** On Wi-Fi or Ethernet: a computer last reached on the LAN may be on this one */
  val local: Boolean,
  /** The default network's interface and routers: a different set (Wi-Fi to cellular, another Wi-Fi) is a change */
  val interfaces: List<String>,
  val gateways: List<String>,
)

/** The default network, reported on every change (the first report is the network as it is) */
class NetworkWatch(context: Context, private val changed: (NetworkState) -> Unit) {
  private val manager = context.getSystemService(ConnectivityManager::class.java)
  private val callback = object : ConnectivityManager.NetworkCallback() {
    override fun onAvailable(network: Network) = report()
    override fun onLost(network: Network) = report()
    override fun onCapabilitiesChanged(network: Network, caps: NetworkCapabilities) = report()
    override fun onLinkPropertiesChanged(network: Network, props: LinkProperties) = report()
  }

  init {
    manager.registerDefaultNetworkCallback(callback)
    report()
  }

  private fun report() {
    val network = manager.activeNetwork
    val caps = network?.let { manager.getNetworkCapabilities(it) }
    val props = network?.let { manager.getLinkProperties(it) }
    changed(
      NetworkState(
        satisfied = caps?.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET) == true ||
          caps?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) == true,
        local = caps?.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) == true ||
          caps?.hasTransport(NetworkCapabilities.TRANSPORT_ETHERNET) == true,
        interfaces = listOfNotNull(props?.interfaceName),
        gateways = props?.routes.orEmpty().filter { it.hasGateway() }.map { it.gateway.toString() },
      ),
    )
  }

  fun stop() {
    runCatching { manager.unregisterNetworkCallback(callback) }
  }
}

/**
 * Computers with a pairing window open on this network (apps/ios NearbyBrowser.swift): NSD browses the gateway's
 * `_acpira-pair._udp` advertisement and resolves each record's TXT. A record is a hint only: the code exchange proves
 * the computer's identity, so a forged one gets nothing but a failed pairing
 */
object NearbyBrowser {
  const val SERVICE_TYPE = "_acpira-pair._udp"

  fun stream(context: Context): Flow<List<NearbyComputer>> = callbackFlow {
    val nsd = context.getSystemService(NsdManager::class.java)
    /** Resolved records by service name; a service seen on two interfaces is one computer */
    val found = ConcurrentHashMap<String, NearbyComputer>()
    val watchers = ConcurrentHashMap<String, Any>()
    fun publish() {
      trySend(merged(found.values.toList()))
    }
    fun resolved(info: NsdServiceInfo) {
      val txt = info.attributes.mapValues { (_, v) -> v?.toString(Charsets.UTF_8).orEmpty() }
      val computer = computer(info.serviceName, txt) ?: return
      found[info.serviceName] = computer
      publish()
    }
    val listener = object : NsdManager.DiscoveryListener {
      override fun onDiscoveryStarted(serviceType: String) = publish()
      override fun onDiscoveryStopped(serviceType: String) {}
      override fun onStartDiscoveryFailed(serviceType: String, errorCode: Int) {
        close()
      }
      override fun onStopDiscoveryFailed(serviceType: String, errorCode: Int) {}
      override fun onServiceFound(info: NsdServiceInfo) {
        if (Build.VERSION.SDK_INT >= 34) {
          // Follows the record: its TXT and addresses as they change, until the service goes
          val cb = object : NsdManager.ServiceInfoCallback {
            override fun onServiceInfoCallbackRegistrationFailed(errorCode: Int) {}
            override fun onServiceUpdated(updated: NsdServiceInfo) = resolved(updated)
            override fun onServiceLost() {
              found.remove(info.serviceName)
              publish()
            }
            override fun onServiceInfoCallbackUnregistered() {}
          }
          watchers.put(info.serviceName, cb)?.let { runCatching { nsd.unregisterServiceInfoCallback(it as NsdManager.ServiceInfoCallback) } }
          nsd.registerServiceInfoCallback(info, context.mainExecutor, cb)
        } else {
          @Suppress("DEPRECATION")
          nsd.resolveService(info, object : NsdManager.ResolveListener {
            override fun onResolveFailed(info: NsdServiceInfo, errorCode: Int) {}
            override fun onServiceResolved(info: NsdServiceInfo) = resolved(info)
          })
        }
      }
      override fun onServiceLost(info: NsdServiceInfo) {
        found.remove(info.serviceName)
        if (Build.VERSION.SDK_INT >= 34) {
          watchers.remove(info.serviceName)?.let { runCatching { nsd.unregisterServiceInfoCallback(it as NsdManager.ServiceInfoCallback) } }
        }
        publish()
      }
    }
    nsd.discoverServices(SERVICE_TYPE, NsdManager.PROTOCOL_DNS_SD, listener)
    awaitClose {
      runCatching { nsd.stopServiceDiscovery(listener) }
      if (Build.VERSION.SDK_INT >= 34) {
        watchers.values.forEach { runCatching { nsd.unregisterServiceInfoCallback(it as NsdManager.ServiceInfoCallback) } }
      }
    }
  }

  fun computer(name: String, txt: Map<String, String>): NearbyComputer? {
    if (txt["v"] != "1") return null
    val id = txt["id"]?.trim().orEmpty()
    if (id.isEmpty()) return null
    val addrs = txt["a"].orEmpty().split(',').map { it.trim() }.filter { it.isNotEmpty() }
    return NearbyComputer(id = id, name = name, model = txt["m"]?.takeIf { it.isNotEmpty() }, addrs = addrs)
  }

  /** One entry per computer, by name then id */
  fun merged(found: List<NearbyComputer>): List<NearbyComputer> {
    val byId = LinkedHashMap<String, NearbyComputer>()
    for (c in found) {
      val known = byId[c.id]
      byId[c.id] = if (known == null) c else known.copy(addrs = known.addrs + c.addrs.filter { it !in known.addrs }, model = known.model ?: c.model)
    }
    return byId.values.sortedWith(compareBy(String.CASE_INSENSITIVE_ORDER, NearbyComputer::name).thenBy { it.id })
  }
}
