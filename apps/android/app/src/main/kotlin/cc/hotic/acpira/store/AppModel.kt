package cc.hotic.acpira.store

import android.content.Context
import android.util.Log
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import cc.hotic.acpira.design.IconStore
import cc.hotic.acpira.i18n.L10n
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.model.AgentInfo
import cc.hotic.acpira.model.AgentTurn
import cc.hotic.acpira.model.ClientInfo
import cc.hotic.acpira.model.ConfigControl
import cc.hotic.acpira.model.Draft
import cc.hotic.acpira.model.Hello
import cc.hotic.acpira.model.HostMsg
import cc.hotic.acpira.model.PlatformRequest
import cc.hotic.acpira.model.SessionState
import cc.hotic.acpira.model.SessionSummary
import cc.hotic.acpira.model.SessionView
import cc.hotic.acpira.model.SettingsView
import cc.hotic.acpira.model.ShellEnv
import cc.hotic.acpira.model.SidecarProtocol
import cc.hotic.acpira.model.UserTurn
import cc.hotic.acpira.model.WebviewMsg
import cc.hotic.acpira.sync.EngineClient
import cc.hotic.acpira.sync.EngineEvent
import cc.hotic.acpira.sync.EngineTransport
import cc.hotic.acpira.sync.SessionFeed
import cc.hotic.acpira.sync.VIEW_CACHE_ENTRIES
import cc.hotic.acpira.sync.WebSocketTransport
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonPrimitive
import java.time.Instant

/** One screen of the app's stack; the list is the root under every other */
sealed interface Screen {
  data class Chat(val id: String) : Screen
  /** A new session draft: the agent and folder it starts with (null: the defaults) */
  data class New(val agent: String? = null, val cwd: String? = null) : Screen
  data object Settings : Screen
  /** One paired computer's page, from the settings */
  data class Computer(val id: String) : Screen
  /** One agent's settings page, and its models page under it */
  data class AgentSettings(val id: String) : Screen
  data class ModelSettings(val id: String) : Screen
  /** Settings → Developer: the frame-rate and hitch overlay's switches */
  data object Developer : Screen
  /** The built-in agent's model sources, a source's form (id null: a new one; error: why it opened) and its add-model page */
  data object Providers : Screen
  data class ProviderForm(val id: String?, val error: String? = null) : Screen
  data class AddModels(val id: String) : Screen
}

/** The wire to the engine as the screens show it */
sealed interface Connection {
  /** No engine: the synthetic sessions */
  data object Demo : Connection
  data object Connecting : Connection
  data object Live : Connection
  data class Closed(val reason: String?) : Connection
}

/** What a model talks to */
sealed interface Wire {
  /** The synthetic sessions */
  data object Demo : Wire
  /** Nothing yet: a paired build waiting for a computer's wire (the connect page covers the list) */
  data object Blank : Wire
  /** The development engine, `acpira --ws` through adb reverse */
  data class Engine(val address: EngineAddress) : Wire
  /** A paired computer's engine over its link */
  class Paired(val computer: String, val transport: EngineTransport) : Wire
}

/** A prompt sent while the engine has not shown its copy yet: drawn at once, gone once the session has it */
data class Outgoing(val text: String, val usersBefore: Int)

/**
 * The app's state: the engine's lists and views (or the demo's) and the actions the screens post. Mirrors what
 * packages/ui/App.tsx does with the same messages, as apps/ios Store/AppModel.swift does; Compose state, read on the
 * main thread
 */
