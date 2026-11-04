package cc.hotic.acpira.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableDoubleStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.BlendMode
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.drawscope.scale
import androidx.compose.ui.graphics.drawscope.translate
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.layout.layout
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.style.TextOverflow
import cc.hotic.acpira.design.IconStore
import cc.hotic.acpira.design.IconStroke
import cc.hotic.acpira.design.LocalMotion
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.RowMotion
import cc.hotic.acpira.design.WebMetrics
import cc.hotic.acpira.design.WebType
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.logic.ProcessItem
import cc.hotic.acpira.model.AgentBlock
import cc.hotic.acpira.model.AgentTurn
import cc.hotic.acpira.model.CompactionBlock
import cc.hotic.acpira.model.CompactionStatus
import cc.hotic.acpira.model.GoalBlock
import cc.hotic.acpira.model.NoticeBlock
import cc.hotic.acpira.model.PlanBlock
import cc.hotic.acpira.model.PlanEntry
import cc.hotic.acpira.model.PlanStatus
import cc.hotic.acpira.model.Question
import cc.hotic.acpira.model.QuestionBlock
import cc.hotic.acpira.model.QuestionOutcome
import cc.hotic.acpira.model.SteerBlock
import cc.hotic.acpira.model.TextBlock
import cc.hotic.acpira.model.ThoughtBlock
import cc.hotic.acpira.model.ToolCallBlock
import cc.hotic.acpira.model.TurnStop
import cc.hotic.acpira.model.UnknownBlock
import cc.hotic.acpira.model.UserTurn
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonPrimitive

// chat/Thought.tsx, Plan.tsx, ProcessHistory.tsx and the Compaction / NoticeRow / Outcome rows of Turns.tsx, after the
// iOS app's Transcript/ProcessViews.swift.

/** An item's entrance identity, shared by every view that may draw it */
fun itemEntrance(id: String) = "item:$id"

/**
 * `process-rows`: rows and dense rows shrink to --process-row, items sit --process-row-gap apart, and prose inside the
 * process keeps --process-prose-gap from its neighbours. Each item enters once under its own id
 */
@Composable
fun ProcessHistory(items: List<ProcessItem>, modifier: Modifier = Modifier) {
  CompositionLocalProvider(LocalRowHeight provides WebMetrics.processRow, LocalRowDenseHeight provides WebMetrics.processRow) {
    Column(modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(WebMetrics.processRowGap)) {
      items.forEachIndexed { i, item ->
        key(item.id) {
          val prose = isProse(item)
          val afterProse = i > 0 && isProse(items[i - 1])
          Box(Modifier.padding(top = if ((prose && i > 0) || afterProse) Metrics.processProseGap - WebMetrics.processRowGap else Metrics.s0)) {
            EntranceOnce(itemEntrance(item.id)) { ProcessItemView(item) }
          }
        }
      }
    }
  }
}

private fun isProse(item: ProcessItem) = item is ProcessItem.Block && (item.block is TextBlock || item.block is SteerBlock)

/** One process item: a read / edit group, a single call, prose, or a line block */
@Composable
fun ProcessItemView(item: ProcessItem) {
  when (item) {
    is ProcessItem.Group -> ToolGroupView(item.kind, item.blocks)
    is ProcessItem.Block -> when (val b = item.block) {
      is ToolCallBlock -> ToolCallView(b, grouped = true)
      is TextBlock -> if (b.markdown.isNotBlank()) Prose(b.markdown.trimEnd())
      else -> LineBlockView(b)
    }
  }
}

@Composable
fun LineBlockView(block: AgentBlock) {
  when (block) {
    is NoticeBlock -> NoticeRow(block)
    is GoalBlock -> GoalRow(block)
    is ThoughtBlock -> ThoughtView(block)
    is PlanBlock -> PlanDetails(block.entries, label = { RowLabel(t("plan.title")) })
    is ToolCallBlock -> ToolCallView(block)
    is CompactionBlock -> CompactionRow(block)
    is SteerBlock -> SteeredMessage(block)
    // The open card waits for an answer elsewhere; only a resolved one has a place in the message
    is QuestionBlock -> if (block.outcome != null) QuestionRecord(block)
    is UnknownBlock -> UnknownBlockRow(block.type)
    else -> {}
  }
}

