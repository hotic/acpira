package cc.hotic.acpira.ui

import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.expandHorizontally
import androidx.compose.animation.shrinkHorizontally
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.slideOutVertically
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.width
import androidx.compose.material3.IconButton
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.platform.testTag
import cc.hotic.acpira.design.LocalNavLead
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.NavLead
import cc.hotic.acpira.i18n.t
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTagsAsResourceId
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.store.AppModel
import cc.hotic.acpira.store.Screen
import androidx.compose.animation.EnterTransition
import androidx.compose.animation.ExitTransition

/** Motion durations (packages/ui motion tokens): a page push and the toast's entrance */
object Motion {
  const val push = 280
  const val fade = 180
}

/**
 * The app's stack: the session list at the root, a chat, a new session or settings over it. The system back gesture
 * pops one screen, as the iOS app's edge swipe does
 */
@Composable
fun Root(model: AppModel, pairing: cc.hotic.acpira.store.PairingModel? = null) {
  // The settings' hidden models reach every composer's model menu (ChatScreen.swift `.environment(\.hiddenModels)`)
  androidx.compose.runtime.CompositionLocalProvider(
    LocalPairing provides pairing,
    LocalHiddenModels provides model.settings.hiddenOptions,
    // The transcript's folds, rails and entrances read the motion switch (the webview's motion axis)
    cc.hotic.acpira.design.LocalMotion provides model.motion,
    // The list's live marks hold still while what they show may be stale (StateMark)
    LocalSyncing provides !model.current,
    // Session links in replies open their chat in the app (ui/Links.kt)
    androidx.compose.ui.platform.LocalUriHandler provides rememberLinkHandler(model),
  ) { Stack(model, pairing) }
}

@Composable
private fun Stack(model: AppModel, pairing: cc.hotic.acpira.store.PairingModel?) {
  // Test tags double as resource ids, so uiautomator and the touch-sim scripts find controls by name
  BoxWithConstraints(Modifier.fillMaxSize().background(Palette.bg0).semantics { testTagsAsResourceId = true }) {
    // A paired build's connect page takes the whole window until the computer's lists are in: a sidebar and a page
    // with nothing to show say nothing (apps/ios RootView `covered`)
    val covered = pairing != null && !model.options.demo && covers(pairing, model)
    val wide = maxWidth >= Metrics.splitMinW && maxHeight >= Metrics.splitMinH && !covered
    SideEffect { model.listInSidebar = wide }
    if (wide) Split(model, roomy = maxWidth - Metrics.sidebarW >= Metrics.contentW + Metrics.pad * 2)
    else {
      BackHandler(enabled = model.path.isNotEmpty()) { model.pop() }
      Pages(model, pairing, model.path.toList(), root = null)
    }
    // The add-computer sheet, also for an `acpira://pair/…` link opened from outside; the first-run flow pairs in place
    if (pairing != null && pairing.addOpen && !pairing.onboarding) PairSheet(pairing)
    // The engine's and the app's own notices, over every screen
    val toast = model.toast
    val fade = if (model.motion) Motion.fade else 0
    AnimatedVisibility(
      visible = toast != null,
      enter = fadeIn(tween(fade)) + slideInVertically(tween(fade)) { -it / 2 },
      exit = fadeOut(tween(fade)) + slideOutVertically(tween(fade)) { -it / 2 },
      modifier = Modifier.align(Alignment.TopCenter).safeDrawingPadding().padding(top = Metrics.s2),
    ) {
      Surface(
        color = Palette.btn1Bg,
        shape = androidx.compose.foundation.shape.RoundedCornerShape(Metrics.rCardRound),
        shadowElevation = Metrics.toastShadow,
      ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
          Text(
            toast.orEmpty(), color = Palette.btn1Fg, fontSize = Type.text3,
            modifier = Modifier.padding(horizontal = Metrics.pad, vertical = Metrics.s2 + Metrics.s1),
          )
          // Undo after a delete, an archive or a project taken off the list (store/SessionLists.kt)
          ToastActionButton(model, toast)
        }
      }
    }
    if (pairing != null) Welcome(model, pairing)
  }
}

/**
 * The pages over [root]: the top one shows, sliding in from the end on a push and back out on a pop. [root] null is the
 * session list (the phone's stack); in the split's detail column it is the column's first page
 */
