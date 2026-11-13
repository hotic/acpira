package cc.hotic.acpira.ui

import kotlinx.coroutines.delay
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.animation.core.tween
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.snap
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.background
import cc.hotic.acpira.design.readableWidth
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.IconButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import cc.hotic.acpira.model.SessionControls
import androidx.compose.runtime.mutableStateMapOf
import cc.hotic.acpira.logic.editTurnConfig
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.TransformOrigin
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Constraints
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.design.VendorMark
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.store.AppModel
import cc.hotic.acpira.store.Screen
import cc.hotic.acpira.store.commands

/**
 * A new session (apps/ios Sessions/NewSessionScreen.swift): the agent's mark in the middle with its name and the folder
 * it starts in, both pickable; the first prompt from the composer starts it. No folder: a chat
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun NewSessionScreen(model: AppModel, screen: Screen.New) {
  // The ring offers every launchable agent, the ones not installed dimmed; only an installed one starts by default
  val agents = model.launchable
  val startable = model.startable
  // As the iOS app picks it (RootView destination): the asked agent, else the newest session's, else the first that can start
  // Until the user picks one the default follows the engine: the split's detail column opens this page before init
  // brings the agents and sessions, and the built-in fallback it had then cannot start anything
  var pickedAgent by rememberSaveable { mutableStateOf<String?>(null) }
  // The newest session's agent only while it can still start (it may have been switched off since)
  val agent = pickedAgent
    ?: screen.agent ?: model.summaries.filter { s -> startable.any { it.id == s.agent } }.maxByOrNull { it.updatedAt }?.agent
    ?: startable.firstOrNull()?.id ?: model.settings.defaultAgent
  // Every known project, the latest first, those taken off the list left out (apps/ios AppModel `projects`)
  val projects = model.lists.projects
  // Until a pick the folder follows the list; a blank folder asks for a chat (the list's "new chat", as apps/ios
  // `.new(cwd: "")`), and a picked chat is null too, hence the separate flag
  var picked by rememberSaveable { mutableStateOf(false) }
  var pickedCwd by rememberSaveable { mutableStateOf<String?>(null) }
  val cwd = if (picked) pickedCwd else if (screen.cwd == "") null else screen.cwd ?: projects.firstOrNull()
  // The capsule and the menu's tick read a chat where the engine keeps chats (apps/ios AppModel `startsChat`)
  val chat = model.startsChat(cwd)
  var draft by rememberSaveable { mutableStateOf("") }
  var staged by remember { mutableStateOf<List<Staged>>(emptyList()) }
  val goo = remember { Goo() }
  val info = model.agent(agent)
  // The agent ring (AgentRing): out or not, the picked agent while its mark flies in, and the hero's name and folder
  // held back until that mark lands or the bubbles are home
  var ring by remember { mutableStateOf(false) }
  var landing by remember { mutableStateOf<String?>(null) }
  var veil by remember { mutableStateOf(false) }
  LaunchedEffect(ring) {
    if (ring) veil = true
    else if (veil) {
      if (model.motion) delay(if (landing != null) HERO_LEAD_MS else HERO_HOLD_MS)
      veil = false
    }
  }
  val dim by animateFloatAsState(if (ring) 1f else 0f, if (model.motion) tween(HERO_AWAY_MS) else snap(), label = "ringDim")
  // The agent's new-session controls (asked again on every wire, whose answer a lost one never brings), and the ring's
  // other agents' too, so a switch finds their models in hand rather than dropping the model chip meanwhile
  LaunchedEffect(model, model.connection, agent, cwd) { model.loadControls(agent, cwd) }
  val ringIds = agents.map { it.id }
  LaunchedEffect(model, model.connection, ringIds, cwd) { model.prefetchControls(ringIds, cwd) }
  // An agent the engine knows no models for gets one fresh probe on this wire
  val known = model.knownControls(agent, cwd)
  LaunchedEffect(model, known, agent, cwd) { model.probeUnknownControls(agent, cwd) }
  // This page's picks, each agent its own (models and modes differ per CLI); a pick is never replaced by a later answer,
  // also after switching away and back (apps/ios NewSessionScreen `picks`)
  val picks = remember { mutableStateMapOf<String, SessionControls>() }
  // Another folder may mean another machine (an SSH host's engine runs the agent with its own models): picks made from
  // one machine's list do not carry over to the other's
  var pickedOn by remember { mutableStateOf(cwd) }
  if (pickedOn != cwd) {
    if (remoteHost(pickedOn.orEmpty()) != null || remoteHost(cwd.orEmpty()) != null) picks.clear()
    pickedOn = cwd
  }
  // The folder menu's "Add folder" is open
  var adding by remember { mutableStateOf(false) }
  val controls = picks[agent] ?: model.controlsFor(agent, cwd)

  Box(Modifier.fillMaxSize()) {
  Scaffold(
    containerColor = Palette.bg0,
    topBar = {
      TopAppBar(
        title = { Text(t("session.new"), fontSize = Type.text1, fontWeight = FontWeight.SemiBold) },
        navigationIcon = {
          LeadButton("new.back") { model.pop() }
        },
        colors = TopAppBarDefaults.topAppBarColors(containerColor = Palette.bg0),
      )
    },
    bottomBar = {
      // The composer holds to the reading column in a wide window
      Box(Modifier.readableWidth()) {
      Composer(
        text = draft,
        onText = { draft = it },
        staged = staged,
        onStaged = { staged = it },
        controls = controls,
        agentMark = agent,
        agentName = info.name,
        running = false,
        usage = null,
        placeholder = t("mobile.new.placeholder"),
        goo = goo,
        motion = model.motion,
        onSend = {
          val text = draft
          val attachments = staged.map { it.draft }
          draft = ""
          staged = emptyList()
          model.start(agent, cwd, text, attachments, controls)
        },
        onStop = {},
        onSetMode = { picks[agent] = controls.copy(modeId = it) },
        onSetConfig = { id, value ->
          // No session answers set_config_option here, so a model pick brings that model's own parameters and a
          // sidekick pick its level select from the remembered shapes (logic/TurnEdit.kt): a model without Fast loses
          // the previous model's Fast
          if (controls.options.any { it.id == id }) {
            picks[agent] = editTurnConfig(controls, controls, id, value, model.newSessionShapes(agent, cwd))
          }
        },
        onToast = model::showToast,
        // The agent's newest session's commands give the draft its `/` list, since the CLI only starts with the first
        // prompt; no session yet to search files in
        assist = ComposerAssist(commands = model.commands(agent, cwd)),
      )
      }
    },
  ) { padding ->
    Box(Modifier.fillMaxSize()) {
    // The page dims under an open ring; a tap on it folds the ring
    if (dim > 0f) {
      Box(
        Modifier.fillMaxSize().background(Palette.gooDim.copy(alpha = Palette.gooDim.alpha * dim))
          .clickable(interactionSource = remember { MutableInteractionSource() }, indication = null, enabled = ring) { ring = false },
      )
    }
    BoxWithConstraints(Modifier.fillMaxSize().padding(padding).gooRecede(goo, model.motion)) {
    val ringRoom = remember { RingRoom() }
    HeroColumn(
      constraints.maxHeight, ringRoom,
      // The column takes the page's taps as a scroller, so it folds an open ring itself, as the dim under it does
      Modifier.fillMaxSize().clickable(interactionSource = remember { MutableInteractionSource() }, indication = null, enabled = ring) { ring = false },
    ) {
      AgentRing(
        agent = info,
        others = agents.filter { it.id != agent },
        open = ring,
        motion = model.motion,
        onOpen = { ring = it },
        onSelect = { pickedAgent = it },
        landing = landing,
        onLanding = { landing = it },
        room = ringRoom,
      )
      // The name and the folder step back while the ring is out, and rise back as a picked mark lands or once the
      // bubbles are home after a plain close
      val away by animateFloatAsState(
        if (veil || ring) 1f else 0f,
        if (!model.motion) snap() else if (veil || ring) tween(HERO_AWAY_MS) else spring(dampingRatio = 0.85f, stiffness = 195f),
        label = "heroAway",
      )
      val hero = Modifier.graphicsLayer {
        alpha = 1 - away
        translationY = -Metrics.s3.toPx() * away
      }
      Text(
        (landing?.let { model.agent(it) } ?: info).name, color = Palette.fgStrong, fontSize = Type.field, lineHeight = Type.fieldLine,
        fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis,
        modifier = hero.padding(horizontal = Metrics.pad),
      )
      var pickFolder by remember { mutableStateOf(false) }
      Box(hero.padding(horizontal = Metrics.pad)) {
        Row(
          verticalAlignment = Alignment.CenterVertically,
          horizontalArrangement = Arrangement.spacedBy(Metrics.s2),
          modifier = Modifier
            .background(Palette.bg1, RoundedCornerShape(percent = 50))
            .border(Metrics.hairline, Palette.conversationLine, RoundedCornerShape(percent = 50))
            .clip(RoundedCornerShape(percent = 50))
            .clickable(enabled = !ring && !veil) { pickFolder = true }
            .padding(horizontal = Metrics.pad, vertical = Metrics.s2)
            .testTag("new.folder"),
        ) {
          val asChat = chat || cwd == null
          Lucide(if (asChat) "messagesSquare" else "folder", Palette.fg2, size = Metrics.icon)
          // A long folder name keeps to one line inside the window
          Text(
            if (asChat) t("session.group.chats") else model.placeName(cwd!!), color = Palette.fg2, fontSize = Type.text1,
            maxLines = 1, overflow = TextOverflow.Ellipsis,
          )
        }
        FolderMenu(
          open = pickFolder,
          cwd = cwd,
          chat = chat,
          // An empty folder is a chat; offered where the engine keeps chats
          chats = model.chatRoot != null,
          projects = projects,
          name = model::placeName,
          onDismiss = { pickFolder = false },
          onPick = { pickedCwd = it; picked = true; pickFolder = false },
          onAdd = { pickFolder = false; adding = true },
        )
      }
    }
    }
    }
  }
  // The composer's open picker, over everything (the composer included), so no piece hides under the card
  GooLayer(goo, model.motion)
  // Add folder: the browser over the whole page; the added folder becomes this draft's
  AddProject(model, adding, onClose = { adding = false }, onAdded = { pickedCwd = it; picked = true })
  }
}

/**
 * The hero column: the ring's drop, the agent's name and the folder capsule, centred in [room] (px). A window too short
 * for them takes the height from the drop (the gap under it, then its size down to [Metrics.heroTileMin]), then from
 * the gaps, and scrolls past that, so the capsule stays in reach. It also tells the ring how far it may reach above and below the drop
 */
