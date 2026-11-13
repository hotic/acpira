package cc.hotic.acpira.ui.subagents

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.PathMeasure
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.scale
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Constraints
import cc.hotic.acpira.design.LocalMotion
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.SubagentMetrics
import cc.hotic.acpira.design.SubagentMotion
import cc.hotic.acpira.design.VendorMark
import cc.hotic.acpira.design.WebMetrics
import cc.hotic.acpira.design.WebType
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.logic.ComposerControls
import cc.hotic.acpira.logic.SubagentReport
import cc.hotic.acpira.logic.breadcrumb
import cc.hotic.acpira.logic.commandDuration
import cc.hotic.acpira.logic.descendantCount
import cc.hotic.acpira.logic.elapsedText
import cc.hotic.acpira.logic.isSidekick
import cc.hotic.acpira.logic.isWaiting
import cc.hotic.acpira.logic.rootRows
import cc.hotic.acpira.logic.secondLine
import cc.hotic.acpira.logic.statusLabel
import cc.hotic.acpira.logic.subagentTitle
import cc.hotic.acpira.model.PermissionBlock
import cc.hotic.acpira.model.SessionOption
import cc.hotic.acpira.model.SubagentState
import cc.hotic.acpira.model.SubagentSummary
import cc.hotic.acpira.ui.AcpRow
import cc.hotic.acpira.ui.EntranceOnce
import cc.hotic.acpira.ui.Glyph
import cc.hotic.acpira.ui.LocalChatHandlers
import cc.hotic.acpira.ui.LocalClock
import cc.hotic.acpira.ui.LocalRowDenseHeight
import cc.hotic.acpira.ui.LocalRowHeight
import cc.hotic.acpira.ui.PermissionCard
import cc.hotic.acpira.ui.rememberFrameSeconds
import cc.hotic.acpira.ui.rowButton
import kotlinx.coroutines.delay
import kotlin.math.PI
import kotlin.math.abs
import kotlin.math.pow
import kotlin.math.sin

// Ported from apps/ios Transcript/SubagentViews.swift (chat/subagents/SubagentGroup.tsx and icons.tsx, and the
// DelegatedSubagents / ChildPermissions parts of Turns.tsx): the rows a turn shows for the children it delegated to (at
// the delegation point, or after the turn's process when no delegation row places them), their state marks, and a
// child's pending approval under its row. A row opens the child's inspector (ui/subagents/SubagentInspector.kt).

/** A tap on a subagent row; null where rows only draw */
val LocalInspectSubagent = compositionLocalOf<((String) -> Unit)?> { null }
/** The session's every node (their `linkOnly` copies), for descendant counts and provenance that cross turns */
val LocalAllSubagents = compositionLocalOf<List<SubagentSummary>> { emptyList() }
/** The nodes placed at this turn's delegation rows, by delegation (`placeNodes`) */
val LocalDelegations = compositionLocalOf<Map<String, List<SubagentSummary>>> { emptyMap() }
/**
 * Inside the subagent inspector: the child's transcript is read-only (no fork, retry or failure remedies), and the turn
 * offers no action row
 */
val LocalSubagentTranscript = compositionLocalOf { false }
/** An agent's display name by id (a summoned child's CLI) */
val LocalAgentNames = compositionLocalOf<(String) -> String> { { it } }

// Rows

/**
 * SubagentGroup: the turn's top-level children in announce order, finished ones included (a workflow's rows are its
 * progress; folding the done ones away made the list jump). No gap between rows: each carries its own padding
 */
@Composable
fun SubagentGroup(nodes: List<SubagentSummary>) {
  val rows = remember(nodes) { rootRows(nodes) }
  if (rows.isEmpty()) return
  val label = t("subagents.group", "n" to nodes.size)
  Column(Modifier.fillMaxWidth().semantics { contentDescription = label }) {
    for (node in rows) {
      key(node.id) {
        // The sidekick's round is a relay, never a subagent row; it runs its own entrances
        if (isSidekick(node)) SidekickRelay(node)
        else EntranceOnce("subagent:${node.id}") { SubagentRowButton(node) }
      }
    }
  }
}