@Composable
private fun Pages(model: AppModel, pairing: cc.hotic.acpira.store.PairingModel?, pages: List<Screen>, root: Screen?, rootLead: NavLead = NavLead.Back) {
  val top: Screen? = pages.lastOrNull() ?: root
  val depth = pages.size
  AnimatedContent(
    targetState = top to depth,
    transitionSpec = {
      val forward = targetState.second >= initialState.second
      val d = tween<androidx.compose.ui.unit.IntOffset>(Motion.push)
      // Motion off (the settings switch), or a new root picked in the sidebar: pages swap at once
      if (!model.motion || (targetState.second == 0 && initialState.second == 0)) EnterTransition.None togetherWith ExitTransition.None
      else if (forward) slideInHorizontally(d) { it } togetherWith slideOutHorizontally(d) { -it / 4 }
      else slideInHorizontally(d) { -it / 4 } togetherWith slideOutHorizontally(d) { it }
    },
    contentKey = { (screen, _) -> screen?.let { key(it) } ?: "list" },
    label = "stack",
  ) { (screen, d) ->
    Surface(color = Palette.bg0, modifier = Modifier.fillMaxSize()) {
      CompositionLocalProvider(LocalNavLead provides if (d == 0 && root != null) rootLead else NavLead.Back) {
        Page(model, pairing, screen)
      }
    }
  }
}

@Composable
private fun Page(model: AppModel, pairing: cc.hotic.acpira.store.PairingModel?, screen: Screen?) {
  when (screen) {
    null -> SessionListScreen(model)
    is Screen.Chat -> ChatScreen(model, screen.id)
    is Screen.New -> NewSessionScreen(model, screen)
    Screen.Settings -> SettingsScreen(model)
    is Screen.Computer -> if (pairing != null) ComputerScreen(model, pairing, screen.id)
    is Screen.AgentSettings -> AgentSettingsScreen(model, screen.id)
    is Screen.ModelSettings -> ModelsSettingsScreen(model, screen.id)
    Screen.Developer -> DeveloperSettingsScreen(model)
    Screen.Providers -> ProvidersScreen(model)
    is Screen.ProviderForm -> ProviderFormScreen(model, screen.id, screen.error)
    is Screen.AddModels -> AddModelsScreen(model, screen.id)
    is Screen.Download -> DownloadScreen(model, screen.sessionId, screen.path)
    is Screen.Changes -> cc.hotic.acpira.ui.files.ChangesScreen(model, screen.session)
    is Screen.File -> cc.hotic.acpira.ui.files.FileScreen(model, screen.session, screen.path, screen.line)
    is Screen.Terminal -> cc.hotic.acpira.ui.terminal.TerminalPage(model, screen.cwd)
    is Screen.Subagent -> cc.hotic.acpira.ui.subagents.SubagentInspector(model, screen.session, screen.id)
    is Screen.Subagents -> cc.hotic.acpira.ui.subagents.SubagentTreePage(model, screen.session)
  }
}

/**
 * A wide window (tablet, a foldable's inner screen): the session list in a sidebar and the detail column beside it,
 * the path's first page at the column's root and the rest pushed over it, so one path serves both shells and a resize
 * across the line keeps the reader where they were (apps/ios RootView `shell`). The sidebar stands open while the
 * detail keeps its reading column ([roomy]); narrower, the detail leads and the sidebar slides over it from its bar
 */
@Composable
private fun Split(model: AppModel, roomy: Boolean) {
  val pairing = LocalPairing.current
  var sliding by rememberSaveable { mutableStateOf(false) }
  // Roomy, the sidebar can still be put away for the detail alone and brought back from its bar (the split view's own
  // toggle on iPadOS, apps/ios RootView `columns`)
  var hidden by rememberSaveable { mutableStateOf(false) }
  val docked = roomy && !hidden
  // Nothing picked yet: a new session, as the desktop opens on its composer
  val root = model.path.firstOrNull() ?: Screen.New()
  val over = model.path.drop(1)
  BackHandler(enabled = sliding && !roomy) { sliding = false }
  BackHandler(enabled = over.isNotEmpty() && !(sliding && !roomy)) { model.pop() }
  // A pick in the sliding sidebar lands its page and folds the sidebar away
  LaunchedEffect(model.path.firstOrNull()) { sliding = false }
  val sidebar = @Composable { hide: () -> Unit ->
    Box(Modifier.width(Metrics.sidebarW).fillMaxHeight().testTag("split.sidebar")) {
      CompositionLocalProvider(LocalSidebarHide provides hide) { SessionListScreen(model) }
    }
  }
  Row(Modifier.fillMaxSize()) {
    val fade = if (model.motion) Motion.push else 0
    // The docked sidebar slides away to the start and back, the detail widening into its room
    AnimatedVisibility(
      visible = docked,
      enter = expandHorizontally(tween(fade), expandFrom = Alignment.Start) + fadeIn(tween(fade)),
      exit = shrinkHorizontally(tween(fade), shrinkTowards = Alignment.Start) + fadeOut(tween(fade)),
    ) {
      Row {
        sidebar { hidden = true }
        Box(Modifier.width(Metrics.hairline).fillMaxHeight().background(Palette.line))
      }
    }
    Box(Modifier.weight(1f).fillMaxHeight().testTag("split.detail")) {
      CompositionLocalProvider(LocalSidebar provides { if (roomy) hidden = false else sliding = true }) {
        Pages(model, pairing, over, root = root, rootLead = if (docked) NavLead.None else NavLead.Sidebar)
      }
    }
  }
  if (!roomy) {
    val fade = if (model.motion) Motion.push else 0
    AnimatedVisibility(visible = sliding, enter = fadeIn(tween(fade)), exit = fadeOut(tween(fade))) {
      Box(Modifier.fillMaxSize().background(Palette.scrim).clickable(interactionSource = null, indication = null) { sliding = false })
    }
    AnimatedVisibility(
      visible = sliding,
      enter = slideInHorizontally(tween(fade)) { -it },
      exit = slideOutHorizontally(tween(fade)) { -it },
    ) {
      Surface(color = Palette.bg0, shadowElevation = Metrics.toastShadow) { sidebar { sliding = false } }
    }
  }
}