@Composable
private fun HeroColumn(room: Int, ringRoom: RingRoom, modifier: Modifier = Modifier, content: @Composable () -> Unit) {
  val scroll = rememberScrollState()
  Layout(content, Modifier.verticalScroll(scroll).then(modifier)) { measurables, constraints ->
    val loose = constraints.copy(minWidth = 0, minHeight = 0, maxHeight = Constraints.Infinity)
    val (ring, name, folder) = measurables.map { it.measure(loose) }
    val words = name.height + folder.height
    val loosest = Metrics.gap.roundToPx()
    val pad = (room - words - loosest * 2 - ring.height).coerceIn(0, Metrics.s3.roundToPx())
    val tile = (room - words - loosest * 2 - pad).coerceIn(Metrics.heroTileMin.roundToPx(), ring.height)
    val gap = ((room - words - tile - pad) / 2).coerceIn(Metrics.s2.roundToPx(), loosest)
    val scale = tile.toFloat() / ring.height
    val total = tile + pad + words + gap * 2
    val height = maxOf(room, total)
    val top = (height - total) / 2
    // From the drop's centre to the page's edges less a margin, in the ring's own (scaled) px
    val centre = top + tile / 2f - scroll.value
    val margin = Metrics.s2.toPx()
    ringRoom.up = (centre - margin) / scale
    ringRoom.down = (room - centre - margin) / scale
    layout(constraints.maxWidth, height) {
      val mid = constraints.maxWidth / 2
      ring.placeWithLayer(mid - ring.width / 2, top) {
        scaleX = scale; scaleY = scale
        transformOrigin = TransformOrigin(0.5f, 0f)
      }
      val nameTop = top + tile + pad + gap
      name.place(mid - name.width / 2, nameTop)
      folder.place(mid - folder.width / 2, nameTop + name.height + gap)
    }
  }
}

