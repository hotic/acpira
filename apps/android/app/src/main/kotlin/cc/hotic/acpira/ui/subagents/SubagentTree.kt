package cc.hotic.acpira.ui.subagents

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.style.TextOverflow
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.SubagentMetrics
import cc.hotic.acpira.design.WebMetrics
import cc.hotic.acpira.design.WebType
import cc.hotic.acpira.design.readableWidth
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.logic.flattenTree
import cc.hotic.acpira.logic.isSidekick
import cc.hotic.acpira.logic.linkOnly
import cc.hotic.acpira.store.AppModel
import cc.hotic.acpira.store.Screen
import cc.hotic.acpira.ui.AcpRow
import cc.hotic.acpira.ui.LocalClock
import cc.hotic.acpira.ui.LocalRowHeight
import cc.hotic.acpira.ui.ToolPageBar

// Ported from apps/ios Subagents/SubagentTree.swift (chat/subagents/SubagentGraph.tsx at phone width), a page the chat's
// "…" menu pushes: the session's every child as the transcript's rows, one lead + gap deeper per level under the
// session's own label row. It is the only way to a grandchild, since a turn shows its top-level children alone. The
// web's elbow lines are left out: the indent carries the tree at this width.

@Composable
fun SubagentTreePage(model: AppModel, session: String) {
  val root = model.views[session]
  // Summoned helpers only: sidekick rounds live in the transcript alone, as on the web
  val nodes = remember(root?.subagents) { root?.subagents.orEmpty().filter { !isSidekick(it) }.map { it.linkOnly } }
  val rows = remember(nodes) { flattenTree(nodes) }
  Scaffold(containerColor = Palette.bg0, topBar = { ToolPageBar(t("subagents.graph"), "subagents", model::pop) }) { padding ->
    CompositionLocalProvider(
      LocalRowHeight provides WebMetrics.chatRow,
      LocalAllSubagents provides nodes,
      LocalInspectSubagent provides { id: String -> model.push(Screen.Subagent(session, id)) },
      LocalAgentNames provides { a: String -> model.agent(a).name },
      LocalClock provides { model.clockNow().toEpochMilli() },
    ) {
      Column(
        Modifier.fillMaxSize().padding(padding).verticalScroll(rememberScrollState()).navigationBarsPadding()
          .readableWidth().padding(horizontal = Metrics.s1 + Metrics.pad, vertical = SubagentMetrics.treePadY).testTag("subagents.tree"),
      ) {
        // The session labels the tree; it is not a node to open
        AcpRow(icon = "gitBranch", trailing = { Text(t("subagents.entry", "n" to nodes.size), style = WebType.text3, color = Palette.fg3) }) {
          Text(root?.title ?: t("subagents.root"), color = Palette.fg1, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
        for ((node, depth) in rows) {
          SubagentRowButton(node, descendants = false, modifier = Modifier.padding(start = WebMetrics.indent * (depth + 1)))
        }
      }
    }
  }
}