/** The row as a tap target opening the inspector; a plain row where there is nothing to open */
@Composable
fun SubagentRowButton(node: SubagentSummary, descendants: Boolean = true, modifier: Modifier = Modifier) {
  val inspect = LocalInspectSubagent.current
  val tap = if (inspect != null) Modifier.rowButton { inspect(node.id) } else Modifier
  SubagentRow(node, descendants, modifier.then(tap).testTag("subagent.row:${node.id}"))
}

/**
 * An unframed row like the process folds: the state mark, the title with its faint meta, the status and elapsed time at
 * the end, then the live activity / waiting / result excerpt one step fainter. No chevron: the row opens a page. A
 * summoned child (another CLI) carries that CLI's mark on its state mark's corner, its model faintly with the reasoning
 * effort as a badge, and the round from the second one on
 */
@Composable
fun SubagentRow(node: SubagentSummary, descendants: Boolean = true, modifier: Modifier = Modifier) {
  val all = LocalAllSubagents.current
  val title = subagentTitle(node)
  val status = statusLabel(node)
  val line = secondLine(node)
  // The detail line repeats nothing the status already says
  val detail = line.takeIf { it != status }
  val below = if (descendants) descendantCount(node.id, all) else 0
  val spoken = listOfNotNull(title, node.role.takeIf { node.harness != null }, status, detail).joinToString(t("common.listSep"))
  AcpRow(
    modifier.padding(vertical = Metrics.gapHalf).semantics(mergeDescendants = true) { contentDescription = spoken },
    lead = { SubagentLead(node) },
  ) {
    Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(Metrics.gap)) {
      Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(Metrics.s2)) {
        TitleLine(title, metaParts(node, title, below), Modifier.weight(1f).alignByBaseline())
        Row(Modifier.alignByBaseline(), horizontalArrangement = Arrangement.spacedBy(Metrics.s2)) {
          Meta(status, statusInk(node), Modifier.alignByBaseline())
          SubagentElapsed(node, Modifier.alignByBaseline())
        }
      }
      detail?.let { Text(it, style = WebType.text3, color = Palette.fg3, maxLines = 1, overflow = TextOverflow.Ellipsis) }
    }
  }
}

/** One faint part of a row's title line; `rank`: the narrower the line, the fewer kept (rank 1 survives longest) */
private class MetaPart(val text: String, val effort: Boolean, val rank: Int)

/**
 * The title line's faint parts in order. A summoned round number tells a follow-up apart, so it stays; model and role
 * are the first to go
 */
private fun metaParts(node: SubagentSummary, title: String, descendants: Int): List<MetaPart> {
  val parts = ArrayList<MetaPart>()
  val h = node.harness
  if (h != null) {
    node.model?.let { parts += MetaPart(it, false, 3) }
    h.effort?.let { parts += MetaPart(ComposerControls.effortLabel(SessionOption(it, it)), true, 2) }
    if (h.round > 1) parts += MetaPart(t("subagents.round", "n" to h.round), false, 1)
  } else if (node.role != null && node.role != title) {
    parts += MetaPart(node.role!!, false, 3)
  }
  if (descendants > 0) parts += MetaPart(t("subagents.descendants", "n" to descendants), false, 2)
  return parts
}

/**
 * The title and its faint parts. At phone width a part cut to "t…" says nothing, so whole parts drop instead, the least
 * telling first, and the title truncates last (the web shrinks the faint parts four times faster than the title)
 */
