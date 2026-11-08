package cc.hotic.acpira.ui.subagents

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.IconButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.WebMetrics
import cc.hotic.acpira.design.WebType
import cc.hotic.acpira.design.readablePadding
import cc.hotic.acpira.design.readableWidth
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.logic.exchanges
import cc.hotic.acpira.logic.receiptOnly
import cc.hotic.acpira.logic.statusLabel
import cc.hotic.acpira.logic.subagentTitle
import cc.hotic.acpira.model.AgentTurn
import cc.hotic.acpira.model.SessionView
import cc.hotic.acpira.model.SubagentState
import cc.hotic.acpira.model.SubagentSummary
import cc.hotic.acpira.model.SubagentTranscript
import cc.hotic.acpira.model.SubagentVisibility
import cc.hotic.acpira.model.UserTurn
import cc.hotic.acpira.store.AppModel
import cc.hotic.acpira.store.answer
import cc.hotic.acpira.ui.AcpRow
import cc.hotic.acpira.ui.AgentMessage
import cc.hotic.acpira.ui.AppMenu
import cc.hotic.acpira.ui.AppMenuItem
import cc.hotic.acpira.ui.ChatHandlers
import cc.hotic.acpira.ui.LocalChatHandlers
import cc.hotic.acpira.ui.LocalRowHeight
import cc.hotic.acpira.ui.MenuGlyph
import cc.hotic.acpira.ui.Orb
import cc.hotic.acpira.ui.QuestionCard
import cc.hotic.acpira.ui.RowLabel
import cc.hotic.acpira.ui.StreamProse
import cc.hotic.acpira.ui.ToolPageBar
import cc.hotic.acpira.ui.UserBubble
import cc.hotic.acpira.model.TextBlock

// Ported from apps/ios Subagents/SubagentInspector.swift (chat/subagents/SubagentInspector.tsx as a pushed page): one
// delegated child's transcript, read-only. The delegated task is the child's prompt, so it heads the page as a user
// bubble; the child's turns follow with the main thread's views. Nothing here steers the session: no composer, no prompt
// edit, retry or fork. The child's own approval and question cards answer through the parent session, and a running
// child the agent lets stop has "Request stop" in the bar's menu. The page follows exactly one child: it observes once
// the parent's view is here and stops when it leaves (SubagentFeed).

@Composable
fun SubagentInspector(model: AppModel, session: String, id: String) {
  val root = model.views[session]
  val node = root?.subagents?.firstOrNull { it.id == id }
  // Observed once the parent's view is here: an observe the engine reads before it shows the session has no transcript
  // to answer with, and is not answered later. A parent carried over a reconnect counts once the new wire shows it
  val shown = model.shown(session)
  LaunchedEffect(shown) { if (shown) model.subagents.observe(session, id) }
  DisposableEffect(session, id) { onDispose { model.subagents.unobserve(session, id) } }
  if (root == null || node == null) {
    // The parent's view still on its way (a page opened from launch), or the child gone with an edit
    Scaffold(containerColor = Palette.bg0, topBar = { ToolPageBar(t("subagents.untitled"), "subagent", model::pop) }) {}
    return
  }
  val transcript = model.subagents.transcript(session, id)
  val cancelable = node.state == SubagentState.running && node.controls.cancel
  var menu by remember { mutableStateOf(false) }
  val handlers = remember(session, model) {
    // Only the child's cards act, through the parent session; everything else is a no-op
    ChatHandlers(
      permission = { b, o -> model.permission(session, b, o) },
      buildPlan = { _, _, _ -> }, editPrompt = {}, goal = { _, _ -> }, sendQueued = {}, editQueued = {}, removeQueued = {},
    )
  }
  // Keyed on the model too: a paired reconnect brings a new one whose client is the live one
  val diffSource = remember<(suspend (cc.hotic.acpira.ui.DiffLocation) -> cc.hotic.acpira.model.FullDiffSource?)?>(session, id, model, model.live) {
    if (!model.live) null else { at: cc.hotic.acpira.ui.DiffLocation -> model.files.diffSource(session, at.toolCallId, at.nth, subagentId = id) }
  }
  Scaffold(
    containerColor = Palette.bg0,
    topBar = {
      ToolPageBar(subagentTitle(node), "subagent", model::pop, subtitle = statusLabel(node)) {
        if (cancelable) {
          Box {
            IconButton(onClick = { menu = true }, modifier = Modifier.testTag("subagent.more")) {
              Lucide("ellipsis", Palette.fg1, size = Metrics.iconCtl, contentDescription = t("session.more"))
            }
            AppMenu(expanded = menu, onDismiss = { menu = false }) {
              val asked = node.cancelRequested == true
              AppMenuItem(
                t(if (asked) "subagents.cancelling" else "subagents.cancel"), danger = !asked,
                onClick = { menu = false; if (!asked) model.subagents.cancel(session, id) },
                icon = { MenuGlyph("square", danger = !asked) }, modifier = Modifier.testTag("subagent.cancel"),
              )
            }
          }
        }
      }
    },
    bottomBar = {
      // The child's open question: the parent's dock does not show it on the phone, so its card sits here
      node.question?.let { q ->
        Box(Modifier.readableWidth().padding(horizontal = Metrics.pad).padding(bottom = Metrics.s2).navigationBarsPadding()) {
          androidx.compose.runtime.key(q.id) {
            QuestionCard(q) { answers, skipped -> model.answer(session, q.id, answers, skipped) }
          }
        }
      }
    },
  ) { padding ->
    CompositionLocalProvider(
      LocalSubagentTranscript provides true,
      LocalChatHandlers provides handlers,
      // Read-only: a selection here must not quote into the parent chat's composer
      cc.hotic.acpira.ui.LocalQuote provides null,
      LocalAgentNames provides { a: String -> model.agent(a).name },
      cc.hotic.acpira.ui.LocalSessionSummaries provides model.summaries,
      // A diff's whole files come through the parent session, naming the child it was shown in
      cc.hotic.acpira.ui.LocalDiffSource provides diffSource,
    ) {
      InspectorThread(model, session, childView(root, node, transcript), node, loaded = transcript != null, padding)
    }
  }
}