class AppModel(
  private val context: Context,
  val options: LaunchOptions,
  private val scope: CoroutineScope,
  wire: Wire = options.engine?.let(EngineAddress::parse)?.let { Wire.Engine(it) } ?: Wire.Demo,
) {
  private var agentList by mutableStateOf<List<AgentInfo>>(emptyList())
  /** Every agent the engine lists; their own marks (plugin and agents.json agents) go to the icon store as they come */
  var agents: List<AgentInfo>
    get() = agentList
    set(value) {
      IconStore.adopt(value.mapNotNull { a -> a.mark?.takeIf { it.isNotEmpty() }?.let { a.id to it } }.toMap())
      agentList = value
    }
  var summaries by mutableStateOf<List<SessionSummary>>(emptyList())
  var settings by mutableStateOf(SettingsView())
  /** The views the screens show, by session id */
  val views = mutableStateMapOf<String, SessionView>()
  val outgoing = mutableStateMapOf<String, List<Outgoing>>()
  /** Where chats' folders live (InitState.chats): a session directly inside is a chat, not a project */
  var chatRoot by mutableStateOf<String?>(null)
  var home by mutableStateOf("")
  var projectOrder by mutableStateOf<List<String>>(emptyList())
  /** Folders added by hand, the latest first: ahead of every other folder in the folder menus */
  var addedProjects by mutableStateOf<List<String>>(emptyList())
  /** What the computer's engine offers beyond the protocol (helloOk.features): empty for the demo and older engines */
  var engineFeatures by mutableStateOf<List<String>>(emptyList())
    private set
  /** The computer's folders, for the add-project picker */
  val folders = LocalFolders(::post)
  /** The built-in agent's model sources; a saved change asks for fresh controls so the composer's model menu has it */
  val sources = ModelSources(::post) { post(WebviewMsg.Controls(BUILTIN_AGENT, fresh = true)) }
  var connection by mutableStateOf<Connection>(Connection.Demo)
  /** The latest toast, until it times out */
  var toast by mutableStateOf<String?>(null)
  /** The screens over the list, bottom first */
  val path = mutableStateListOf<Screen>()
  /** The demo's clock: the fixtures' now, running from launch */
  val now: () -> Instant
  /** The paired computer this model's wire leads to (null: the development engine or the demo) */
  val computer: String? = (wire as? Wire.Paired)?.computer
  /** The engine's init has landed on this wire */
  var synced by mutableStateOf(false)
    private set
  /** Took over the screens of the model before it (a new wire to the same computer) */
  var resumed by mutableStateOf(false)
    private set
  /**
   * The moment the screens stopped following the engine live: the app went to the background or the wire ended. The
   * running turn's clock stands at it until the engine's view of the chat on screen comes again, so it never runs on
   * through the absence and then jumps back to the engine's time (apps/ios AppModel `clockHeld`)
   */
  var clockHeld by mutableStateOf<Instant?>(null)
    private set
  /** The app is in the background (`setAway`) */
  private var away = false
  /** The wire ended (its failure, null when closed from this side); the pairing model takes it from here */
  var onClosed: ((String?) -> Unit)? = null
  /** Session actions the wire before may not have delivered, sent once this wire's init is in */
  private var replay: List<WebviewMsg> = emptyList()

  private var client: EngineClient? = null
  private val feed = SessionFeed()
  /**
   * Views carried over from the wire before (`resume`) that this wire has not shown yet, least recently shown first.
   * They stay beside the feed's own until the cache bound needs their room, so a chat opened after a reconnect shows
   * its last copy at once instead of waiting for the whole transcript (apps/ios AppModel `carried`)
   */
  private var carried: List<String> = emptyList()
  /** After newSession: the next session the engine shows that is not `from` is the one it made */
  private var awaiting: Pair<String?, (String) -> Unit>? = null
  private var demoSeq = 0

  val live: Boolean get() = client != null

  // The phone's own look (apps/ios Prefs): theme system | light | dark, language auto | a table id, motion. A launch
  // extra wins for that run and is not stored
  private val prefs = context.getSharedPreferences("appearance", Context.MODE_PRIVATE)
  var theme by mutableStateOf(options.theme ?: prefs.getString("theme", null) ?: "system")
    private set
  var language by mutableStateOf(options.locale ?: prefs.getString("locale", null) ?: "auto")
    private set
  var motion by mutableStateOf(options.motion && prefs.getBoolean("motion", true))
    private set

  fun pickTheme(id: String) {
    theme = id
    prefs.edit().putString("theme", id).apply()
  }

  /** Switches the table first, so the screens rebuilt under the new [language] read it */
  fun pickLanguage(id: String) {
    L10n.setup(context, id.takeUnless { it == "auto" })
    language = id
    prefs.edit().putString("locale", id).apply()
  }

  fun pickMotion(on: Boolean) {
    motion = on
    prefs.edit().putBoolean("motion", on).apply()
  }

  /** An engine setting shared with the computer: applied here at once, the engine's `settings` push confirms it */
  fun setDefaultAgent(id: String) {
    settings = settings.copy(defaultAgent = id)
    post(WebviewMsg.SetSetting("defaultAgent", JsonPrimitive(id)))
  }

  init {
    L10n.setup(context, language.takeUnless { it == "auto" })
    if (wire is Wire.Demo) {
      val fixturesNow = Fixtures.now(context)
      val launched = Instant.now()
      now = { fixturesNow.plusMillis(Instant.now().toEpochMilli() - launched.toEpochMilli()) }
      loadDemo()
    } else {
      now = { Instant.now() }
      when (wire) {
        is Wire.Engine -> connect(WebSocketTransport(wire.address.port, wire.address.token), "http://127.0.0.1:${wire.address.port}/blobs")
        // A paired wire's blobs come over its link; the computer's HTTP route is loopback only
        is Wire.Paired -> connect(wire.transport, null)
        else -> connection = Connection.Connecting
      }
    }
  }

  fun agent(id: String): AgentInfo = agents.firstOrNull { it.id == id } ?: AgentInfo(id, id)

  /** Agents a new session can start on, in the user's order */
  val startable: List<AgentInfo>
    get() {
      val order = settings.agentOrder
      return agents.filter { it.disabled != true && it.id !in settings.disabledAgents && it.available != false }
        .sortedBy { a -> order.indexOf(a.id).let { if (it < 0) Int.MAX_VALUE else it } }
    }

  fun isChat(cwd: String): Boolean {
    val root = chatRoot ?: return false
    return cwd.startsWith("$root/") && !cwd.removePrefix("$root/").contains('/')
  }

  fun placeName(cwd: String): String = if (isChat(cwd)) t("session.group.chats") else cwd.split('/', '\\').lastOrNull { it.isNotEmpty() } ?: cwd

  // Navigation

  fun push(screen: Screen) {
    path.add(screen)
    if (screen is Screen.Chat) select(screen.id)
  }

  /**
   * The session list stands in a sidebar beside the detail column (a wide window, ui/Root.kt): a pick there replaces
   * the page beside it instead of pushing over the list (apps/ios RootView `listInSidebar`)
   */
  var listInSidebar by mutableStateOf(false)

  /** A pick from the list: pushed over it on a phone, the detail column's new root beside the sidebar */
  fun open(screen: Screen) {
    if (!listInSidebar) return push(screen)
    val left = path.lastOrNull { it is Screen.Chat } as Screen.Chat?
    path.clear()
    if (left != null && left != screen) post(WebviewMsg.DeselectSession(left.id))
    push(screen)
  }

  /** The top screen goes; a chat left for the list stops counting as watched */
  fun pop() {
    val top = path.removeLastOrNull() ?: return
    if (top is Screen.Chat && path.none { it is Screen.Chat }) post(WebviewMsg.DeselectSession(top.id))
    path.lastOrNull()?.let { if (it is Screen.Chat) select(it.id) }
  }

  /** The clocks' time: the held moment while what the screens show may be stale, else now */
  fun clockNow(): Instant = clockHeld ?: now()

  /**
   * The app went to the background (true) or came back. Back on a wire that stayed up, the chat on screen is selected
   * again so the engine answers with its view as it is now; a new wire reopens it after its init (`reopen`)
   */
  fun setAway(background: Boolean) {
    if (background == away) return
    away = background
    // In the background the chat on screen is no longer watched, so a turn ending there gets its unread mark (and
    // its notice); back in front it is selected again (apps/ios AppModel `setAway` / `syncWatching`)
    if (background) {
      holdClock()
      if (connection == Connection.Live) onScreenChat()?.let { post(WebviewMsg.DeselectSession(it)) }
    } else if (connection == Connection.Live) onScreenChat()?.let { select(it) }
  }

  private fun holdClock() {
    if (live && clockHeld == null) clockHeld = now()
  }

  private fun onScreenChat(): String? = (path.lastOrNull { it is Screen.Chat } as Screen.Chat?)?.id

  private fun select(id: String) {
    if (!live) return
    feed.select(id)?.let { views[id] = it }
    post(WebviewMsg.SelectSession(id))
  }

  // Actions

  fun post(message: WebviewMsg): Boolean {
    val c = client ?: return false
    c.post(message)
    return true
  }

  fun send(sessionId: String, text: String, attachments: List<Draft> = emptyList()) {
    val body = text.trim()
    if (body.isEmpty() && attachments.isEmpty()) return
    val view = views[sessionId]
    val users = view?.turns?.count { it is UserTurn } ?: 0
    outgoing[sessionId] = outgoing[sessionId].orEmpty() + Outgoing(body, users)
    if (post(WebviewMsg.Send(sessionId, body, attachments.takeIf { it.isNotEmpty() }))) return
    demoSend(sessionId, body)
  }

  /** The composer's mode pick (Goo.swift onSetMode); the demo changes its own view */
  fun setMode(sessionId: String, id: String) {
    if (post(WebviewMsg.SetMode(sessionId, id))) return
    update(sessionId) { v -> v.copy(controls = v.controls.copy(modeId = id)) }
  }

  /** The composer's pick of a config option (model, effort, Fast, working mode, approval) */
  fun setConfig(sessionId: String, configId: String, value: String) {
    if (post(WebviewMsg.SetConfig(sessionId, configId, value))) return
    update(sessionId) { v ->
      v.copy(controls = v.controls.copy(options = v.controls.options.map { if (it.id == configId) it.copy(value = value) else it }))
    }
  }

  fun stop(sessionId: String) {
    if (!post(WebviewMsg.Stop(sessionId))) views[sessionId]?.let { views[sessionId] = it.copy(running = false) }
  }

  fun permission(sessionId: String, blockId: String, optionId: String) {
    if (post(WebviewMsg.Permission(sessionId, blockId, optionId))) return
    // The demo answers by dropping the card and finishing the turn
    val view = views[sessionId] ?: return
    views[sessionId] = view.copy(running = false, turns = view.turns.map { turn ->
      if (turn is AgentTurn) turn.copy(blocks = turn.blocks.filterNot { it is cc.hotic.acpira.model.PermissionBlock && it.id == blockId }) else turn
    })
  }

  /** Retry: the failed turn goes, its prompt runs again (`retryTurn`; `retry` restarts the session) */
  fun retryTurn(sessionId: String) {
    if (post(WebviewMsg.RetryTurn(sessionId))) return
    // The demo drops the failed turn and its prompt, then sends the prompt again
    val view = views[sessionId] ?: return
    if (view.turns.lastOrNull() !is AgentTurn) return
    val prompt = view.turns.getOrNull(view.turns.size - 2) as? UserTurn ?: return
    views[sessionId] = view.copy(turns = view.turns.dropLast(2))
    send(sessionId, prompt.text)
  }

  /**
   * Fork from a turn: a new session holding the transcript through that turn, opened over this one (apps/ios AppModel
   * `fork` / `engineFork`). The engine makes it and shows it on this view; that view opens its chat
   */
  fun fork(sessionId: String, turnIndex: Int) {
    val source = views[sessionId] ?: return
    if (turnIndex !in source.turns.indices) return
    val opened: (String) -> Unit = { id ->
      path.add(Screen.Chat(id))
      showToast(t("mobile.forked"))
    }
    if (live) {
      awaiting = sessionId to opened
      post(WebviewMsg.ForkSession(sessionId, turnIndex))
      return
    }
    val id = "demo-${++demoSeq}"
    val stamp = now().toString()
    val title = t("session.forkTitle", "title" to source.title)
    views[id] = source.copy(
      id = id, title = title, turns = source.turns.take(turnIndex + 1), running = false, queued = null, createdAt = stamp, updatedAt = stamp,
    )
    summaries = listOf(SessionSummary(id = id, title = title, agent = source.agent, cwd = source.cwd, updatedAt = stamp, accountId = source.accountId)) + summaries
    opened(id)
  }

  /** Starts a session of [agent] in [cwd] (a chat when null) and sends [text] once the engine has opened it */
  /**
   * A new session from the new-session page: [picked] are the page's model / effort / mode picks, applied where they
   * differ from what the engine started it with (apps/ios AppModel+Engine.swift apply), before the first prompt goes
   */
  fun start(agent: String?, cwd: String?, text: String, attachments: List<Draft> = emptyList(), picked: List<ConfigControl> = emptyList()) {
    val body = text.trim()
    val replace: (String) -> Unit = { id ->
      val at = path.indexOfLast { it is Screen.New }
      if (at >= 0) path[at] = Screen.Chat(id) else path.add(Screen.Chat(id))
      val made = views[id]?.controls?.options.orEmpty()
      for (option in picked) {
        val value = option.value ?: continue
        val now = made.firstOrNull { it.id == option.id } ?: continue
        if (now.value != value) setConfig(id, option.id, value)
      }
      if (body.isNotEmpty() || attachments.isNotEmpty()) send(id, body, attachments)
    }
    if (live) {
      awaiting = feed.views.current?.id to replace
      post(WebviewMsg.NewSession(agent = agent, cwd = cwd, chat = if (cwd == null) true else null))
      return
    }
    val id = "demo-${++demoSeq}"
    val stamp = now().toString()
    val chosen = agent ?: settings.defaultAgent
    val folder = cwd ?: "${chatRoot ?: "/"}/chat-$demoSeq"
    views[id] = SessionView(id = id, agent = chosen, title = body.take(40).ifEmpty { t("session.new") }, cwd = folder, createdAt = stamp, updatedAt = stamp)
    summaries = listOf(SessionSummary(id = id, title = views[id]!!.title, agent = chosen, cwd = folder, updatedAt = stamp)) + summaries
    replace(id)
  }

  /** New-session controls by agent (HostMsg.Controls): the new session page's model chip before a session exists */
  val agentControls = mutableStateMapOf<String, List<ConfigControl>>()

  /**
   * The engine's new-session controls for a folder on an SSH host, by [hostControlsKey]: there the agent may start in the
   * host's engine, with the host CLI's own config and models, so the agent-wide lists above do not fit
   */
  val hostControls = mutableStateMapOf<String, List<ConfigControl>>()

  /** What a new session of [agent] in [cwd] offers before it runs: a host folder's own answer, else the agent's */
  fun newSessionControls(agent: String, cwd: String?): List<ConfigControl> =
    hostControlsKey(agent, cwd)?.let { hostControls[it].orEmpty() } ?: agentControls[agent].orEmpty()

  /**
   * Asks the engine for an agent's new-session controls, for [cwd] when it is on an SSH host; the demo reads them off one
   * of the agent's views
   */
  fun loadControls(agent: String, cwd: String? = null) {
    if (post(WebviewMsg.Controls(agent, cwd = cwd?.takeIf { hostControlsKey(agent, it) != null }))) return
    if (agentControls[agent] == null) {
      agentControls[agent] = views.values.firstOrNull { it.agent == agent && it.controls.options.isNotEmpty() }?.controls?.options.orEmpty()
    }
  }

  /**
   * A folder picked by hand (apps/ios AppModel `addProject`): first in the folder menus, and registered with the engine,
   * which puts it at the end of the user's order (back on the list if it was taken off)
   */
  fun addProject(path: String) {
    val folder = path.trim()
    if (folder.isEmpty()) return
    addedProjects = listOf(folder) + addedProjects.filter { it != folder }
    if (!post(WebviewMsg.AddProject(folder)) && folder !in projectOrder) projectOrder = projectOrder + folder
  }

  fun togglePin(id: String) {
    val s = summaries.firstOrNull { it.id == id } ?: return
    val pinned = s.pinned != true
    if (!post(WebviewMsg.PinSession(id, pinned))) summaries = summaries.map { if (it.id == id) it.copy(pinned = pinned) else it }
  }

  fun delete(id: String) {
    if (!post(WebviewMsg.DeleteSession(id))) summaries = summaries.filter { it.id != id }
  }

  fun showToast(text: String) {
    toast = text
    scope.launch {
      delay(TOAST_MS)
      if (toast == text) toast = null
    }
  }

  // The demo

  private fun loadDemo() {
    val fixtures = Fixtures.sessionNames.mapNotNull { name -> Fixtures.session(context, name)?.let { name to it } }
    val showcase = fixtures.firstOrNull { it.first == "showcase" }?.second
    val list = Fixtures.sessions(context).toMutableList()
    // A listed session opens the fixture with its title, or the showcase dressed as that session
    for (s in list) {
      val base = fixtures.firstOrNull { it.second.title == s.title }?.second ?: showcase ?: continue
      views[s.id] = base.copy(id = s.id, title = s.title, agent = s.agent, cwd = s.cwd)
    }
    // Fixtures no listed session points at stay reachable by name
    for ((name, view) in fixtures) if (list.none { it.title == view.title }) views["fx-$name"] = view.copy(id = "fx-$name")
    // One session with background work, so the list shows every state mark
    list.indexOfFirst { it.title == "上下文压缩" }.takeIf { it >= 0 }?.let { list[it] = list[it].copy(state = SessionState.background) }
    summaries = list
    agents = Fixtures.agents(context)
    chatRoot = "/Users/dev/.acpira/chats"
    when (options.screen) {
      "chat" -> path.add(Screen.Chat(fixtureId(options.fixture)))
      "new" -> path.add(Screen.New())
      "settings" -> path.add(Screen.Settings)
    }
  }

  /** The session a fixture name opens */
  private fun fixtureId(name: String): String {
    val title = Fixtures.session(context, name)?.title ?: return summaries.firstOrNull()?.id.orEmpty()
    return summaries.firstOrNull { it.title == title }?.id ?: "fx-$name"
  }

  private fun demoSend(sessionId: String, text: String) {
    val view = views[sessionId] ?: return
    val started = now().toEpochMilli().toDouble()
    val reply = DemoReply.turn(text, view.cwd, started)
    views[sessionId] = view.copy(turns = view.turns + UserTurn(text = text), running = true)
    outgoing.remove(sessionId)
    // The reply streams in: the thought, the read, then the paragraph a few words at a time
    scope.launch {
      val blocks = reply.blocks
      for (n in 1 until blocks.size) {
        delay(DEMO_STEP_MS)
        update(sessionId) { v -> v.copy(turns = v.turns.dropLastWhile { it is AgentTurn && it.endedAt == null } + reply.copy(blocks = blocks.take(n), endedAt = null, stop = null)) }
      }
      val text = (blocks.last() as cc.hotic.acpira.model.TextBlock).markdown
      val words = text.split(" ")
      for (n in 1..words.size step 3) {
        delay(DEMO_WORD_MS)
        val partial = cc.hotic.acpira.model.TextBlock(markdown = words.take(n).joinToString(" "), streaming = true)
        update(sessionId) { v -> v.copy(turns = v.turns.dropLastWhile { it is AgentTurn && it.endedAt == null } + reply.copy(blocks = blocks.dropLast(1) + partial, endedAt = null, stop = null)) }
      }
      update(sessionId) { v -> v.copy(running = false, turns = v.turns.dropLastWhile { it is AgentTurn && it.endedAt == null } + reply.copy(endedAt = now().toEpochMilli().toDouble())) }
    }
  }

  private fun update(id: String, change: (SessionView) -> SessionView) {
    views[id]?.let { views[id] = change(it) }
  }

  // The engine

  private fun connect(transport: EngineTransport, blobBase: String?) {
    connection = Connection.Connecting
    val language = cc.hotic.acpira.i18n.L10n.locale
    val hello = Hello(
      requestId = "hello-1",
      client = ClientInfo("acpira-android", appVersion(), EngineClient.capabilities),
      // projects: the app lists sessions by folder, so the engine never starts a session without one
      env = ShellEnv(hostLanguage = language, blobBase = blobBase, projects = true),
    )
    val c = EngineClient(transport, hello)
    client = c
    c.post(WebviewMsg.Ready(patches = true))
    scope.launch(Dispatchers.Main) { c.events.collect { receive(it) } }
    // The wire is read and decoded off the main thread (a long session's init is megabytes); pushes land on it
    c.start(CoroutineScope(scope.coroutineContext + Dispatchers.Default))
  }

  private fun appVersion(): String = runCatching {
    context.packageManager.getPackageInfo(context.packageName, 0).versionName
  }.getOrNull() ?: "0"

  private fun receive(event: EngineEvent) {
    when (event) {
      is EngineEvent.Connected -> {
        engineFeatures = event.ok.features.orEmpty()
        connection = Connection.Live
      }
      is EngineEvent.Host -> receive(event.message)
      is EngineEvent.Platform -> when (val r = event.request) {
        is PlatformRequest.Toast -> showToast(r.text)
        else -> {}
      }
      is EngineEvent.Dropped -> {
        Log.w(TAG, "dropped ${event.reason}")
        showToast(t("mobile.engine.dropped"))
      }
      is EngineEvent.Incompatible -> showToast(t(if (event.engineVersion < SidecarProtocol.VERSION) "mobile.engine.updateComputer" else "mobile.engine.updateApp"))
      is EngineEvent.Closed -> {
        Log.i(TAG, "closed ${event.reason}")
        connection = Connection.Closed(event.reason)
        holdClock()
        // A paired computer's wire has its own screens for an ended connection; the development engine gets a toast
        val ended = onClosed
        if (ended != null) ended(event.reason) else showToast(t("mobile.engine.disconnected"))
      }
    }
  }

  private fun receive(message: HostMsg) {
    when (message) {
      is HostMsg.Init -> {
        val s = message.state
        agents = s.agents
        settings = s.settings
        summaries = s.sessions
        chatRoot = s.chats
        home = s.home
        s.categories?.projects?.let { projectOrder = it }
        present(message)
        synced = true
        if (resumed) reopen()
        // The launch asks: a session to open, or one to start
        options.session?.let { if (path.none { p -> p is Screen.Chat }) push(Screen.Chat(it)) }
        if (options.start && path.none { it is Screen.Chat }) start(options.agent, null, options.prompt.orEmpty())
      }
      is HostMsg.Agents -> agents = message.agents
      is HostMsg.Sessions -> summaries = message.sessions
      is HostMsg.Categories -> message.categories.projects?.let { projectOrder = it }
      is HostMsg.Settings -> settings = message.settings
      is HostMsg.Controls -> {
        val key = hostControlsKey(message.agent, message.cwd)
        if (key != null) hostControls[key] = message.controls else agentControls[message.agent] = message.controls
      }
      is HostMsg.Session, is HostMsg.Patch, HostMsg.NoSession -> present(message)
      is HostMsg.LocalDir -> folders.receive(message)
      is HostMsg.Providers -> sources.receive(message)
      is HostMsg.ProviderProbed -> sources.receive(message)
      else -> {}
    }
  }

  private fun present(message: HostMsg) {
    when (val outcome = feed.receive(message)) {
      is SessionFeed.Outcome.Show -> {
        val view = outcome.view ?: return
        // Only the views the feed keeps stay, so a long transcript is not held past its cache bound
        val kept = feed.views.kept.toSet()
        // Views carried over from the wire before stay too, while the bound leaves them room; one this wire shows
        // becomes the feed's own
        if (carried.isNotEmpty()) {
          val left = carried.filter { it !in kept }
          carried = left.takeLast(maxOf(0, VIEW_CACHE_ENTRIES - kept.size))
        }
        val keep = if (carried.isEmpty()) kept else kept + carried
        views.keys.filter { it !in keep }.forEach { views.remove(it) }
        views[view.id] = view
        // The engine's view of the chat on screen, back in the foreground: the clock runs on the engine's times again
        if (clockHeld != null && !away && view.id == onScreenChat()) clockHeld = null
        // A prompt still on its way gives up its place once the session shows the engine's copy
        outgoing[view.id]?.let { sent ->
          val users = view.turns.count { it is UserTurn }
          val left = sent.filter { it.usersBefore >= users }
          if (left.isEmpty()) outgoing.remove(view.id) else outgoing[view.id] = left
        }
        awaiting?.let { (from, then) ->
          if (view.id != from) {
            awaiting = null
            then(view.id)
          }
        }
      }
      SessionFeed.Outcome.Unchanged -> {}
      SessionFeed.Outcome.Resync -> client?.post(WebviewMsg.Resync)
    }
  }

  fun close() {
    client?.stop()
  }

  /**
   * Takes over the screens of the model before this one, a lost wire to the same computer (apps/ios AppModel+Engine
   * `resume`): the lists, the views and the stack stay as they were while this wire syncs, and what the old wire may
   * not have delivered goes out again once this one's init is in
   */
  fun resume(from: AppModel) {
    agents = from.agents
    settings = from.settings
    summaries = from.summaries
    from.views.forEach { (id, v) -> views[id] = v }
    // Least recently shown first: what the wire before still carried itself, then its own feed's order
    carried = (from.carried + from.feed.views.kept).distinct().filter { it in views }
    from.outgoing.forEach { (id, o) -> outgoing[id] = o }
    chatRoot = from.chatRoot
    home = from.home
    projectOrder = from.projectOrder
    addedProjects = from.addedProjects
    path.addAll(from.path)
    replay = from.replay + (from.client?.unsent() ?: emptyList())
    clockHeld = from.clockHeld
    away = from.away
    resumed = true
  }

  /**
   * After a resumed wire's init: its view starts on no session, so the chat on screen is opened again, then the held
   * actions go out in order. The engine refuses an action for a session its view does not show, so one for another
   * session switches the view to it first and back after
   */
  private fun reopen() {
    val onScreen = path.lastOrNull { it is Screen.Chat }?.let { (it as Screen.Chat).id }
    onScreen?.let { select(it) }
    val held = replay
    replay = emptyList()
    var shown = onScreen
    for (action in held) {
      val target = sessionOf(action)
      if (target != null && target != shown) {
        post(WebviewMsg.SelectSession(target))
        shown = target
      }
      post(action)
    }
    if (shown != onScreen && onScreen != null) post(WebviewMsg.SelectSession(onScreen))
  }

  private fun sessionOf(action: WebviewMsg): String? = when (action) {
    is WebviewMsg.Send -> action.sessionId
    is WebviewMsg.Stop -> action.sessionId
    is WebviewMsg.Permission -> action.sessionId
    is WebviewMsg.Answer -> action.sessionId
    is WebviewMsg.SendQueued -> action.sessionId
    is WebviewMsg.SetMode -> action.sessionId
    is WebviewMsg.SetConfig -> action.sessionId
    else -> null
  }

  companion object {
    /** The built-in agent's id (providers.ts BUILTIN_AGENT_ID): the one with model sources */
    const val BUILTIN_AGENT = "acpira"
    private const val TAG = "acpira"
    private const val TOAST_MS = 3200L
    private const val DEMO_STEP_MS = 700L
    private const val DEMO_WORD_MS = 60L
  }
}

/** Where [AppModel.hostControls] keeps an agent's answer for a folder on an SSH host (`ssh://<alias>/<path>`); null otherwise */
fun hostControlsKey(agent: String, cwd: String?): String? = cwd?.takeIf { it.startsWith("ssh://") }?.let { "$it\n$agent" }