@Composable
private fun TitleLine(title: String, parts: List<MetaPart>, modifier: Modifier = Modifier) {
  val gap = Metrics.s2
  Layout(
    content = {
      Text(title, color = Palette.fg1, maxLines = 1, overflow = TextOverflow.Ellipsis, softWrap = false)
      for (p in parts) {
        if (p.effort) EffortTag(p.text) else Meta(p.text)
      }
    },
    modifier = modifier,
  ) { measurables, constraints ->
    val gapPx = gap.roundToPx()
    val loose = Constraints(maxHeight = constraints.maxHeight)
    val partPlaceables = measurables.drop(1).map { it.measure(loose) }
    val titleFull = measurables[0].maxIntrinsicWidth(constraints.maxHeight)
    val ranks = parts.map { it.rank }.distinct().sortedDescending()
    // The widest set of parts that fits beside the whole title; else the title alone, truncating
    var keep = emptyList<Int>()
    for (rank in ranks) {
      val kept = parts.indices.filter { parts[it].rank <= rank }
      val width = titleFull + kept.sumOf { partPlaceables[it].width + gapPx }
      if (width <= constraints.maxWidth) { keep = kept; break }
    }
    val partsW = keep.sumOf { partPlaceables[it].width + gapPx }
    val titleP = measurables[0].measure(Constraints(maxWidth = (constraints.maxWidth - partsW).coerceAtLeast(0), maxHeight = constraints.maxHeight))
    val height = maxOf(titleP.height, keep.maxOfOrNull { partPlaceables[it].height } ?: 0)
    val baseline = titleP[androidx.compose.ui.layout.FirstBaseline]
    layout(constraints.maxWidth.takeIf { it != Constraints.Infinity } ?: (titleP.width + partsW), height, mapOf(androidx.compose.ui.layout.FirstBaseline to baseline)) {
      titleP.placeRelative(0, 0)
      var x = titleP.width
      for (i in keep) {
        val p = partPlaceables[i]
        x += gapPx
        val b = p[androidx.compose.ui.layout.FirstBaseline]
        val y = if (b != androidx.compose.ui.layout.AlignmentLine.Unspecified && baseline != androidx.compose.ui.layout.AlignmentLine.Unspecified) baseline - b
          else (titleP.height - p.height) / 2
        p.placeRelative(x, y)
        x += p.width
      }
    }
  }
}

/** A waiting child takes the warn tone; how it ended colours the rest */
@Composable
private fun statusInk(node: SubagentSummary): Color = when {
  isWaiting(node) -> Palette.warn
  node.state == SubagentState.completed -> Palette.ok
  node.state == SubagentState.failed -> Palette.danger
  else -> Palette.fg3
}

/** A faint text-3 part of the row's title line */
@Composable
private fun Meta(text: String, ink: Color = Palette.fg3, modifier: Modifier = Modifier) {
  Text(text, style = WebType.text3, color = ink, maxLines = 1, overflow = TextOverflow.Ellipsis, softWrap = false, modifier = modifier)
}

/** ChipTag small: the composer chip's level name on a quiet fill */
@Composable
fun EffortTag(label: String, modifier: Modifier = Modifier) {
  Box(
    modifier.height(SubagentMetrics.chipTagSmall).background(Palette.active, RoundedCornerShape(Metrics.rXs)).padding(horizontal = Metrics.s1),
    contentAlignment = Alignment.Center,
  ) {
    // Its own line box: the row's inherited line height would overflow the tag and clip the glyphs
    Text(
      label, color = Palette.fg3, maxLines = 1, softWrap = false,
      style = androidx.compose.ui.text.TextStyle(
        fontSize = SubagentMetrics.tagText, lineHeight = SubagentMetrics.tagText,
        lineHeightStyle = androidx.compose.ui.text.style.LineHeightStyle(
          androidx.compose.ui.text.style.LineHeightStyle.Alignment.Center, androidx.compose.ui.text.style.LineHeightStyle.Trim.Both,
        ),
      ),
    )
  }
}

/** useElapsed: a running child ticks on the app's clock; a finished one reads its engine timestamps */
@Composable
fun SubagentElapsed(node: SubagentSummary, modifier: Modifier = Modifier) {
  // Read through the latest clock: the loop outlives a model swap (Rows.kt LiveElapsed)
  val clock by rememberUpdatedState(LocalClock.current)
  val live = node.state == SubagentState.running && node.endedAt == null
  var now by remember { mutableLongStateOf(clock()) }
  if (live) LaunchedEffect(node.id) {
    while (true) {
      now = clock()
      delay(CLOCK_TICK_MS)
    }
  }
  val text = if (live) commandDuration(now - node.announcedAt, whole = true) else elapsedText(node, clock().toDouble())
  Text(
    text, style = WebType.text3.copy(fontFeatureSettings = "tnum"), color = Palette.fg3, maxLines = 1, softWrap = false,
    modifier = modifier,
  )
}