/**
 * The folder capsule's menu (apps/ios NativeProjectMenu): the chat on top, then this computer's projects and the SSH
 * hosts' as submenus, so a long project list never opens as one tall menu. A submenu replaces the level it came from,
 * with its head as the way back
 */
@Composable
private fun FolderMenu(
  open: Boolean,
  cwd: String?,
  chat: Boolean,
  chats: Boolean,
  projects: List<String>,
  name: (String) -> String,
  onDismiss: () -> Unit,
  onPick: (String?) -> Unit,
  onAdd: () -> Unit,
) {
  // null: the top level; "" this computer's projects; a host alias: that host's
  var level by remember { mutableStateOf<String?>(null) }
  // The draft's own folder first, then the known ones, each once (NativeProjectMenu.places); a chat's folder is no project
  val folders = ((if (chat || cwd == null) emptyList() else listOf(cwd)) + projects).filter { it.isNotEmpty() }.distinct()
  val local = folders.filter { remoteHost(it) == null }
  val remote = folders.filter { remoteHost(it) != null }
  val hosts = remote.mapNotNull(::remoteHost).distinct()
  // A folder ticks only while the draft is not a chat
  val folder = cwd.takeUnless { chat }
  AppMenu(
    expanded = open,
    onDismiss = { onDismiss(); level = null },
    modifier = Modifier.heightIn(max = Metrics.menuRow * MENU_ROWS),
  ) {
    when (val at = level) {
      null -> {
        // Where the engine keeps no chats there is no chat to start
        if (chats) {
          AppMenuItem(
            t("session.group.chats"), onClick = { onPick(null); level = null }, checked = chat,
            icon = { MenuGlyph("messagesSquare") }, modifier = Modifier.testTag("folder.chats"),
          )
          AppMenuGap()
        }
        if (local.isNotEmpty()) {
          AppMenuItem(
            t("mobile.projects.local"), onClick = { level = "" }, checked = folder != null && remoteHost(folder) == null,
            icon = { MenuGlyph("chevronRight") }, modifier = Modifier.testTag("folder.local"),
          )
        }
        for (h in hosts) {
          AppMenuItem(
            h, subtitle = t("mobile.projects.remote"), onClick = { level = h }, checked = folder?.let(::remoteHost) == h,
            icon = { MenuGlyph("chevronRight") },
          )
        }
        // After the folders, as the iOS menu ends: a folder of the computer picked by hand
        AppMenuGap()
        AppMenuItem(
          t("mobile.projects.add"), onClick = { onAdd(); level = null }, checked = false,
          icon = { MenuGlyph("folderPlus") }, modifier = Modifier.testTag("folder.add"),
        )
      }
      else -> {
        AppMenuBack(if (at.isEmpty()) t("mobile.projects.local") else at, onClick = { level = null })
        AppMenuGap()
        for (p in if (at.isEmpty()) local else remote.filter { remoteHost(it) == at }) {
          AppMenuItem(
            name(p), subtitle = parentOf(p), onClick = { onPick(p); level = null }, checked = p == folder,
            icon = { MenuGlyph(if (at.isEmpty()) "folder" else "globe") },
          )
        }
      }
    }
  }
}

