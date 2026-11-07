package cc.hotic.acpira.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import cc.hotic.acpira.design.readableWidth
import cc.hotic.acpira.design.readablePadding
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.IconButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.logic.exchanges
import cc.hotic.acpira.logic.retryableTurn
import cc.hotic.acpira.model.UserTurn
import cc.hotic.acpira.store.AppModel
import cc.hotic.acpira.store.Screen
import kotlinx.coroutines.delay

/**
 * A session's chat (apps/ios Chat/ChatScreen.swift): the top bar with back, new session and the session menu, the
 * thread by exchange, the composer at the bottom. The thread follows the newest output while the reader stays at the
 * end, and stays put once they scroll up
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ChatScreen(model: AppModel, id: String) {
  val view = model.views[id]
  val summary = model.summaries.firstOrNull { it.id == id }
  val agent = view?.agent ?: summary?.agent ?: model.settings.defaultAgent
  var draft by rememberSaveable(id) { mutableStateOf(model.options.draft.orEmpty()) }
  var staged by remember(id) { mutableStateOf<List<Staged>>(emptyList()) }
  val goo = remember(id) { Goo() }
  // `popup` launch option: a picker open on launch (screenshots), after the chips have been laid out
  LaunchedEffect(goo) {
    val popup = model.options.popup ?: return@LaunchedEffect
    if (model.motion) delay(POPUP_DELAY_MS)
    val parts = popup.split('.')
    goo.open = parts[0]
    goo.stage = parts.getOrNull(1)?.toIntOrNull() ?: 0
  }
  val running = view?.running == true
  val pending = model.outgoing[id].orEmpty()
  val thread = rememberLazyListState()
  val items = remember(view?.turns) { view?.let { exchanges(it.turns) }.orEmpty() }
  val total = items.size + pending.size
  val context = LocalContext.current
  // The settled turns' actions (retry, fork, toasts) and the one turn Retry is offered on
  val turnActions = remember(id) { TurnHandlers(retry = { model.retryTurn(id) }, fork = { model.fork(id, it) }, toast = model::showToast) }
  val retryAt = remember(view) { view?.let(::retryableTurn) }
  val modelOption = remember(view?.controls) {
    view?.controls?.options?.firstOrNull { it.category == "model" }?.let { c -> c.options.firstOrNull { it.id == c.value } }
  }

  // Follow the end while the reader is there: a new exchange or a growing last turn keeps it in view
  var following by remember { mutableStateOf(true) }
  LaunchedEffect(thread) {
    snapshotFlow { thread.layoutInfo.let { info -> (info.visibleItemsInfo.lastOrNull()?.index ?: -1) >= info.totalItemsCount - 1 } }
      .collect { atEnd -> if (!thread.isScrollInProgress) return@collect; following = atEnd }
  }
  LaunchedEffect(view?.turns, pending.size) {
    if (following && total > 0) thread.scrollToItem(total - 1, Int.MAX_VALUE / 2)
  }

  Box(Modifier.fillMaxSize()) {
  Scaffold(
    containerColor = Palette.bg0,
    topBar = {
      TopAppBar(
        title = {
          Text(
            view?.title ?: summary?.title.orEmpty(), maxLines = 1, overflow = TextOverflow.Ellipsis,
            fontSize = Type.text1, fontWeight = FontWeight.SemiBold, modifier = Modifier.testTag("chat.title"),
          )
        },
        navigationIcon = {
          LeadButton("chat.back") { model.pop() }
        },
        actions = {
          IconButton(onClick = { model.push(Screen.New(agent = agent, cwd = view?.cwd?.takeUnless { model.isChat(it) })) }, modifier = Modifier.testTag("chat.new")) {
            Lucide("squarePen", Palette.fg1, size = Metrics.iconCtl, contentDescription = t("session.new"))
          }
          var menu by remember { mutableStateOf(false) }
          Box {
            IconButton(onClick = { menu = true }, modifier = Modifier.testTag("chat.more")) {
              Lucide("ellipsis", Palette.fg1, size = Metrics.iconCtl, contentDescription = t("session.more"))
            }
            AppMenu(expanded = menu, onDismiss = { menu = false }) {
              AppMenuItem(
                t(if (summary?.pinned == true) "common.unpin" else "common.pin"),
                onClick = { menu = false; model.togglePin(id) },
                icon = { MenuGlyph(if (summary?.pinned == true) "pinOff" else "pin") },
              )
              AppMenuGap()
              AppMenuItem(
                t("common.delete"),
                onClick = { menu = false; model.delete(id); model.pop() },
                danger = true,
                icon = { MenuGlyph("trash2", danger = true) },
              )
            }
          }
        },
        colors = TopAppBarDefaults.topAppBarColors(containerColor = Palette.bg0, scrolledContainerColor = Palette.bg0),
      )
    },
    bottomBar = {
      // The composer holds to the reading column in a wide window, as the thread above it does
      Box(Modifier.readableWidth()) {
      Composer(
        text = draft,
        onText = { draft = it },
        staged = staged,
        onStaged = { staged = it },
        controls = view?.controls ?: cc.hotic.acpira.model.SessionControls(),
        agentMark = agent,
        agentName = model.agent(agent).name,
        running = running,
        usage = view?.usage,
        placeholder = t(if (running) "composer.placeholder.queue" else "composer.placeholder").substringBefore("@").trimEnd(),
        goo = goo,
        motion = model.motion,
        onSend = {
          model.send(id, draft, staged.map { it.draft })
          draft = ""
          staged = emptyList()
          following = true
        },
        onStop = { model.stop(id) },
        onSetMode = { model.setMode(id, it) },
        onSetConfig = { config, value -> model.setConfig(id, config, value) },
        onToast = model::showToast,
      )
      }
    },
  ) { padding ->
    if (view == null) {
      Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { CircularProgressIndicator(color = Palette.fg3) }
      return@Scaffold
    }
    BoxWithConstraints(Modifier.fillMaxSize()) {
    LazyColumn(
      state = thread,
      modifier = Modifier.fillMaxSize().gooRecede(goo, model.motion).testTag("chat.thread"),
      // The list stays full width so a drag beside the reading column still scrolls it
      contentPadding = readablePadding(maxWidth, top = padding.calculateTopPadding() + Metrics.s2, bottom = padding.calculateBottomPadding() + Metrics.pad),
      verticalArrangement = Arrangement.spacedBy(Metrics.msgGap),
    ) {
      items.forEachIndexed { e, exchange ->
        item(key = "x:${exchange.id}") {
          androidx.compose.foundation.layout.Column(verticalArrangement = Arrangement.spacedBy(Metrics.msgGap)) {
            exchange.user?.let { UserBubble(it, onToast = turnActions.toast) }
            for ((index, turn) in exchange.agents) {
              val last = e == items.size - 1 && index == exchange.agents.last().first
              AgentMessage(
                turn = turn,
                running = running && last,
                key = "$id:$index",
                now = { model.clockNow().toEpochMilli() },
                onPermission = { block, option -> model.permission(id, block, option) },
                index = index,
                retry = index == retryAt,
                settings = exchange.user?.settings,
                controls = view.controls,
                actions = turnActions,
              )
            }
          }
        }
      }
      pending.forEachIndexed { i, p -> item(key = "pending:$i") { UserBubble(UserTurn(text = p.text), pending = true) } }
    }
    }
  }
  // The composer's open picker, over everything (the composer included), so no piece hides under the card
  GooLayer(goo, model.motion)
  }
}

/** How long a launch's picker waits before opening, so a recording shows it come out */
private const val POPUP_DELAY_MS = 1_000L