/** A prompt sent while the turn ran: the user's bubble, at the thread's own width (it stays in view as the fold closes) */
@Composable
fun SteeredMessage(block: SteerBlock) {
  Box(
    Modifier.layout { measurable, constraints ->
      val out = (Metrics.pad + Metrics.threadInset).roundToPx()
      val placeable = measurable.measure(constraints.copy(maxWidth = constraints.maxWidth + out * 2, minWidth = constraints.minWidth + out * 2))
      layout(constraints.maxWidth, placeable.height) { placeable.place(-out, 0) }
    },
  ) { UserBubble(UserTurn(text = block.text, attachments = block.attachments)) }
}

// Question record, unknown blocks

/**
 * Questions.tsx QuestionRecord: an answered or skipped question keeps one row in the message, the answers folded under
 * it (open when answered)
 */
@Composable
fun QuestionRecord(block: QuestionBlock) {
  val answers = block.answers.orEmpty()
  val label = when (block.outcome) {
    QuestionOutcome.answered -> if (answers.size == 1) t("question.answeredOne") else t("question.answered", "n" to answers.size)
    QuestionOutcome.skipped -> t("question.skipped")
    else -> t("question.cancelled")
  }
  val scope = LocalEntranceScope.current
  val fold = rememberFold("${scope.scope}:question:${block.id}", block.outcome == QuestionOutcome.answered)
  Disclosure(fold.open, fold::toggle, icon = "messageCircleQuestion", label = { RowLabel(label) }) {
    Column(verticalArrangement = Arrangement.spacedBy(Metrics.s1_5)) {
      for (q in block.questions) {
        val names = answerNames(q, answers[q.id])
        Column {
          Text(q.text, color = Palette.fg3, style = WebType.text2)
          Text(
            if (names.isEmpty()) t("question.noAnswer") else names.joinToString(t("common.listSep")),
            color = if (names.isEmpty()) Palette.fg3 else Palette.fg1, style = WebType.text2,
          )
        }
      }
    }
  }
}

/** An answer's option labels (free text as written) */
private fun answerNames(q: Question, answer: JsonElement?): List<String> {
  val values = when (answer) {
    is JsonPrimitive -> listOf(answer.content)
    is JsonArray -> answer.mapNotNull { (it as? JsonPrimitive)?.content }
    else -> emptyList()
  }
  return values.map { v -> q.options.firstOrNull { it.id == v }?.label ?: v }
}

/** A block type this build does not know (an engine ahead of the app): a quiet row naming it */
@Composable
fun UnknownBlockRow(type: String) {
  AcpRow(icon = "puzzle") {
    RowLabel(t("mobile.unknownBlock"))
    RowTarget(type, mono = true, ink = Palette.fg3)
  }
}

// Thought

/**
 * A thought: one action row (Thinking while it streams), its text in thought-tone prose inside a bounded scrollport
 * that follows the stream until the reader drags it, and fades an edge only while more text lies beyond it
 */
@Composable
fun ThoughtView(block: ThoughtBlock) {
  val live = block.streaming == true
  val row = LocalRowHeight.current
  // Thoughts carry no id; the start stamp names them within the turn
  val id = "thought:${block.startedAt?.toLong() ?: ""}"
  val scope = LocalEntranceScope.current
  val fold = rememberFold("${scope.scope}:$id", false)
  EntranceOnce(id) {
    Disclosure(fold.open, fold::toggle, icon = "brain", label = { RowLabel(if (live) t("host.thinking") else t("thought.label"), shimmer = live) }) {
      val scroll = rememberScrollState()
      var follow by remember { mutableStateOf(true) }
      val edge = with(LocalDensity.current) { Metrics.s2.toPx() }
      val fade = with(LocalDensity.current) { WebMetrics.scrollFade.toPx() }
      val hair = with(LocalDensity.current) { Metrics.hairline.toPx() }
      // A drag lets go of the stream; settling back at the end takes it again
      LaunchedEffect(scroll) {
        snapshotFlow { scroll.isScrollInProgress }.collect { moving ->
          if (moving) follow = false else follow = scroll.maxValue - scroll.value <= edge
        }
      }
      LaunchedEffect(scroll, live) {
        snapshotFlow { scroll.maxValue }.collect { max ->
          if (live && follow && !scroll.isScrollInProgress) scroll.scrollTo(max)
        }
      }
      Box(
        Modifier.heightIn(max = row * WebMetrics.thoughtRows)
          .graphicsLayer(compositingStrategy = CompositingStrategy.Offscreen)
          .drawWithContent {
            drawContent()
            if (scroll.value > hair) {
              drawRect(Brush.verticalGradient(listOf(Color.Black, Color.Transparent), endY = fade), size = Size(size.width, fade), blendMode = BlendMode.DstOut)
            }
            if (scroll.maxValue - scroll.value > hair) {
              drawRect(
                Brush.verticalGradient(listOf(Color.Transparent, Color.Black), startY = size.height - fade, endY = size.height),
                topLeft = Offset(0f, size.height - fade), size = Size(size.width, fade), blendMode = BlendMode.DstOut,
              )
            }
          }
          .verticalScroll(scroll),
      ) {
        Prose(block.text.trimEnd(), color = Palette.fg2)
      }
    }
  }
}