private const val CLOCK_TICK_MS = 1000L

// Marks

/**
 * HistoryMessage.tsx SubagentsReported: each summoned agent's return in the relay handback's shape, at the head of the
 * reply it woke. The message itself is for the agent; a wake with only the sidekick shows nothing
 */
@Composable
fun SubagentReturns(reports: List<SubagentReport>, modifier: Modifier = Modifier) {
  CompositionLocalProvider(LocalRowHeight provides WebMetrics.processRow, LocalRowDenseHeight provides WebMetrics.processRow) {
    Column(modifier.fillMaxWidth().padding(horizontal = Metrics.pad + Metrics.threadInset), verticalArrangement = Arrangement.spacedBy(WebMetrics.processRowGap)) {
      for (r in reports) {
        AcpRow(lead = { VendorMark(r.agent, r.name, Palette.fg1, size = WebMetrics.icon) }) {
          Text(r.name, color = Palette.fg1, maxLines = 1, overflow = TextOverflow.Ellipsis, softWrap = false, modifier = Modifier.alignByBaseline())
          Text(t(if (r.asks) "chat.subagentAsks" else "sidekick.handback"), color = Palette.fg2, maxLines = 1, softWrap = false, modifier = Modifier.alignByBaseline())
          r.line?.let { SqueezedTitle(it) }
        }
      }
    }
  }
}

/** The task in a handoff or return row, the first to give up its width (subagents.css `flex-shrink: 1000`) */
@Composable
fun RowScope.SqueezedTitle(title: String) {
  Text(
    title, color = Palette.fg3, maxLines = 1, overflow = TextOverflow.Ellipsis, softWrap = false,
    modifier = Modifier.weight(1f, fill = false).alignByBaseline(),
  )
}

/** The lead slot: the state mark at --subagent-mark, and a summoned child's CLI mark on its corner over a page tile */
@Composable
fun SubagentLead(node: SubagentSummary) {
  val names = LocalAgentNames.current
  Box(Modifier.size(SubagentMetrics.mark)) {
    SubagentStateIcon(node, Modifier.align(Alignment.Center))
    node.harness?.agent?.takeIf { it.isNotEmpty() }?.let { agent ->
      Box(
        Modifier.align(Alignment.BottomEnd)
          .offset(x = -SubagentMetrics.badgeInset, y = -SubagentMetrics.badgeInset)
          .background(Palette.bg0, RoundedCornerShape(Metrics.rSm))
          .padding(Metrics.hairline),
      ) {
        VendorMark(agent, names(agent), Palette.fg1, size = SubagentMetrics.badge - Metrics.hairline * 2)
      }
    }
  }
}

/**
 * icons.tsx stateIcon: a pending card outranks the state, as that is what the child is blocked on. Running and completed
 * share one view (SubagentMark), so a child finishing in front of the reader plays the bars → ✓ finish
 */
@Composable
fun SubagentStateIcon(node: SubagentSummary, modifier: Modifier = Modifier) {
  val ink = LocalContentColor.current
  Box(modifier, contentAlignment = Alignment.Center) {
    when {
      !node.permissions.isNullOrEmpty() -> Glyph("hand", Palette.warn)
      node.question != null -> Glyph("messageCircleQuestion", Palette.warn)
      node.state == SubagentState.running || node.state == SubagentState.completed -> SubagentMark(node.state == SubagentState.completed)
      node.state == SubagentState.failed -> Glyph("triangleAlert", Palette.danger)
      node.state == SubagentState.cancelled -> Glyph("x", ink)
      else -> Glyph("unplug", ink)
    }
  }
}

/**
 * Three dotted bars bob while the child works: each bar is five fixed beads, and it "grows" by fading its outer beads in
 * (subagents.css). When a live child completes, each bar pulls its beads into its centre, the three centres land on the
 * ✓'s vertices and the stroke draws through them. A node first seen completed is a plain ✓; with motion off the bars
 * stand still, every bead shown
 */
