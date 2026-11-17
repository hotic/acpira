package cc.hotic.acpira.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import cc.hotic.acpira.design.footFade
import cc.hotic.acpira.design.readableWidth
import cc.hotic.acpira.design.topFade
import cc.hotic.acpira.design.readablePadding
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.IconButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.nestedscroll.NestedScrollConnection
import androidx.compose.ui.input.nestedscroll.NestedScrollSource
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.layout.Remeasurement
import androidx.compose.ui.layout.RemeasurementModifier
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.LocalLifecycleOwner
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Velocity
import androidx.lifecycle.Lifecycle
import cc.hotic.acpira.design.Haptic
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.design.play
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.logic.canEditPrompts
import cc.hotic.acpira.logic.canSetGoal
import cc.hotic.acpira.logic.editTurnConfig
import cc.hotic.acpira.logic.editingView
import cc.hotic.acpira.logic.EarlierReach
import cc.hotic.acpira.logic.ThreadItem
import cc.hotic.acpira.logic.ThreadStart
import cc.hotic.acpira.logic.holdPlace
import cc.hotic.acpira.logic.heldPrompt
import cc.hotic.acpira.logic.heldStage
import cc.hotic.acpira.logic.lastTypedPrompt
import cc.hotic.acpira.logic.pendingPlanReview
import cc.hotic.acpira.logic.retryableTurn
import cc.hotic.acpira.logic.reviseOption
import cc.hotic.acpira.logic.turnEndHaptic
import cc.hotic.acpira.model.AgentTurn
import cc.hotic.acpira.model.GoalAction
import cc.hotic.acpira.model.SessionControls
import cc.hotic.acpira.model.SessionStatus
import cc.hotic.acpira.model.SessionView
import cc.hotic.acpira.model.SubagentState
import cc.hotic.acpira.model.UserTurn
import cc.hotic.acpira.store.AppModel
import cc.hotic.acpira.store.Screen
import cc.hotic.acpira.store.beginEdit
import cc.hotic.acpira.store.beginEditLast
import cc.hotic.acpira.store.buildPlan
import cc.hotic.acpira.store.compact
import cc.hotic.acpira.store.editQueued
import cc.hotic.acpira.store.failureAction
import cc.hotic.acpira.store.stopCommand
import cc.hotic.acpira.store.stopTask
import cc.hotic.acpira.store.goal
import cc.hotic.acpira.store.openAccount
import cc.hotic.acpira.store.personas
import cc.hotic.acpira.store.quote
import cc.hotic.acpira.store.seedQuotes
import cc.hotic.acpira.store.refreshQuota
import cc.hotic.acpira.store.removeAccount
import cc.hotic.acpira.store.selectAccount
import cc.hotic.acpira.store.sendEdit
import cc.hotic.acpira.store.sendQueued
import cc.hotic.acpira.store.takeQueued
import cc.hotic.acpira.store.openTerminal
import cc.hotic.acpira.logic.countsLabel
import cc.hotic.acpira.logic.isSidekick
import cc.hotic.acpira.logic.linkOnly
import cc.hotic.acpira.logic.nodesByTurn
import cc.hotic.acpira.logic.shownReports
import cc.hotic.acpira.ui.prose.LocalThreadTaps
import cc.hotic.acpira.ui.prose.ThreadTaps
import cc.hotic.acpira.ui.prose.threadTaps
import cc.hotic.acpira.ui.subagents.LocalAgentNames
import cc.hotic.acpira.ui.subagents.LocalAllSubagents
import cc.hotic.acpira.ui.subagents.LocalInspectSubagent
import cc.hotic.acpira.ui.subagents.LocalSidekickModels
import cc.hotic.acpira.ui.subagents.SidekickModels
import cc.hotic.acpira.ui.subagents.SubagentReturns
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * A session's chat (apps/ios Chat/ChatScreen.swift): the top bar with back, new session and the session menu (a pill
 * saying the agent is starting in the title's place while it does), the thread turn by turn with queued prompts at its
 * end, the to-do card floating over its bottom with the way back to the latest message, and the dock (ui/Dock.kt) at
 * the bottom. The thread follows the newest output while the reader stays at the end, and stays put once they scroll up
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ChatScreen(model: AppModel, id: String) {
  val view = model.views[id]
  val summary = model.summaries.firstOrNull { it.id == id }
  val agent = view?.agent ?: summary?.agent ?: model.settings.defaultAgent
  // The unsent prompt and attachments live in the app model, so leaving the chat and coming back keeps them, and a
  // prompt edit can fill the composer
  val draft = remember(id, model) { model.chat.draft(id) }
  LaunchedEffect(draft) {
    if (draft.seeded) return@LaunchedEffect
    draft.seeded = true
    model.options.draft?.let { if (draft.text.isEmpty()) draft.text = it }
  }
  val goo = remember(id) { Goo() }
  // `popup` launch option: a picker open on launch (screenshots), after the chips have been laid out; `account` and
  // `edit` raise the account sheet or take the newest prompt in for editing
  LaunchedEffect(goo) {
    val popup = model.options.popup ?: return@LaunchedEffect
    // The media layer opens its own popup
    if (popup == "lightbox") return@LaunchedEffect
    if (model.motion) delay(POPUP_DELAY_MS)
    when (popup) {
      "account" -> return@LaunchedEffect model.openAccount(id)
      "edit" -> return@LaunchedEffect model.beginEditLast(id)
      "quotes" -> return@LaunchedEffect model.seedQuotes(id)
    }
    val parts = popup.split('.')
    goo.open = parts[0]
    goo.stage = parts.getOrNull(1)?.toIntOrNull() ?: 0
    // `model.0.sidekick`: the sidekick's own beans, `model.N.sidekick` one of their panels
    goo.deeper = parts.getOrNull(2)
  }
  // A prompt the session holds back (starting, replaying remembered controls) shows as sent, and so does the phone's
  // own prompt still on its way to the engine; the working row says which stage holds it (HeldPrompt.kt)
  val pending = model.outgoing[id].orEmpty()
  val agentName = model.agent(agent).name
  val held = remember(view, pending) { view?.let { heldPrompt(it, pending) } }
  val stage = remember(view, pending, agentName) { view?.let { heldStage(it, pending, agentName) } }
  val running = held?.running == true
  // A turn ending live in this chat, the app in front (AppModel+Engine.swift `present`); a view this wire has not shown
  // yet (syncing) only reports how a turn ended before
  val haptics = LocalHapticFeedback.current
  val lifecycle = LocalLifecycleOwner.current.lifecycle
  val live = view?.running == true
  val wasLive = remember(id, model) { booleanArrayOf(live) }
  LaunchedEffect(live) {
    if (wasLive[0] && !live && !model.syncing(id) && lifecycle.currentState.isAtLeast(Lifecycle.State.RESUMED)) {
      view?.let(::turnEndHaptic)?.let { haptics.play(it) }
    }
    wasLive[0] = live
  }
  val thread = rememberLazyListState()
  val turns = held?.turns.orEmpty()
  val queued = held?.queued.orEmpty()
  // The thread holds one item per turn, keyed by the turn's place in the session, and a lead item ahead of them while
  // the view leaves part of the session out (a windowed view, logic/ThreadWindow.kt): a chunk of the earlier part then
  // only adds items in front, and no item changes its key
  val start = remember(held?.earlier) { held?.let(ThreadStart::of) }
  val before = start?.turns ?: 0
  val lead = start?.lead ?: 0
  val total = lead + turns.size + if (queued.isEmpty()) 0 else 1
  // What a prompt's long-press menu offers: edit while the session is idle, Set as goal on the newest typed prompt
  val editable = held?.let(::canEditPrompts) == true
  val goalPrompt = remember(held) { held?.takeIf(::canSetGoal)?.let { lastTypedPrompt(it.turns) } }
  val handlers = remember(id, model) {
    ChatHandlers(
      permission = { b, o -> model.permission(id, b, o) },
      buildPlan = { plan, option, pick -> model.buildPlan(id, plan, option, pick) },
      editPrompt = { model.beginEdit(id, it) },
      goal = { action, objective -> model.goal(id, action, objective) },
      sendQueued = { model.sendQueued(id, it) },
      editQueued = { model.editQueued(id, it) },
      removeQueued = { model.takeQueued(id, it) },
      failureAction = { model.failureAction(id, it) },
      stopTask = { model.stopTask(id, it) },
      stopCommand = { model.stopCommand(id, it) },
    )
  }
  // The settled turns' actions (retry, fork, toasts) and the one turn Retry is offered on
  val turnActions = remember(id) { TurnHandlers(retry = { model.retryTurn(id) }, fork = { model.fork(id, it) }, toast = model::showToast) }
  val retryAt = remember(held) { held?.let(::retryableTurn) }
  // Selected reply text quotes into this chat's composer; `@` offers the personas and the engine's file search
  val quote = remember(id, model) { { text: String -> model.quote(id, text) } }
  val personas = remember(model.settings, model.agents) { model.personas }
  // The sent prompts paint the tokens the composer paints: the session's commands and the personas (PromptMarks.swift)
  val marks = remember(view?.commands, personas) { PromptMarkNames(view?.commands.orEmpty().map { it.name }, personas.map { it.name }) }
  // Each child reaches only the turn that announced it; the sidekick's rounds are the lead's own relay, not subagents:
  // they neither offer the graph nor count in it (ChatScreen.swift helpers)
  val nodes = remember(held?.subagents) { nodesByTurn(held?.subagents.orEmpty()) }
  val allNodes = remember(held?.subagents) { held?.subagents.orEmpty().map { it.linkOnly } }
  val helpers = remember(view?.subagents) { view?.subagents.orEmpty().filter { !isSidekick(it) } }
  val context = androidx.compose.ui.platform.LocalContext.current
  val sidekickModels = remember(held?.controls, held?.agent) { held?.controls?.let { SidekickModels.of(context, it, held.agent) } ?: SidekickModels() }
  val inspect = remember(id, model) { { child: String -> model.push(Screen.Subagent(id, child)) } }
  // A diff whose view left its files out asks the engine for them (a demo has none to ask). Keyed on the model too: a
  // paired reconnect brings a new one for the same session, and the old one's client is closed
  val diffSource = remember<(suspend (DiffLocation) -> cc.hotic.acpira.model.FullDiffSource?)?>(id, model, model.live) {
    if (!model.live) null else { at: DiffLocation -> model.files.diffSource(id, at.toolCallId, at.nth) }
  }

  // Follow the end while the reader is there: a new exchange or a growing last turn keeps it in view
  var following by remember { mutableStateOf(true) }
  LaunchedEffect(thread) {
    snapshotFlow { thread.layoutInfo.let { info -> (info.visibleItemsInfo.lastOrNull()?.index ?: -1) >= info.totalItemsCount - 1 } }
      .collect { atEnd -> if (!thread.isScrollInProgress) return@collect; following = atEnd }
  }
  LaunchedEffect(held?.turns, queued.size) {
    if (following && total > 0) thread.scrollToItem(total - 1, Int.MAX_VALUE / 2)
  }
  // The last exchange can still grow after it is laid out (prose measured late, a card's actions arriving), and the dock
  // under it can too: keep the thread's end in view while following
  LaunchedEffect(thread) {
    snapshotFlow {
      val info = thread.layoutInfo
      info.visibleItemsInfo.lastOrNull()?.let { listOf(it.index, it.size, info.afterContentPadding, info.viewportEndOffset) }
    }.collect { last ->
      val count = thread.layoutInfo.totalItemsCount
      if (last == null || !following || thread.isScrollInProgress || last[0] != count - 1) return@collect
      thread.scrollToItem(count - 1, Int.MAX_VALUE / 2)
    }
  }
  // A chunk of the earlier part went in front of what the thread shows: what the reader looks at stays where it is.
  // Read off the layout from before the chunk, here, ahead of the list's measure with it (holdPlace)
  val startSeen = remember(id) { arrayOfNulls<ThreadStart>(1) }
  val reach = remember(id) { EarlierReach() }
  val grown = remember(thread) { GrownHold(thread) }
  SideEffect {
    val old = startSeen[0]
    startSeen[0] = start
    if (old == null || start == null) return@SideEffect
    // A whole view opened the window anew (a chunk only ever moves the start ahead)
    if (start != old && !start.aheadOf(old)) reach.opened()
    if (!start.aheadOf(old)) return@SideEffect
    val shown = thread.layoutInfo.visibleItemsInfo.map { ThreadItem(it.key, it.index, it.offset, it.size) }
    val hold = holdPlace(old, start, shown) ?: return@SideEffect
    // "Following" only says the last turn is in view. A reader further up inside it (a long last turn is where a
    // window gets cut) is not at the end: the chunk changed the turns, and following would take them there
    if (thread.canScrollForward) following = false
    hold.request?.let { (index, offset) -> thread.requestScrollToItem(index, offset) }
    grown.pending = hold.grown
  }
  // When the part ahead of a windowed thread is asked for (EarlierReach): by the reader's own drag or fling towards
  // the start with it within reach, never by where the layout stands. The scrolls issued here (holding the place, following the
  // end) do not come through nested scroll, so they do not count as the reader's
  val reaching = remember(id, model, thread, reach) {
    object : NestedScrollConnection {
      override fun onPostScroll(consumed: Offset, available: Offset, source: NestedScrollSource): Offset {
        // y grows towards the thread's start: what the list took of a drag or fling that way, and what it left
        if (thread.nearStart() && reach.reached(consumed.y, available.y, source == NestedScrollSource.UserInput)) model.loadEarlier(id)
        return Offset.Zero
      }
      // Every drag ends here, whatever its speed
      override suspend fun onPreFling(available: Velocity): Velocity {
        reach.released()
        return Velocity.Zero
      }
    }
  }
  // A thread that opens too short to scroll would leave the reader nothing to scroll for: it fills itself, a few parts
  // at most. Asked once the list is laid out from this start, a frame on
  LaunchedEffect(id, start) {
    if (start == null || start.lead == 0) return@LaunchedEffect
    withFrameNanos { }
    if (reach.laidOut(thread.canScrollBackward || thread.canScrollForward)) model.loadEarlier(id)
  }
  // Back to the latest: offered while the thread's end sits more than half a screen below the view
  val far by remember {
    derivedStateOf {
      val info = thread.layoutInfo
      val last = info.visibleItemsInfo.lastOrNull() ?: return@derivedStateOf false
      if (last.index < info.totalItemsCount - 1) return@derivedStateOf true
      val viewport = info.viewportEndOffset - info.viewportStartOffset
      last.offset + last.size - (info.viewportEndOffset - info.afterContentPadding) > viewport / 2
    }
  }
  val scope = rememberCoroutineScope()
  // A reply block or prompt handed to range selection drops it on a tap elsewhere in the thread
  val threadTaps = remember(id) { ThreadTaps() }
  // The older reply whose action row a tap has opened: one per chat
  val turnReveal = remember(id) { TurnReveal() }
  // The floating to-do card's height: the thread's end clears it
  var planDockH by remember { mutableIntStateOf(0) }

  // The chat's images open in its Lightbox (ui/MediaViews.kt)
  MediaLayer(id, model) {
  Box(Modifier.fillMaxSize()) {
  Scaffold(
    containerColor = Palette.bg0,
    topBar = {
      TopAppBar(
        title = {
          StartingTitle(active = held?.status == SessionStatus.starting && !running, agentName = agentName) {
            Text(
              view?.title ?: summary?.title.orEmpty(), maxLines = 1, overflow = TextOverflow.Ellipsis,
              fontSize = Type.text1, fontWeight = FontWeight.SemiBold, modifier = Modifier.testTag("chat.title"),
            )
          }
        },
        navigationIcon = {
          LeadButton("chat.back") { model.pop() }
        },
        actions = {
          IconButton(onClick = { model.push(Screen.New(agent = agent, cwd = view?.cwd?.takeUnless { model.isChat(it) })) }, modifier = Modifier.testTag("chat.new")) {
            Lucide("squarePen", Palette.fg1, size = Metrics.iconCtl, contentDescription = t("session.new"))
          }
          // The session's "…" menu, shared with the list's rows (ui/SessionMenu.kt)
          ChatMenuButton(
            model, id,
            ChatEntries(
              account = { model.openAccount(id) }.takeIf { hasAccountMenu(model.agent(agent)) },
              changes = { model.push(Screen.Changes(id)) }.takeIf { !view?.cwd.isNullOrEmpty() },
              subagents = { model.push(Screen.Subagents(id)) }.takeIf { helpers.isNotEmpty() },
              subagentCounts = countsLabel(helpers),
              terminal = { view?.cwd?.takeIf { it.isNotEmpty() }?.let { model.openTerminal(it) } ?: Unit },
              settings = { model.push(Screen.Settings) },
            ),
          )
        },
        colors = TopAppBarDefaults.topAppBarColors(containerColor = Palette.bg0, scrolledContainerColor = Palette.bg0),
      )
    },
    bottomBar = {
      // The dock holds to the reading column in a wide window, as the thread above it does; the page colour fades up
      // under it at the full width, the gesture bar's strip included (design/Layout.kt footFade)
      Box(Modifier.fillMaxWidth().background(footFade()).readableWidth()) {
      val edit = draft.edit
      val dockView = view ?: SessionView(id = id, agent = agent)
      Dock(model, id, dockView, editing = edit?.original) {
        // While a sent prompt is rewritten the same composer edits it: its pickers act on the edit's own selections and
        // send rewrites the turn (Dock.swift, PromptEdit.swift)
        val composing = if (edit != null) editingView(dockView, edit.controls) else dockView
        val revising = edit == null && pendingPlanReview(dockView.turns)?.let(::reviseOption) != null
        Composer(
          text = draft.text,
          onText = { draft.text = it },
          staged = draft.staged,
          onStaged = { draft.staged = it },
          controls = composing.controls,
          agentMark = agent,
          agentName = agentName,
          running = edit == null && running,
          usage = composing.usage,
          placeholder = if (revising) t("composer.placeholder.revisePlan")
            else t(if (running) "composer.placeholder.queue" else "composer.placeholder").substringBefore("@").trimEnd(),
          goo = goo,
          motion = model.motion,
          onSend = {
            if (draft.edit != null) model.sendEdit(id)
            else {
              model.send(id, draft.text, draft.staged.map { it.draft })
              draft.text = ""
              draft.staged = emptyList()
            }
            following = true
          },
          onStop = { model.stop(id) },
          onSetMode = { mode ->
            val e = draft.edit
            if (e != null) draft.edit = e.copy(controls = e.controls.copy(modeId = mode)) else model.setMode(id, mode)
          },
          onSetConfig = { config, value ->
            val e = draft.edit
            if (e != null) draft.edit = e.copy(controls = editTurnConfig(e.controls, dockView.controls, config, value, dockView.modelShapes))
            else model.setConfig(id, config, value)
          },
          onToast = model::showToast,
          assist = ComposerAssist(composing.commands, personas, model.fileSearch),
          sidekickLive = dockView.subagents.orEmpty().any { isSidekick(it) && it.state == SubagentState.running },
          usageCard = UsageCardActions(
            compactAt = model.compactAt,
            canCompact = dockView.commands.any { it.name == "compact" },
            onCompact = { model.compact(id) },
          ),
          syncing = model.syncing(id),
        )
      }
      }
    },
  ) { padding ->
    if (view == null || held == null) {
      // The list arrives before the transcript, which can take a while over a paired link; there is no byte progress on
      // the wire, so the spinner stays until the session itself arrives, or the reason it will not
      // (AppModel.unavailable) with a way to ask again (ChatScreen.swift)
      val reason = model.unavailable[id]
      Column(
        Modifier.fillMaxSize(),
        verticalArrangement = Arrangement.spacedBy(if (reason == null) Metrics.s2 else Metrics.s1, Alignment.CenterVertically),
        horizontalAlignment = Alignment.CenterHorizontally,
      ) {
        if (reason != null) {
          // A short headline carries the page; the engine's own sentence sits under it, small and in a narrow column
          Text(t("mobile.session.unavailable"), color = Palette.fg1, fontSize = Type.text1, fontWeight = FontWeight.Medium)
          Text(reason, color = Palette.fg3, fontSize = Type.text3, textAlign = TextAlign.Center, modifier = Modifier.widthIn(max = Metrics.waitNoteW).testTag("chat.unavailable"))
          CardButton(t("common.retry"), primary = false, modifier = Modifier.padding(top = Metrics.s3).testTag("chat.retry"), onClick = { model.retryOpen(id) })
        } else {
          CircularProgressIndicator(color = Palette.fg2)
          Text(t("session.import.loading"), color = Palette.fg3, fontSize = Type.text2, modifier = Modifier.testTag("chat.loading"))
        }
      }
      return@Scaffold
    }
    BoxWithConstraints(Modifier.fillMaxSize()) {
    val planDock = with(LocalDensity.current) { planDockH.toDp() }
    val canSend = held.status == SessionStatus.ready || held.status == SessionStatus.starting
    val widgetPrompt = remember(id, model, canSend, haptics) {
      if (!canSend) null else { text: String ->
        haptics.play(Haptic.Send)
        model.send(id, text)
        following = true
      }
    }
    CompositionLocalProvider(
      LocalChatHandlers provides handlers,
      LocalCanStopCommands provides (held.canStopCommands == true),
      LocalPlanContext provides PlanContext(held.controls, held.running, held.status == SessionStatus.ready),
      LocalLiveStage provides stage,
      LocalQuote provides quote,
      LocalWidgetPrompt provides widgetPrompt,
      // A subagent row pushes that child's inspector
      LocalInspectSubagent provides inspect,
      LocalAllSubagents provides allNodes,
      LocalSidekickModels provides sidekickModels,
      LocalAgentNames provides { a: String -> model.agent(a).name },
      LocalSessionSummaries provides model.summaries,
      LocalDiffSource provides diffSource,
      LocalPromptMarks provides marks,
      LocalThreadTaps provides threadTaps,
      LocalTurnReveal provides turnReveal,
    ) {
    // Messages leave under the bar through a fade, once the thread has moved off its top
    val scrolled by remember(thread) { derivedStateOf { thread.canScrollBackward } }
    LazyColumn(
      state = thread,
      modifier = Modifier.fillMaxSize().gooRecede(goo, model.motion).topFade(scrolled, padding.calculateTopPadding())
        .nestedScroll(reaching).then(grown).onGloballyPositioned { grown.settle() }
        .threadTaps(threadTaps).testTag("chat.thread"),
      // The list stays full width so a drag beside the reading column still scrolls it
      contentPadding = readablePadding(maxWidth, top = padding.calculateTopPadding() + Metrics.s2, bottom = padding.calculateBottomPadding() + planDock + Metrics.pad),
      verticalArrangement = Arrangement.spacedBy(Metrics.msgGap),
    ) {
      // The part of the session ahead of the view is asked for as the reader reaches for it; the lead keeps its room
      // whether or not a request is on its way, so the spinner moves nothing
      if (lead > 0) {
        item(key = "earlier", contentType = "earlier") {
          Box(Modifier.fillMaxWidth().height(Metrics.icon).testTag("chat.earlier"), contentAlignment = Alignment.Center) {
            if (model.loadingEarlier(id)) {
              CircularProgressIndicator(Modifier.size(Metrics.icon).testTag("chat.earlier.loading"), color = Palette.fg3, strokeWidth = Metrics.pairCodeLine)
            }
          }
        }
      }
      // The selections of the prompt each reply answers; a window can open on a reply whose prompt is still left out
      var prompt: UserTurn? = null
      turns.forEachIndexed { index, turn ->
        // The turn's place in the session: its key, and what the engine is told (edit, fork, a subagent's anchor)
        val at = before + index
        when (turn) {
          is UserTurn -> {
            prompt = turn
            item(key = at, contentType = "user") {
              Column(verticalArrangement = Arrangement.spacedBy(Metrics.msgGap)) {
                UserBubble(
                  turn, onToast = turnActions.toast,
                  onEdit = if (editable) ({ handlers.editPrompt(at) }) else null,
                  onGoal = if (index == goalPrompt) ({ handlers.goal(GoalAction.set, turn.text.trim()) }) else null,
                )
                // The subagent returns that woke this reply, in the relay handback's shape (ThreadItems.swift)
                if (turn.auto == true && turn.autoReason == "subagents") {
                  val reports = remember(turn.text) { shownReports(turn.text) }
                  if (reports.isNotEmpty()) SubagentReturns(reports)
                }
              }
            }
          }
          is AgentTurn -> {
            val settings = prompt?.settings
            item(key = at, contentType = "agent") {
              val last = index == turns.size - 1
              AgentMessage(
                turn = turn,
                running = running && last,
                key = "$id:$at",
                now = { model.clockNow().toEpochMilli() },
                onPermission = { block, option -> model.permission(id, block, option) },
                index = at,
                retry = index == retryAt,
                settings = settings,
                controls = held.controls,
                actions = turnActions,
                subagents = nodes[at].orEmpty(),
                last = last,
                cut = if (index == 0) start?.blocks ?: 0 else 0,
              )
            }
          }
        }
      }
      // Prompts queued behind the running turn, as faint bubbles after it: each goes once the turn ends, or steers it
      if (queued.isNotEmpty()) {
        item(key = "queue") {
          Column(verticalArrangement = Arrangement.spacedBy(Metrics.msgGap)) {
            val steer = held.canSteer == true && held.running
            for (q in queued) {
              androidx.compose.runtime.key(q.id) {
                QueueBubble(q, steer, onSend = { handlers.sendQueued(q.id) }, onEdit = { handlers.editQueued(q.id) }, onRemove = { handlers.removeQueued(q.id) })
              }
            }
          }
        }
      }
    }
    }
    // The docked to-do card floats over the thread's bottom, the way back to the latest message above it
    Column(
      Modifier.align(Alignment.BottomCenter).readableWidth().padding(bottom = padding.calculateBottomPadding()),
      horizontalAlignment = Alignment.End,
    ) {
      ScrollToLatest(shown = far && !(running && following), modifier = Modifier.padding(end = Metrics.pad, bottom = Metrics.s2)) {
        following = true
        val end = maxOf(total - 1, 0)
        scope.launch { if (model.motion) thread.animateScrollToItem(end, Int.MAX_VALUE / 2) else thread.scrollToItem(end, Int.MAX_VALUE / 2) }
      }
      // It steps back under an open picker, or its words read through the beans' captions
      Box(Modifier.fillMaxWidth().onSizeChanged { planDockH = it.height }.gooAside(goo, model.motion)) { PlanDock(held) { handlers.goal(it, null) } }
    }
    }
  }
  // The composer's open picker, over everything (the composer included), so no piece hides under the card
  GooLayer(goo, model.motion)
  // The logins as a sheet; the quotas are fetched as it comes up, as on the desktop (AccountQuota.swift)
  if (model.chat.accountSheet == id) {
    val info = model.agent(agent)
    AccountSheet(
      info, model.chat.accounts.filter { it.agent == info.id }, view?.accountId, model::clockNow,
      onSelect = { model.selectAccount(id, it) },
      onRemove = { model.removeAccount(it) },
      onAppear = { model.refreshQuota(info.id) },
      onDismiss = { model.chat.accountSheet = null },
    )
  }
  }
  }
}