// Plan

/** Historical plans and to-do tool results: one disclosure row with the done count and a caret, the entries below */
@Composable
fun PlanDetails(
  entries: List<PlanEntry>,
  foldKey: String = "",
  label: @Composable androidx.compose.foundation.layout.RowScope.() -> Unit,
  trailing: (@Composable androidx.compose.foundation.layout.RowScope.() -> Unit)? = null,
) {
  val done = entries.count { it.status == PlanStatus.completed }
  val scope = LocalEntranceScope.current
  val fold = rememberFold("${scope.scope}:plan:$foldKey:${entries.size}:${entries.firstOrNull()?.title}", false)
  // Entry status dots are not stops of this rail: it runs from the list icon down the body
  Disclosure(
    fold.open, fold::toggle, icon = "listTodo",
    label = {
      label()
      Text("$done/${entries.size}", color = Palette.fg3, style = WebType.text3.copy(fontFeatureSettings = TABULAR), maxLines = 1, softWrap = false, modifier = Modifier.alignByBaseline())
      FoldCaret(fold.open, Palette.actionFg, markOnBaseline())
    },
    trailing = trailing,
  ) {
    PlanEntries(entries)
  }
}

/** The entry list: a status dot in the lead slot, the title dimming once done, a faint priority glyph for high / low */
@Composable
fun PlanEntries(entries: List<PlanEntry>) {
  Column {
    for (entry in entries) {
      val ink = when (entry.status) {
        PlanStatus.completed -> Palette.fg2
        PlanStatus.in_progress -> Palette.fgStrong
        PlanStatus.pending -> Palette.fg1
      }
      AcpRow(
        dense = true, leadTop = true,
        lead = { PlanDot(entry.status) },
        trailing = when (entry.priority) {
          "high" -> ({ Glyph("chevronsUp", Palette.fg3, WebMetrics.chevron, IconStroke.chevron) })
          "low" -> ({ Glyph("chevronsDown", Palette.fg3, WebMetrics.chevron, IconStroke.chevron) })
          else -> null
        },
      ) {
        Text(entry.title, color = ink)
      }
    }
  }
}

/** Plan entry status: a filled check, a ring around an accent dot, an empty ring */
@Composable
fun PlanDot(status: PlanStatus) {
  val size = WebMetrics.chevron
  when (status) {
    PlanStatus.completed -> Box(Modifier.size(size).background(Palette.fg3, CircleShape), contentAlignment = Alignment.Center) {
      Glyph("check", Palette.bg0, WebMetrics.planCheck, IconStroke.planCheck)
    }
    PlanStatus.in_progress -> Box(Modifier.size(size).border(WebMetrics.planRing, Palette.brand, CircleShape), contentAlignment = Alignment.Center) {
      Box(Modifier.size(WebMetrics.planDot).background(Palette.brand, CircleShape))
    }
    PlanStatus.pending -> Box(Modifier.size(size).border(WebMetrics.planRing, Palette.fg3, CircleShape))
  }
}

// Compaction

/** Compactions seen running here: the row can be composed again between its running and its completed state */
private val compactionSeen = HashSet<String>()

private fun nowSeconds() = System.nanoTime() / 1e9

/**
 * Context compaction as an action row (Turns.tsx Compaction): the live status shimmers while the icon's four arrows
 * keep pushing inward. A compaction watched to its end lets the loop reach its rest beat, then clamps the arrows shut
 * once and lands the new label bright before it eases to the action ink
 */
