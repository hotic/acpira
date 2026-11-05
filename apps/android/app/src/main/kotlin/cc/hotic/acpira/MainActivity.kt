package cc.hotic.acpira

import android.content.Intent
import android.os.Bundle
import android.util.Log
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.snapshotFlow
import androidx.lifecycle.lifecycleScope
import cc.hotic.acpira.ui.LOCAL_NETWORK
import cc.hotic.acpira.ui.localNetworkGranted
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import cc.hotic.acpira.design.AcpiraTheme
import cc.hotic.acpira.remote.Remote
import cc.hotic.acpira.store.AppModel
import cc.hotic.acpira.store.LaunchOptions
import cc.hotic.acpira.store.PairLink
import cc.hotic.acpira.store.PairingModel
import cc.hotic.acpira.store.Wire
import cc.hotic.acpira.sync.ConnectionError
import cc.hotic.acpira.sync.PairedComputer
import cc.hotic.acpira.sync.PairedConnection

/** The one activity: edge to edge, Compose draws every screen (ui/Root.kt) */
class MainActivity : ComponentActivity() {
  /** The model the screens follow: the demo, the development engine, or the followed computer's current wire */
  private var model by mutableStateOf<AppModel?>(null)
  /** Paired remote access; null for a development engine, a demo launch, or a build without acpira-mobile */
  private var pairing: PairingModel? = null

  override fun onCreate(savedInstanceState: Bundle?) {
    val options = LaunchOptions.from(intent)
    val service = if (options.engine == null && !options.demo) Remote.service(applicationContext) else null
    val pairing = service?.let { PairingModel(it, applicationContext, lifecycleScope) }
    this.pairing = pairing
    // A paired build launched bare shows no fixtures: the list stands empty under the connect page until a computer's
    // wire brings its own model
    val first = if (pairing != null) AppModel(applicationContext, options, lifecycleScope, Wire.Blank) else AppModel(applicationContext, options, lifecycleScope)
    model = first
    enableEdgeToEdge(first.theme)
    super.onCreate(savedInstanceState)
    // Settings → Developer's frame-rate and hitch overlay, in a window of its own above every screen and sheet
    cc.hotic.acpira.design.PerfHud.install(this)
    if (pairing != null) {
      // A computer paired before (on an earlier release, or with the permission later taken back): its LAN path needs
      // Android 17's local network grant; asked once here, the relay carries the wire meanwhile
      val ask = registerForActivityResult(ActivityResultContracts.RequestPermission()) { if (it) pairing.connectSelected() }
      lifecycleScope.launch {
        snapshotFlow { pairing.loaded && pairing.computers.isNotEmpty() }.first { it }
        if (!localNetworkGranted(this@MainActivity)) ask.launch(LOCAL_NETWORK)
      }
      pairing.onConnection = { computer, wire -> adopt(options, computer, wire) }
      val debuggable = applicationInfo.flags and android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE != 0
      options.nearby?.takeIf { debuggable }?.split('@', limit = 2)?.takeIf { it.size == 2 }?.let { (id, addr) ->
        pairing.pinnedNearby = cc.hotic.acpira.sync.NearbyComputer(id = id, name = addr, addrs = listOf(addr))
      }
      pairing.load()
      openLink(intent)
    }
    setContent {
      val app = model ?: return@setContent
      // A theme pick restyles the bars too; a language pick rebuilds every screen in the new table (the model keeps
      // the stack, so the reader stays where they were)
      LaunchedEffect(app.theme) { enableEdgeToEdge(app.theme) }
      key(app.language) {
        AcpiraTheme(dark = darkOf(app.theme) ?: isSystemInDarkTheme()) {
          cc.hotic.acpira.ui.Root(app, pairing)
        }
      }
    }
  }

  /**
   * A paired computer's wire becomes the model the screens follow; its end goes back to the pairing model, whose screens
   * say why and offer a retry. A new wire to the computer already on screen keeps the screens (`resume`)
   */
  private fun adopt(options: LaunchOptions, computer: PairedComputer, wire: PairedConnection) {
    val pairing = pairing ?: return
    val previous = model
    val next = AppModel(applicationContext, options, lifecycleScope, Wire.Paired(computer.id, wire))
    if (previous != null && previous.computer == computer.id && (previous.synced || previous.resumed)) next.resume(previous)
    next.onClosed = { reason ->
      Log.i("acpira", "paired wire ended: $reason")
      pairing.connectionEnded(wire.endReason() ?: ConnectionError.Offline, wire)
    }
    model = next
    previous?.close()
  }

  override fun onNewIntent(intent: Intent) {
    super.onNewIntent(intent)
    openLink(intent)
  }

  /** `acpira://pair/…` opened from outside: the add-computer sheet confirms it */
  private fun openLink(intent: Intent?) {
    val link = intent?.dataString ?: return
    pairing?.open(link)
  }

  override fun onStart() {
    super.onStart()
    model?.setAway(false)
    val pairing = pairing ?: return
    pairing.service.enterForeground()
    // Back from the background, where the service closed the wire: dial the followed computer again
    if (pairing.loaded && pairing.connection == null && pairing.selected != null && pairing.link != PairLink.Connecting) pairing.connectSelected()
  }

  override fun onStop() {
    // The running clock stands from here until the engine's view comes again (AppModel `clockHeld`)
    model?.setAway(true)
    pairing?.service?.enterBackground()
    super.onStop()
  }

  /** Bars stay transparent; their icons follow the theme the app draws in */
  private fun enableEdgeToEdge(theme: String) {
    val bars = when (darkOf(theme)) {
      true -> SystemBarStyle.dark(android.graphics.Color.TRANSPARENT)
      false -> SystemBarStyle.light(android.graphics.Color.TRANSPARENT, android.graphics.Color.TRANSPARENT)
      null -> SystemBarStyle.auto(android.graphics.Color.TRANSPARENT, android.graphics.Color.TRANSPARENT)
    }
    enableEdgeToEdge(statusBarStyle = bars, navigationBarStyle = bars)
  }

  private fun darkOf(theme: String): Boolean? = when (theme) {
    "dark" -> true
    "light" -> false
    else -> null
  }

  override fun onDestroy() {
    if (isFinishing) model?.close()
    super.onDestroy()
  }
}