@Composable
fun SubagentMark(done: Boolean) {
  val motion = LocalMotion.current
  val ink = LocalContentColor.current
  val fg = Palette.fg1
  // When the child completed in front of the reader (frame seconds); null: no finish to play
  var finishedAt by remember { mutableStateOf<Double?>(null) }
  var was by remember { mutableStateOf(done) }
  if (was != done) {
    if (!was && done && motion) finishedAt = System.nanoTime() / NANOS
    was = done
  }
  LaunchedEffect(finishedAt) {
    if (finishedAt == null) return@LaunchedEffect
    delay((SubagentMotion.finish * 1000).toLong())
    finishedAt = null
  }
  val started = finishedAt
  if (done && started == null) {
    // The settled ✓ at --icon in the slot, in the row's quiet ink like every other Check
    Glyph("check", ink)
    return
  }
  val time = rememberFrameSeconds(paused = !motion)
  Canvas(Modifier.size(SubagentMetrics.mark)) {
    val now = time.value
    when {
      started != null -> MarkPainter.finish(this, now - started, started, fg, ink)
      motion -> MarkPainter.bars(this, now, fg)
      else -> MarkPainter.bars(this, null, fg)
    }
  }
}

private const val NANOS = 1e9

/** icons.tsx's geometry on lucide's 24 grid and the finish's timeline (playFinish); the ✓ is lucide Check's own path */
private object MarkPainter {
  private val spread = SubagentMetrics.spread
  private val barX = listOf(12 - 5 * spread, 12f, 12 + 5 * spread)
  /** Ring 0 is the centre bead; rings 1 and 2 step out by the pitch */
  private val pitch = 3 * spread
  private val beads = listOf(0, -1, 1, -2, 2)
  private val check = listOf(4f to 12f, 9f to 17f, 20f to 6f)

  /** A bar's half-height h(t) = 3.5 + 2.5·sin(2πt) in thirds of the pitch, each outer ring fading over one pitch */
  fun opacity(ring: Int, phase: Double): Float {
    if (ring <= 0) return 1f
    val h = (3.5 + 2.5 * sin(2 * PI * phase)) / 3
    return (h - ring + 0.5).coerceIn(0.0, 1.0).toFloat()
  }

  /** Bar i's place in the loop at [time]: each bar runs a third of a loop ahead of the one before it */
  fun phase(i: Int, time: Double): Double = ((time + i * SubagentMotion.barStagger) / SubagentMotion.loop).mod(1.0)

  /** The running bars at [time]; null stands them still with every bead shown */
  fun bars(scope: DrawScope, time: Double?, ink: Color) = with(scope) {
    val s = size.width / 24f
    barX.forEachIndexed { i, x ->
      for (k in beads) {
        val o = time?.let { opacity(abs(k), phase(i, it)) } ?: 1f
        val r = SubagentMetrics.beadR * (if (k == 0) 1f else 0.4f + 0.6f * o) * s
        drawCircle(ink.copy(alpha = ink.alpha * o), r, Offset(x * s, (12 + k * pitch) * s))
      }
    }
  }

