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
import cc.hotic.acpira.store.openLink
import cc.hotic.acpira.sync.ConnectionError
import cc.hotic.acpira.sync.PairedComputer
import cc.hotic.acpira.sync.PairedConnection

/** The one activity: edge to edge, Compose draws every screen (ui/Root.kt) */
class MainActivity : ComponentActivity() {
  /** The model the screens follow: the demo, the development engine, or the followed computer's current wire */
  private var model by mutableStateOf<AppModel?>(null)
  /** Paired remote access; null for a development engine, a demo launch, or a build without acpira-mobile */
  private var pairing: PairingModel? = null
  /** Notices from the followed computer's list (store/Attention.kt) */
  private val attention by lazy { cc.hotic.acpira.store.Attention(applicationContext) }
  /** The activity is on screen: the chat it shows notifies nothing */
  private var inFront = false
  /** The running-turn service holds the wire while the app is away; the background grace starts once it lets go */
  private var holding = false

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
      // A notice tapped before the paired computers were read follows its computer once they are
      lifecycleScope.launch {
        snapshotFlow { pairing.loaded }.first { it }
        openNotified()
      }
    }
    followAttention()
    openNotice(intent)
    openLink(intent)
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
    // A tapped notice from this computer opens its chat on the new wire; one from another computer keeps waiting
    pendingOpen?.takeIf { it.computer == null || it.computer == computer.id }?.let { open ->
      pendingOpen = null
      if (next.path.lastOrNull() != cc.hotic.acpira.store.Screen.Chat(open.session)) next.open(cc.hotic.acpira.store.Screen.Chat(open.session))
    }
  }

  override fun onNewIntent(intent: Intent) {
    super.onNewIntent(intent)
    openLink(intent)
    openNotice(intent)
  }

  /**
   * Every list the followed model brings goes past [attention]; while the service holds the wire for running turns,
   * the count keeps its notice current, and once nothing runs the service lets go and the background grace starts
   */
  private fun followAttention() {
    val ask = registerForActivityResult(ActivityResultContracts.RequestPermission()) {}
    lifecycleScope.launch {
      snapshotFlow { model?.let { Triple(it, it.summaries, it.connection) } }.collect { now ->
        val (app, list, connection) = now ?: return@collect
        val shown = if (inFront) (app.path.lastOrNull { it is cc.hotic.acpira.store.Screen.Chat } as cc.hotic.acpira.store.Screen.Chat?)?.id else null
        attention.update(app, list, shown)
        shown?.let(attention::clear)
        // The first live wire asks once for notifications (iOS asks on its first wire too)
        if (connection == cc.hotic.acpira.store.Connection.Live) askNotifications(ask)
        if (holding) {
          val running = running(app)
          if (running == 0) letGo() else cc.hotic.acpira.store.AttentionService.start(this@MainActivity, running, pairing?.selected?.name)
        }
      }
    }
  }

  private fun running(app: AppModel) = app.summaries.count { it.state == cc.hotic.acpira.model.SessionState.working }

  private fun askNotifications(ask: androidx.activity.result.ActivityResultLauncher<String>) {
    if (android.os.Build.VERSION.SDK_INT < android.os.Build.VERSION_CODES.TIRAMISU) return
    val prefs = getSharedPreferences(NOTIFY_PREFS, MODE_PRIVATE)
    // The first-run flow asks in its own step (ui/Welcome.kt), after the wire that pairing brings up
    if (pairing?.onboarding == true) return
    if (prefs.getBoolean(NOTIFY_ASKED, false) || cc.hotic.acpira.store.Attention.allowed(this)) return
    prefs.edit().putBoolean(NOTIFY_ASKED, true).apply()
    ask.launch(android.Manifest.permission.POST_NOTIFICATIONS)
  }

  /** The service ends; the wire gets the grace a plain trip to the background gives it */
  private fun letGo() {
    if (!holding) return
    holding = false
    cc.hotic.acpira.store.AttentionService.stop(this)
    if (!inFront) pairing?.service?.enterBackground()
  }

  /** A tapped notice: the session it names on the computer it came from */
  private fun openNotice(intent: Intent?) {
    if (intent?.action != cc.hotic.acpira.store.Attention.ACTION_OPEN) return
    val id = intent.getStringExtra(cc.hotic.acpira.store.Attention.EXTRA_SESSION) ?: return
    pendingOpen = Notice(intent.getStringExtra(cc.hotic.acpira.store.Attention.EXTRA_COMPUTER), id)
    openNotified()
  }

  /**
   * A tapped notice (apps/ios RootView `openNotified`): its chat at once when its computer is on screen, else follow that
   * computer and let the new wire open it (`adopt`). One from a computer no longer paired is dropped
   */
  private fun openNotified() {
    val open = pendingOpen ?: return
    val app = model ?: return
    val pairing = pairing
    // A development engine has no pairing: the session is that engine's
    if (pairing == null || (app.computer != null && app.computer == open.computer)) {
      pendingOpen = null
      if (app.path.lastOrNull() != cc.hotic.acpira.store.Screen.Chat(open.session)) app.open(cc.hotic.acpira.store.Screen.Chat(open.session))
      return
    }
    // Until the paired computers are read, it waits (onCreate calls again once they are)
    if (!pairing.loaded) return
    if (pairing.computers.none { it.id == open.computer }) {
      pendingOpen = null
      return
    }
    open.computer?.let { if (pairing.selected?.id != it) pairing.select(it) }
  }

  /** A tapped notice waiting for its computer's wire: the computer (null: a development engine) and the session */
  private data class Notice(val computer: String?, val session: String)

  private var pendingOpen: Notice? = null

  override fun onPause() {
    super.onPause()
    // Still visible here, where a foreground service may start: turns still running keep the wire up until they end
    val app = model ?: return
    val running = running(app)
    if (running > 0 && cc.hotic.acpira.store.Attention.allowed(this)) {
      holding = cc.hotic.acpira.store.AttentionService.start(this, running, pairing?.selected?.name)
    }
  }

  override fun onResume() {
    super.onResume()
    if (holding) {
      holding = false
      cc.hotic.acpira.store.AttentionService.stop(this)
    }
  }

  /**
   * `acpira://pair/…` opened from outside: the add-computer sheet confirms it. A session link opens its chat, on a paired
   * build once the computer's lists are in (store/Links.kt)
   */
  private fun openLink(intent: Intent?) {
    val link = intent?.dataString ?: return
    if (cc.hotic.acpira.logic.SessionLinks.idOfLink(link) != null) {
      lifecycleScope.launch {
        val app = snapshotFlow { model?.takeIf { pairing == null || it.synced } }.first { it != null }
        app?.openLink(link)
      }
      return
    }
    pairing?.open(link)
  }

  override fun onStart() {
    super.onStart()
    inFront = true
    model?.setAway(false)
    val pairing = pairing ?: return
    pairing.service.enterForeground()
    // Back from the background, where the service closed the wire: dial the followed computer again
    if (pairing.loaded && pairing.connection == null && pairing.selected != null && pairing.link != PairLink.Connecting) pairing.connectSelected()
  }

  override fun onStop() {
    // The running clock stands from here until the engine's view comes again (AppModel `clockHeld`)
    inFront = false
    model?.setAway(true)
    // While the running-turn service holds the wire, the grace starts only once it lets go
    if (!holding) pairing?.service?.enterBackground()
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

  private companion object {
    const val NOTIFY_PREFS = "notify"
    const val NOTIFY_ASKED = "asked"
  }

  override fun onDestroy() {
    if (isFinishing) {
      cc.hotic.acpira.store.AttentionService.stop(this)
      model?.close()
    }
    super.onDestroy()
  }
}