/** How long a launch's picker waits before opening, so a recording shows it come out */
private const val POPUP_DELAY_MS = 1_000L

/** The items at the thread's start a reader counts as near it from: the lead, then a prompt and its reply */
private const val EARLIER_REACH = 2

/** The reader is within reach of the thread's start: among its first items, and less than a screen into the one on top */
private fun LazyListState.nearStart(): Boolean {
  val info = layoutInfo
  val first = info.visibleItemsInfo.firstOrNull() ?: return true
  return first.index <= EARLIER_REACH && -first.offset < info.viewportEndOffset - info.viewportStartOffset
}

/**
 * Puts the end of a turn that grew at its top back where it stood (ThreadHold.grown), in the frame that laid it out
 * again. `settle` runs from a positioned callback, after the layout and ahead of the draw; the scroll alone would be
 * placed a frame later (the list only invalidates its placement for it), so the list is remeasured by hand
 */
private class GrownHold(private val list: LazyListState) : RemeasurementModifier {
  /** The turn's key and where its end stood, until the list has laid it out again */
  var pending: Pair<Int, Int>? = null
  private var remeasurement: Remeasurement? = null

  override fun onRemeasurementAvailable(remeasurement: Remeasurement) {
    this.remeasurement = remeasurement
  }

  fun settle() {
    val (key, end) = pending ?: return
    pending = null
    val now = list.layoutInfo.visibleItemsInfo.firstOrNull { it.key == key } ?: return
    val moved = now.offset + now.size - end
    if (moved == 0) return
    list.dispatchRawDelta(moved.toFloat())
    remeasurement?.forceRemeasure()
  }
}
