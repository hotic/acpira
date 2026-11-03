package cc.hotic.acpira.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
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
import cc.hotic.acpira.design.IconStore
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.logic.exchanges
import cc.hotic.acpira.model.UserTurn
import cc.hotic.acpira.store.AppModel
import cc.hotic.acpira.store.Screen

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
  var draft by rememberSaveable(id) { mutableStateOf("") }
  val running = view?.running == true
  val pending = model.outgoing[id].orEmpty()
  val thread = rememberLazyListState()
  val items = remember(view?.turns) { view?.let { exchanges(it.turns) }.orEmpty() }
  val total = items.size + pending.size
  val context = LocalContext.current
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
          IconButton(onClick = { model.pop() }, modifier = Modifier.testTag("chat.back")) {
            Lucide("arrowLeft", Palette.fg1, size = Metrics.iconCtl, contentDescription = t("common.back"))
          }
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
            DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
              DropdownMenuItem(
                text = { Text(t(if (summary?.pinned == true) "common.unpin" else "common.pin")) },
                leadingIcon = { Lucide("pin", Palette.fg1) },
                onClick = { menu = false; model.togglePin(id) },
              )
              DropdownMenuItem(
                text = { Text(t("common.delete"), color = Palette.danger) },
                leadingIcon = { Lucide("trash2", Palette.danger) },
                onClick = { menu = false; model.delete(id); model.pop() },
              )
            }
          }
        },
        colors = TopAppBarDefaults.topAppBarColors(containerColor = Palette.bg0, scrolledContainerColor = Palette.bg0),
      )
    },
    bottomBar = {
      Composer(
        text = draft,
        onText = { draft = it },
        // The current model's vendor, as the iOS composer's model chip shows it; the agent's own mark without one
        mark = modelOption?.let { IconStore.brandOf(context, it.id) ?: IconStore.brandOf(context, it.name) } ?: agent,
        markName = modelOption?.name ?: model.agent(agent).name,
        running = running,
        placeholder = t(if (running) "composer.placeholder.queue" else "composer.placeholder").substringBefore("@").trimEnd(),
        onSend = {
          model.send(id, draft)
          draft = ""
          following = true
        },
        onStop = { model.stop(id) },
      )
    },
  ) { padding ->
    if (view == null) {
      Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { CircularProgressIndicator(color = Palette.fg3) }
      return@Scaffold
    }
    LazyColumn(
      state = thread,
      modifier = Modifier.fillMaxSize().testTag("chat.thread"),
      contentPadding = PaddingValues(top = padding.calculateTopPadding() + Metrics.s2, bottom = padding.calculateBottomPadding() + Metrics.pad),
      verticalArrangement = Arrangement.spacedBy(Metrics.msgGap),
    ) {
      items.forEachIndexed { e, exchange ->
        item(key = "x:${exchange.id}") {
          androidx.compose.foundation.layout.Column(verticalArrangement = Arrangement.spacedBy(Metrics.msgGap)) {
            exchange.user?.let { UserBubble(it) }
            for ((index, turn) in exchange.agents) {
              val last = e == items.size - 1 && index == exchange.agents.last().first
              AgentMessage(
                turn = turn,
                running = running && last,
                key = "$id:$index",
                now = { model.now().toEpochMilli() },
                onPermission = { block, option -> model.permission(id, block, option) },
              )
            }
          }
        }
      }
      pending.forEachIndexed { i, p -> item(key = "pending:$i") { UserBubble(UserTurn(text = p.text), pending = true) } }
    }
  }
}