/** Opens the sliding sidebar, for the detail column root's leading button; null outside the split */
val LocalSidebar = androidx.compose.runtime.compositionLocalOf<(() -> Unit)?> { null }

/** Puts the sidebar away, for the list's bar while it stands in the sidebar; null outside it */
val LocalSidebarHide = androidx.compose.runtime.compositionLocalOf<(() -> Unit)?> { null }

/**
 * A page bar's leading button: back on a stacked page, the sidebar at the detail column's root while it slides over,
 * nothing beside an open one ([LocalNavLead])
 */
@Composable
fun LeadButton(tag: String, onBack: () -> Unit) {
  when (LocalNavLead.current) {
    NavLead.Back -> IconButton(onClick = onBack, modifier = Modifier.testTag(tag)) {
      Lucide("arrowLeft", Palette.fg1, size = Metrics.iconCtl, contentDescription = t("common.back"))
    }
    NavLead.Sidebar -> {
      val open = LocalSidebar.current
      IconButton(onClick = { open?.invoke() }, modifier = Modifier.testTag("split.open")) {
        Lucide("panelLeft", Palette.fg1, size = Metrics.iconCtl, contentDescription = t("desktop.showSidebar"))
      }
    }
    NavLead.None -> {}
  }
}

/**
 * The first-run flow over every screen, once the paired computers are known: for a build that pairs, launched bare,
 * with none paired yet and the flow never finished. A launch that finds a computer already paired never shows it, and
 * it does not come back, even with every computer removed (apps/ios RootView `startWelcome`)
 */
@Composable
private fun Welcome(model: AppModel, pairing: cc.hotic.acpira.store.PairingModel) {
  val context = LocalContext.current
  val prefs = remember { context.getSharedPreferences(WELCOME_PREFS, android.content.Context.MODE_PRIVATE) }
  var welcoming by rememberSaveable { mutableStateOf(false) }
  val options = model.options
  LaunchedEffect(pairing.loaded) {
    if (!pairing.loaded || welcoming) return@LaunchedEffect
    if (options.welcome == null) {
      if (prefs.getBoolean(WELCOMED, false) || options.demo || options.engine != null) return@LaunchedEffect
      if (pairing.computers.isNotEmpty()) {
        prefs.edit().putBoolean(WELCOMED, true).apply()
        return@LaunchedEffect
      }
    }
    welcoming = true
  }
  // The add sheet stays down while the flow pairs in place, also after the activity is recreated
  SideEffect { pairing.onboarding = welcoming }
  val fade = if (model.motion) Motion.push else 0
  AnimatedVisibility(visible = welcoming, enter = fadeIn(tween(fade)), exit = fadeOut(tween(fade))) {
    WelcomeFlow(pairing, options.welcome, model.motion, onDone = {
      prefs.edit().putBoolean(WELCOMED, true).apply()
      welcoming = false
      pairing.onboarding = false
      // A pairing link the flow took leaves the add sheet asked for
      pairing.addOpen = false
    })
  }
}

private const val WELCOME_PREFS = "welcome"
private const val WELCOMED = "welcomed"

private fun key(screen: Screen): String = when (screen) {
  is Screen.Chat -> "chat:${screen.id}"
  is Screen.New -> "new"
  Screen.Settings -> "settings"
  is Screen.Computer -> "computer:${screen.id}"
  is Screen.AgentSettings -> "agent:${screen.id}"
  is Screen.ModelSettings -> "models:${screen.id}"
  Screen.Developer -> "developer"
  Screen.Providers -> "providers"
  is Screen.ProviderForm -> "provider:${screen.id}"
  is Screen.AddModels -> "addModels:${screen.id}"
  is Screen.Download -> "download:${screen.sessionId}:${screen.path}"
  is Screen.Changes -> "changes:${screen.session}"
  is Screen.File -> "file:${screen.session}:${screen.path}"
  is Screen.Terminal -> "terminal:${screen.cwd}"
  is Screen.Subagent -> "subagent:${screen.session}:${screen.id}"
  is Screen.Subagents -> "subagents:${screen.session}"
}