  /** The finish, [e] seconds in; the loop is frozen where it stood at [startedAt] */
  fun finish(scope: DrawScope, e: Double, startedAt: Double, ink: Color, rest: Color) = with(scope) {
    val s = size.width / 24f
    val scale = SubagentMetrics.checkScale
    val vertex = check.map { (12 + (it.first - 12) * scale) to (12 + (it.second - 12) * scale) }
    val m = SubagentMotion
    val settle = m.gather + 0.22
    // A small pop while the ink settles from the running colour into the row's resting one
    val popped = clamp((e - settle) / m.pop)
    val grow = (1 + 0.18 * sin(PI * popped)).toFloat()
    scale(grow, grow) {
      val barsAlpha = 1 - clamp((e - (m.gather + 0.2)) / m.fade)
      val pulled = easeOut(clamp(e / (m.gather * 0.8)))
      barX.forEachIndexed { i, x ->
        val moved = easeOut(clamp((e - i * m.lag) / m.gather))
        val cx = x + (vertex[i].first - x) * moved
        val cy = 12 + (vertex[i].second - 12) * moved
        for (k in beads) {
          val start = opacity(abs(k), phase(i, startedAt))
          // Outer beads slide into the centre and vanish; the centre bead is the point that lands on the vertex
          val o = ((if (k == 0) 1f else start * (1 - pulled)) * barsAlpha).coerceIn(0f, 1f)
          val r = SubagentMetrics.beadR * (if (k == 0) 1f else (0.4f + 0.6f * start) * (1 - pulled) + 0.4f * pulled) * s
          drawCircle(ink.copy(alpha = ink.alpha * o), r, Offset(cx * s, (cy + k * pitch * (1 - pulled)) * s))
        }
      }
      val path = Path().apply {
        moveTo(vertex[0].first * s, vertex[0].second * s)
        for (v in vertex.drop(1)) lineTo(v.first * s, v.second * s)
      }
      val drawn = easeInOut(clamp((e - (m.gather - 0.06)) / m.draw))
      val measure = PathMeasure().apply { setPath(path, false) }
      val line = Path()
      measure.getSegment(0f, measure.length * drawn, line, true)
      val stroke = Stroke(width = 1.5f * s * scale, cap = StrokeCap.Round, join = StrokeJoin.Round)
      val settled = clamp((e - settle) / m.ink)
      drawPath(line, ink.copy(alpha = ink.alpha * (1 - settled)), style = stroke)
      drawPath(line, rest.copy(alpha = rest.alpha * settled), style = stroke)
    }
  }

  fun clamp(x: Double): Float = x.coerceIn(0.0, 1.0).toFloat()
  /** cubic-bezier(.3, .7, .2, 1), near enough */
  fun easeOut(x: Float): Float = 1 - (1 - x).pow(3)
  /** cubic-bezier(.5, 0, .2, 1), near enough */
  fun easeInOut(x: Float): Float = if (x < 0.5f) 4 * x * x * x else 1 - (-2 * x + 2).pow(3) / 2
}

// Placement in the turn

/**
 * DelegatedSubagents: the rows of one delegation point (a parallel launch is one point), read from the turn's placed
 * nodes, with any pending child approval right under them
 */
@Composable
fun DelegatedSubagents(ids: List<String>, modifier: Modifier = Modifier) {
  val delegations = LocalDelegations.current
  val nodes = ids.flatMap { delegations[it].orEmpty() }
  if (nodes.isEmpty()) return
  Column(modifier, verticalArrangement = Arrangement.spacedBy(Metrics.gap)) {
    SubagentGroup(nodes)
    ChildPermissions(nodes)
  }
}

/**
 * ChildPermissions: a child's pending approval under the rows. A summoned child's sits at the label column with nothing
 * above it, since the row already names who asks; another child's gets a provenance line naming the path to it. The
 * card answers through the root session, which routes it to the child
 */
@Composable
fun ChildPermissions(nodes: List<SubagentSummary>) {
  val all = LocalAllSubagents.current
  val handlers = LocalChatHandlers.current
  val cards: List<Pair<SubagentSummary, PermissionBlock>> = nodes.flatMap { n -> n.permissions.orEmpty().map { n to it } }
  for ((node, block) in cards) {
    key(block.id) {
      val pick: (String) -> Unit = { handlers?.permission?.invoke(block.id, it) }
      if (node.harness != null && block.planId == null) {
        Box(Modifier.padding(start = WebMetrics.indent)) { PermissionCard(block, pick) }
      } else {
        val path = breadcrumb(node.id, all.ifEmpty { nodes }).joinToString(" › ") { subagentTitle(it) }
        Column(verticalArrangement = Arrangement.spacedBy(Metrics.s1)) {
          AcpRow(dense = true) {
            Text(t("subagents.provenance", "path" to path), style = WebType.text3, color = Palette.fg3, modifier = Modifier.alignByBaseline())
          }
          PermissionCard(block, pick)
        }
      }
    }
  }
}