/** `ssh://<alias>/<path>`'s alias (apps/ios RemoteFolder.parse); null for a local folder */
private fun remoteHost(folder: String): String? {
  if (!folder.startsWith(SSH)) return null
  val rest = folder.removePrefix(SSH)
  val slash = rest.indexOf('/')
  return if (slash > 0) rest.substring(0, slash) else null
}

/** The folder's parent as a short subtitle (NativeProjectMenu.subtitle): its last two parts, `…/` before more */
private fun parentOf(folder: String): String {
  val path = remoteHost(folder)?.let { folder.removePrefix(SSH + it) } ?: folder
  val parts = path.trimEnd('/', '\\').split('/', '\\').filter { it.isNotEmpty() }.dropLast(1)
  if (parts.isEmpty()) return if (path.startsWith("/")) "/" else "."
  val tail = parts.takeLast(2).joinToString("/")
  return if (parts.size > 2) "…/$tail" else (if (path.startsWith("/")) "/" else "") + tail
}

private const val SSH = "ssh://"
/** The menu's height in rows before it scrolls */
private const val MENU_ROWS = 8.5f

/** The hero's name and folder: away as the ring opens (Motion.ringHeroAway), back after a plain close once the
 * bubbles are home (ringHeroHold), or this far into a pick's flight (ringHeroLead) */
private const val HERO_AWAY_MS = 200
private const val HERO_HOLD_MS = 450L
private const val HERO_LEAD_MS = 100L