@Composable
fun CompactionRow(block: CompactionBlock) {
  val motion = LocalMotion.current
  var loopStart by remember { mutableDoubleStateOf(nowSeconds()) }
  var settleStart by remember { mutableStateOf<Double?>(null) }
  var finishing by remember { mutableStateOf(false) }
  var first by remember { mutableStateOf(true) }
  LaunchedEffect(block.status) {
    val rebuilt = first
    first = false
    if (block.status == CompactionStatus.in_progress) {
      compactionSeen += block.id
      loopStart = nowSeconds()
      settleStart = null
      finishing = false
      return@LaunchedEffect
    }
    if (!compactionSeen.remove(block.id) || block.status != CompactionStatus.completed || !motion) return@LaunchedEffect
    // A row composed again between the two states has no loop left to wait for and settles at once
    val at = (nowSeconds() - loopStart) % RowMotion.compactCycle
    val wait = if (rebuilt || at >= RowMotion.compactRest) 0.0 else RowMotion.compactRest - at
    if (wait > 0) {
      finishing = true
      kotlinx.coroutines.delay((wait * MS).toLong())
      finishing = false
    }
    settleStart = nowSeconds()
    kotlinx.coroutines.delay((RowMotion.compactSettle * MS).toLong())
    settleStart = null
  }
  // A completion still waiting for the arrows' rest beat keeps the running look, so label and icon change together
  val running = block.status == CompactionStatus.in_progress || finishing
  val failed = block.status == CompactionStatus.failed
  val label = t(
    when {
      running -> "turns.compacting"
      block.status == CompactionStatus.completed -> "turns.compacted"
      failed -> "turns.compactFailed"
      else -> "turns.compactCancelled"
    },
  )
  val settling = settleStart
  AcpRow(
    tone = RowTone.action,
    lead = { CompactionIcon(failed, if (running && motion) loopStart else null, settling) },
  ) {
    if (settling != null) {
      val time = rememberFrameSeconds()
      val strong = Palette.fgStrong
      val ink = Palette.actionFg
      val p = ((time.value - settling) / RowMotion.compactSettle).toFloat().coerceIn(0f, 1f)
      Text(label, color = lerp(strong, ink, EASE_OUT.transform(p)), maxLines = 1, softWrap = false, modifier = Modifier.alignByBaseline())
    } else {
      RowLabel(label, shimmer = running)
    }
    if (failed) block.error?.let { RowTarget(it, ink = Palette.fg3) }
  }
}

private const val MS = 1000
private val EASE_OUT = androidx.compose.animation.core.CubicBezierEasing(0f, 0f, 0.58f, 1f)

/** Each arrow's way out, in icon units: the Shrink paths in order */
private val compactWays = listOf(Offset(3f, 3f), Offset(-3f, 3f), Offset(3f, -3f), Offset(-3f, -3f))

/** Arrow [i] at [t] seconds into the loop: a share of its way out and its opacity (0%: out at 0.3, 60–100%: home) */
private fun compactLoop(i: Int, t: Double): Pair<Float, Float> {
  val local = t - i * RowMotion.compactStagger
  if (local < 0) return 0f to 1f
  val p = (local % RowMotion.compactCycle) / RowMotion.compactCycle
  if (p >= LOOP_OUT) return 0f to 1f
  val e = RowMotion.compactLoop.transform((p / LOOP_OUT).toFloat())
  return (1 - e) to (LOOP_DIM + (1 - LOOP_DIM) * e)
}

/** All arrows at [t] seconds into the clamp: home, half their way in at 40%, a tenth out at 70%, home */
private fun compactSnap(t: Double): Float {
  val p = (t / RowMotion.compactSnap).coerceIn(0.0, 1.0).toFloat()
  for (k in 1 until snapStops.size) {
    val (p0, v0) = snapStops[k - 1]
    val (p1, v1) = snapStops[k]
    if (p <= p1) return v0 + (v1 - v0) * RowMotion.compactSnapCurve.transform((p - p0) / (p1 - p0))
  }
  return 0f
}

private const val LOOP_OUT = 0.6
private const val LOOP_DIM = 0.3f
private val snapStops = listOf(0f to 0f, 0.4f to -0.5f, 0.7f to 0.1f, 1f to 0f)