/** The child as a session the transcript views can draw: the task as its opening prompt (unless the turns carry it), then its turns */
private fun childView(root: SessionView, node: SubagentSummary, transcript: SubagentTranscript?): SessionView {
  // A summoned round's transcript is its whole thread, each round led by its task: no second copy on top. A Claude run
  // resumed by SendMessage (a session node with a sidechain id) reads the same way
  val resumed = node.visibility == SubagentVisibility.session && node.peer.sidechainId != null
  val taskInTurns = if (node.harness != null || resumed) {
    transcript?.turns?.firstOrNull()?.let { it is UserTurn } ?: (node.harness?.let { it.round > 1 } ?: true)
  } else false
  val task = if (taskInTurns) emptyList() else node.task?.let { listOf(UserTurn(text = it)) }.orEmpty()
  val receipt = receiptOnly(node)
  return SessionView(
    id = "subagent:${node.id}", agent = node.harness?.agent ?: root.agent, title = subagentTitle(node), cwd = root.cwd,
    turns = task + (if (receipt) emptyList() else transcript?.turns.orEmpty()),
    running = !receipt && transcript?.running == true,
  )
}

/**
 * The child's thread: the main thread's views and its stick-to-bottom following, with what the inspector adds around
 * the turns: the loading row, a receipt node's note and result, a nested child's result, the note on a child whose
 * outcome never arrived
 */
@Composable
private fun InspectorThread(model: AppModel, session: String, view: SessionView, node: SubagentSummary, loaded: Boolean, padding: androidx.compose.foundation.layout.PaddingValues) {
  val receipt = receiptOnly(node)
  val empty = view.turns.none { it is AgentTurn }
  val loading = !receipt && (!loaded || (empty && node.state == SubagentState.running))
  // The result the parent received: all a receipt node has, a nested child's closing word, and what is left of a
  // settled child with no process of its own
  val result = if (receipt || node.visibility == SubagentVisibility.nested || (loaded && empty && node.state != SubagentState.running)) node.result else null
  val items = remember(view.turns) { exchanges(view.turns) }
  val extra = receipt || loading || result != null || node.state == SubagentState.disconnected
  val total = items.size + if (extra) 1 else 0
  val list = rememberLazyListState()
  var following by remember { mutableStateOf(true) }
  LaunchedEffect(list) {
    snapshotFlow { list.layoutInfo.let { info -> (info.visibleItemsInfo.lastOrNull()?.index ?: -1) >= info.totalItemsCount - 1 } }
      .collect { atEnd -> if (list.isScrollInProgress) following = atEnd }
  }
  // Opens at the bottom and sticks to it while the child streams; dragging away releases the follow
  LaunchedEffect(view.turns, total) {
    if (following && total > 0) list.scrollToItem(total - 1, Int.MAX_VALUE / 2)
  }
  BoxWithConstraints(Modifier.fillMaxSize()) {
    LazyColumn(
      state = list,
      modifier = Modifier.fillMaxSize().testTag("subagent.thread"),
      contentPadding = readablePadding(maxWidth, top = padding.calculateTopPadding() + Metrics.s2, bottom = padding.calculateBottomPadding() + Metrics.pad),
      verticalArrangement = Arrangement.spacedBy(Metrics.msgGap),
    ) {
      items.forEachIndexed { e, exchange ->
        item(key = "x:${exchange.id}") {
          Column(verticalArrangement = Arrangement.spacedBy(Metrics.msgGap)) {
            exchange.user?.let { UserBubble(it) }
            for ((index, turn) in exchange.agents) {
              val last = e == items.size - 1 && index == exchange.agents.last().first
              AgentMessage(
                turn = turn, running = view.running && last, key = "${view.id}:$index",
                now = { model.clockNow().toEpochMilli() }, onPermission = { b, o -> model.permission(session, b, o) },
                index = index,
              )
            }
          }
        }
      }
      if (extra) {
        item(key = "extra") {
          Column(
            Modifier.fillMaxWidth().padding(horizontal = Metrics.s1 + Metrics.pad),
            verticalArrangement = Arrangement.spacedBy(Metrics.msgGap),
          ) {
            CompositionLocalProvider(LocalRowHeight provides WebMetrics.chatRow) {
              if (receipt) {
                Text(t("subagents.receiptOnly"), color = Palette.fg3, style = WebType.text2)
              } else if (loading) {
                // Led by the Orb and labelled like a working turn's head, which takes its place once the turns arrive
                AcpRow(lead = { Orb() }, modifier = Modifier.testTag("subagent.loading")) { RowLabel(t("subagents.loading"), shimmer = true) }
              }
              result?.let { ResultSection(it) }
              if (node.state == SubagentState.disconnected) {
                AcpRow(icon = "unplug") { Text(t("subagents.disconnectedNote"), color = Palette.fg3, modifier = Modifier.weight(1f)) }
              }
            }
          }
        }
      }
    }
  }
}

/** The result the parent received, under a caption, as prose */
@Composable
private fun ResultSection(markdown: String) {
  Column(verticalArrangement = Arrangement.spacedBy(Metrics.s1)) {
    Text(t("subagents.result"), style = WebType.text3, color = Palette.fg3)
    StreamProse(TextBlock(markdown = markdown))
  }
}