/** The compaction lead: the Shrink icon, its four arrows moved one by one while it loops or clamps */
@Composable
private fun CompactionIcon(failed: Boolean, loop: Double?, settle: Double?) {
  val ink = Palette.actionFg
  if (failed || (loop == null && settle == null)) {
    Glyph(if (failed) "triangleAlert" else "shrink", ink)
    return
  }
  val context = LocalContext.current
  val paths = remember { IconStore.lucidePaths(context, "shrink").orEmpty() }
  val time = rememberFrameSeconds()
  Canvas(Modifier.size(WebMetrics.icon)) {
    val k = size.minDimension / ICON_UNITS
    val now = time.value
    scale(k, k, pivot = Offset.Zero) {
      paths.forEachIndexed { i, p ->
        val (out, alpha) = if (loop != null) compactLoop(i, now - loop) else compactSnap(now - (settle ?: now)) to 1f
        val way = compactWays[i % compactWays.size]
        translate(way.x * out, way.y * out) {
          drawPath(p, ink.copy(alpha = ink.alpha * alpha), style = Stroke(IconStroke.icon, cap = StrokeCap.Round, join = StrokeJoin.Round))
        }
      }
    }
  }
}

private const val ICON_UNITS = 24f

// Notices, outcome, goals

/** Workspace hook notices (`acp/session/hooks.rs`) carry this id prefix and the gate's shield glyph */
private const val HOOK_NOTICE = "hooks:"

private fun noticeGlyph(block: NoticeBlock, error: Boolean) = when {
  block.id.startsWith(HOOK_NOTICE) -> if (error) "shieldX" else "shieldAlert"
  block.info == true -> "info"
  else -> "triangleAlert"
}

/**
 * One row per failure (Turns.tsx NoticeRow). A notice with details folds like a thought, the title alone on the row and
 * the details on the rail once open; an error opens by default and marks only its glyph with the warn tone. An info
 * notice only informs, without the triangle
 */
@Composable
fun NoticeRow(block: NoticeBlock) {
  block.retry?.let { RetryRow(block.id, it); return }
  val error = block.severity == "error"
  val tint = if (error) Palette.warn else Palette.fg3
  val details = block.details?.takeIf { it.isNotBlank() }
  if (details != null) {
    val scope = LocalEntranceScope.current
    val fold = rememberFold("${scope.scope}:notice:${block.id}", error)
    Disclosure(fold.open, fold::toggle, icon = noticeGlyph(block, error), iconTint = tint, label = { RowLabel(block.title) }) {
      NoticeDetails(details)
    }
  } else {
    AcpRow(icon = noticeGlyph(block, error), iconTint = tint) {
      Text(block.title, color = if (error) Palette.fg2 else Palette.fg3)
    }
  }
}

/** Details under an open notice: `- path` lines read as file rows, the rest verbatim */
@Composable
private fun NoticeDetails(text: String) {
  Column {
    for (line in text.trimEnd().split('\n')) {
      val path = NOTICE_PATH.matchEntire(line)?.groupValues?.get(1)?.takeIf { '/' in it || '.' in it }
      if (path != null) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(Metrics.gap)) {
          Glyph("fileText", Palette.fg3)
          Text(path, color = Palette.fg2, style = WebType.mono, maxLines = 1, overflow = TextOverflow.MiddleEllipsis)
        }
      } else {
        Text(line.ifEmpty { " " }, color = Palette.fg3, style = WebType.text3)
      }
    }
  }
}

private val NOTICE_PATH = Regex("""\s*-\s+(\S+)""")

/** One faint row closing the message: how the turn ended, the error's own words as the target */
@Composable
fun OutcomeRow(turn: AgentTurn, outcome: String) {
  // A warning glyph for the short stops, none for "stopped" / "no reply"
  val warn = turn.stop != TurnStop.cancelled && turn.stop != TurnStop.end_turn
  AcpRow(icon = if (warn) "triangleAlert" else null) {
    RowLabel(outcome)
    if (turn.stop == TurnStop.error) turn.error?.message?.takeIf { it.isNotBlank() }?.let { RowTarget(it, ink = Palette.fg3) }
  }
}

/** A goal milestone: its status glyph, the event and the objective's first line */
@Composable
fun GoalRow(block: GoalBlock) {
  AcpRow(icon = "goal") {
    RowLabel(t("goal.event.${block.event.name}"))
    block.objective?.lineSequence()?.firstOrNull()?.trim()?.takeIf { it.isNotEmpty() }?.let { RowTarget(it) }
  }
}
