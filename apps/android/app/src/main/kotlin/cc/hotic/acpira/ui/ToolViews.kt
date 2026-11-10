package cc.hotic.acpira.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.CubicBezierEasing
import androidx.compose.animation.core.animateDpAsState
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.snap
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.wrapContentHeight
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.IconButton
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTagsAsResourceId
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.Dp
import cc.hotic.acpira.design.Haptic
import cc.hotic.acpira.design.IconStore
import cc.hotic.acpira.design.IconStroke
import cc.hotic.acpira.design.LocalMotion
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.RowMotion
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.design.WebMetrics
import cc.hotic.acpira.design.WebType
import cc.hotic.acpira.design.play
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.logic.CodeToken
import cc.hotic.acpira.logic.GroupKind
import cc.hotic.acpira.logic.commandDuration
import cc.hotic.acpira.logic.fileName
import cc.hotic.acpira.logic.filePath
import cc.hotic.acpira.logic.groupNames
import cc.hotic.acpira.logic.lastComponent
import cc.hotic.acpira.logic.namesLabel
import cc.hotic.acpira.logic.toolFiles
import cc.hotic.acpira.logic.toolIcon
import cc.hotic.acpira.logic.toolVerb
import cc.hotic.acpira.logic.visibleToolContents
import cc.hotic.acpira.model.AsyncTaskInfo
import cc.hotic.acpira.model.DiffLine
import cc.hotic.acpira.model.DiffSource
import cc.hotic.acpira.model.DiffStat
import cc.hotic.acpira.model.RetryLog
import cc.hotic.acpira.model.ToolCallBlock
import cc.hotic.acpira.model.ToolContent
import cc.hotic.acpira.model.ToolKind
import cc.hotic.acpira.model.ToolStatus
import cc.hotic.acpira.ui.prose.codeRuns
import cc.hotic.acpira.ui.prose.rememberDiffTokens
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.LocalContentColor
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.layout.layout
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.role
import cc.hotic.acpira.logic.SessionLinks
import cc.hotic.acpira.logic.TaskRowState
import cc.hotic.acpira.logic.fileReference
import cc.hotic.acpira.logic.taskRowState
import cc.hotic.acpira.logic.taskUsage
import cc.hotic.acpira.model.FullDiffSource
import cc.hotic.acpira.model.ImageRef
import cc.hotic.acpira.store.MediaSource
import cc.hotic.acpira.store.imageMimeOf
import cc.hotic.acpira.store.toolImages
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import kotlin.math.ceil
import kotlin.math.max
import kotlin.math.roundToInt

// chat/ToolCall.tsx, ToolGroup.tsx, Terminal.tsx and the DiffBlock of CodeBlock.tsx under toolLine = icon, after the
// iOS app's Transcript/ToolViews.swift: every process row leads with its kind's icon, and an open row draws its rail
// from that icon.

/** The live clock in epoch ms (the engine's clock, offset), for running commands' seconds */
val LocalClock = compositionLocalOf<() -> Long> { { System.currentTimeMillis() } }

/** Inside the code sheet: a code card grows to its whole height and scrolls there */
private val LocalCodeSheet = compositionLocalOf { false }

// One tool call

@Composable
fun ToolCallView(block: ToolCallBlock, grouped: Boolean = false) {
  EntranceOnce("${block.id}:tool") { ToolRows(block, grouped) }
}

@Composable
private fun ToolRows(block: ToolCallBlock, grouped: Boolean) {
  val running = block.status == ToolStatus.in_progress
  val execute = block.kind == ToolKind.execute
  val target = block.target?.trim().orEmpty()
  // A shell command handed over verbatim: one line on the row, the whole command in the card
  val command = if (execute && block.targetMono == true && block.verbKey == null && target.isNotEmpty()) block.target else null
  val files = remember(block) { toolFiles(block) }
  val visible = remember(block) { visibleToolContents(block) }
  val span = if (block.kind == ToolKind.edit) editSpan(listOf(block)) else null
  val verb = toolVerb(block)
  val shownTarget = if (block.kind == ToolKind.read && files.isNotEmpty()) null else block.target
  val label: @Composable RowScope.() -> Unit = {
    ToolLabel(verb, running, command, shownTarget, block.targetMono == true, span, block.takeIf(SessionLinks::isTool))
  }
  val trailing: @Composable RowScope.() -> Unit = { ToolTrailing(block, execute) }
  val icon = toolIcon(block)
  val scope = LocalEntranceScope.current
  val fold = rememberFold("${scope.scope}:tool:${block.id}", false)
  val todos = block.todoEntries

  when {
    todos != null -> PlanDetails(todos, foldKey = block.id, label = label, trailing = trailing)
    // A skill load is one row naming the skill; only a failure opens to its message
    block.verbKey == "verb.skill" && !(block.status == ToolStatus.failed && visible.isNotEmpty()) ->
      AcpRow(tone = RowTone.action, icon = icon, trailing = trailing, content = label)
    // A process fold owns a separate, persistent media part; standalone tool rows keep their own result
    isImageResult(block) -> ImageResultRow(block, grouped) { AcpRow(tone = RowTone.action, icon = icon, trailing = trailing, content = label) }
    files.isNotEmpty() && block.kind == ToolKind.search ->
      Disclosure(fold.open, fold::toggle, icon = icon, indent = false, rail = RailMode.rows, label = label, trailing = trailing) {
        CompositionLocalProvider(LocalRowEnterDelay provides RowMotion.rowEnterFile) { ResultList(files, block.kind) }
      }
    files.size == 1 && block.kind == ToolKind.read ->
      FileRow(files[0], image = remember(block) { toolImages(block).firstOrNull() }, icon = icon, label = label, trailing = trailing)
    files.isNotEmpty() -> RailColumn(enabled = true, endAtLastRow = true) {
      AcpRow(tone = RowTone.action, icon = icon, trailing = trailing, content = label)
      EntranceOnce("${block.id}:files") {
        ResultList(files, block.kind, images = if (block.kind == ToolKind.read) remember(block) { toolImages(block) } else emptyList())
      }
    }
    block.kind == ToolKind.read || block.kind == ToolKind.search ||
      (block.kind == ToolKind.edit && visible.isEmpty() && block.asyncTask == null) ||
      (grouped && block.content == null && command == null) ->
      AcpRow(tone = RowTone.action, icon = icon, trailing = trailing, content = label)
    execute || (visible.isNotEmpty() && visible.all { it is ToolContent.Text }) ->
      // The command card sits in the label column, `codeCardGap` from the rows around it
      CommandDisclosure(block, command, visible, icon, label, trailing, defaultOpen = execute && !grouped && running)
    block.content is ToolContent.ListItems ->
      Disclosure(fold.open, fold::toggle, icon = icon, indent = false, rail = RailMode.rows, label = label, trailing = trailing) {
        ToolBody(block, visible)
      }
    else -> {
      // A diff is a code card too: in the label column on the row's rail, spaced as the command card is
      Disclosure(
        fold.open, fold::toggle, Modifier.padding(bottom = foldAfter(fold.open, WebMetrics.codeCardAfter)),
        icon = icon, bodyTop = WebMetrics.codeCardGap, label = label, trailing = trailing,
      ) {
        Column(verticalArrangement = Arrangement.spacedBy(Metrics.gap)) {
          TaskMeta(block.asyncTask)
          ToolBody(block, visible)
        }
      }
    }
  }
}

/** Room under an open card that eases with its fold */
@Composable
private fun foldAfter(open: Boolean, room: Dp): Dp {
  val motion = LocalMotion.current
  val shown by animateDpAsState(if (open) room else Metrics.s0, if (motion) RowMotion.fold() else snap(), label = "after")
  return shown
}

/** Verb, then the command or target (with an edit's line span); a session tool names the session by its title */
@Composable
private fun RowScope.ToolLabel(
  verb: String, shimmer: Boolean, command: String?, target: String?, mono: Boolean, span: String?, sessionTool: ToolCallBlock? = null,
) {
  RowLabel(verb, shimmer)
  when {
    command != null -> RowTarget(command, mono = true)
    sessionTool != null -> SessionToolTarget(sessionTool)
    !target.isNullOrEmpty() -> TargetWithAside(target, span, mono)
  }
}

/** The app's session list, for rows that name a session by its title (read / search session tools) */
val LocalSessionSummaries = compositionLocalOf<List<cc.hotic.acpira.model.SessionSummary>> { emptyList() }

/**
 * A read / search session row's target (SessionToolTarget): the session's title in place of its link, then the turns
 * read or the query. Only these rows read the list, in the chat and the child inspector alike
 */
@Composable
private fun RowScope.SessionToolTarget(block: ToolCallBlock) {
  val (id, rest) = SessionLinks.splitTarget(block.target)
  val aside = if (block.verbKey == "verb.readSession") SessionLinks.readLabel(rest) else rest
  if (id != null) {
    TargetWithAside(SessionLinks.title(id, LocalSessionSummaries.current), aside)
  } else if (aside != null) {
    RowTarget(aside)
  }
}

/**
 * Commands end with ✓ / ✗ and their run time; other rows show their diff stat; a background task its state. A running
 * command the session can stop on its own leads with Stop
 */
@Composable
internal fun RowScope.ToolTrailing(block: ToolCallBlock, execute: Boolean) {
  val task = block.asyncTask
  when {
    execute && task != null -> TaskTag(task)
    execute -> {
      val stop = LocalChatHandlers.current?.stopCommand
      if (stop != null && LocalCanStopCommands.current && commandStoppable(block)) CommandStop(block.id, stop)
      val start = block.startedAt
      val end = block.endedAt
      CommandOutcome(
        block.status,
        if (start != null && end != null) commandDuration(end - start) else null,
        if (block.status == ToolStatus.in_progress) start else null,
      )
    }
    else -> {
      block.diffStat?.let { DiffStatText(it.add, it.del) }
      task?.let { TaskTag(it) }
    }
  }
}

/** A command the agent handed over verbatim and still running: the only rows whose process the engine can find */
internal fun commandStoppable(block: ToolCallBlock): Boolean =
  block.kind == ToolKind.execute && block.status == ToolStatus.in_progress && block.observation != "unknown" &&
    block.targetMono == true && block.verbKey == null && !block.target.isNullOrBlank()

/**
 * Stop on a running command's row, the same square as a background task's: ends that command's process and the turn
 * goes on. Gone once tapped, so a second tap has nothing to land on while the process dies
 */
@Composable
private fun CommandStop(id: String, stop: (String) -> Unit) {
  var asked by remember(id) { mutableStateOf(false) }
  if (asked) return
  StopSquare(t("command.stop"), "command.stop") { asked = true; stop(id) }
}

/** The small stop square of a process row, as the webview's sm button with a size-3 glyph: the button's box overhangs
 *  the row by a step above, below and after */
@Composable
private fun StopSquare(label: String, tag: String, onClick: () -> Unit) {
  val bleed = with(LocalDensity.current) { Metrics.s1.roundToPx() }
  val haptics = LocalHapticFeedback.current
  Box(
    Modifier
      .layout { measurable, constraints ->
        val p = measurable.measure(constraints.copy(minWidth = 0, minHeight = 0, maxHeight = Constraints.Infinity))
        layout(maxOf(p.width - bleed, 0), maxOf(p.height - bleed * 2, 0)) { p.place(0, -bleed) }
      }
      .size(WebMetrics.ctlSm)
      .clip(CircleShape)
      .clickable { haptics.play(Haptic.Stop); onClick() }
      .semantics { contentDescription = label; role = Role.Button }
      .testTag(tag),
    contentAlignment = Alignment.Center,
  ) {
    Glyph("square", Palette.fg2, WebMetrics.chevron, IconStroke.icon)
  }
}

/**
 * An AIR async task on its row: the state ("Stopping…" once a stop is asked; done in the ok tone, failed in danger),
 * then Stop while the task runs and the adapter can stop it. A refusal comes back as the engine's error toast, and the
 * row is live again with its Stop
 */
@Composable
private fun RowScope.TaskTag(task: AsyncTaskInfo) {
  val state = taskRowState(task)
  val tone = when (state.tone) {
    TaskRowState.Tone.danger -> Palette.danger
    TaskRowState.Tone.ok -> Palette.ok
    TaskRowState.Tone.quiet -> Palette.fg3
  }
  Text(state.tag, color = tone, maxLines = 1, softWrap = false)
  val stop = LocalChatHandlers.current?.stopTask
  // Small enough to sit in a process row without growing it
  if (state.stoppable && stop != null) StopSquare(t("asyncTask.stop"), "task.stop") { stop(task.id) }
}

/**
 * TaskMeta: the task's own report under its row, its latest summary, the tool it was last on and its usage. The output
 * file stays hidden: the card already shows the command
 */
@Composable
private fun TaskMeta(task: AsyncTaskInfo?) {
  task ?: return
  val usage = taskUsage(task)
  // A workflow's last tool is whichever of its agents reported last, which says nothing on its own
  val tool = if (task.taskType == "workflow") null else task.lastToolName?.takeIf { it.isNotBlank() }
  val summary = task.summary?.takeIf { it.isNotBlank() }
  if (summary == null && tool == null && usage == null) return
  Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(Metrics.s1)) {
    for (line in listOfNotNull(summary, tool, usage)) Text(line, color = Palette.fg3, style = WebType.text3)
  }
}

private val EaseOut = CubicBezierEasing(0f, 0f, 0.58f, 1f)
/** SwiftUI's `.bouncy` spring over the outcome's draw: bounce 0.3 over 0.42 s */
private const val OUTCOME_DAMPING = 0.7f
private val OUTCOME_STIFFNESS = (2 * Math.PI / (RowMotion.outcomeDraw / 1000.0)).let { (it * it).toFloat() }

/**
 * CommandOutcome.tsx: a command's ✓ / ✗ in a slot held while it runs, so its arrival moves nothing on the row, then
 * its run time (a live command ticks whole seconds; a finished one reads its timestamps). A command that ends on screen
 * draws its glyph with a small pop and fades its final time in; one already ended when the row was composed stays still
 */
@Composable
internal fun RowScope.CommandOutcome(status: ToolStatus, duration: String?, since: Double?) {
  val done = status == ToolStatus.completed || status == ToolStatus.failed
  val motion = LocalMotion.current
  val live = remember { mutableStateOf(!done) }
  val draw = remember { Animatable(1f) }
  val time = remember { Animatable(1f) }
  LaunchedEffect(done) {
    if (!done || !live.value) return@LaunchedEffect
    live.value = false
    if (!motion) return@LaunchedEffect
    launch {
      draw.snapTo(0f)
      draw.animateTo(1f, spring(OUTCOME_DAMPING, OUTCOME_STIFFNESS))
    }
    time.snapTo(0f)
    delay(RowMotion.outcomeTimeDelay.toLong())
    time.animateTo(1f, tween(RowMotion.outcomeTime, easing = EaseOut))
  }
  val context = LocalContext.current
  val ok = status == ToolStatus.completed
  val ink = if (ok) Palette.ok else Palette.danger
  val paths = remember(ok) { IconStore.lucidePaths(context, if (ok) "check" else "x").orEmpty() }
  Canvas(
    Modifier.size(WebMetrics.chevron).graphicsLayer {
      val s = RowMotion.outcomePopFrom + (1 - RowMotion.outcomePopFrom) * draw.value
      scaleX = s
      scaleY = s
    },
  ) {
    if (done) drawTrimmed(paths, draw.value.coerceIn(0f, 1f), ink, IconStroke.outcome)
  }
  // The slot keeps at least the width of the widest time below a minute, so the ticking whole seconds turning into
  // the final tenths never shift the command beside them
  if (duration != null || since != null) {
    val hair = with(LocalDensity.current) { Metrics.hairline.toPx() }
    Box(contentAlignment = Alignment.CenterEnd) {
      Text(commandDuration(SHORTEST_MS), modifier = Modifier.alpha(0f), maxLines = 1)
      if (duration != null) {
        Text(duration, maxLines = 1, modifier = Modifier.graphicsLayer {
          alpha = time.value
          translationY = (1 - time.value) * hair * 2
        })
      } else if (since != null) {
        CommandSeconds(since)
      }
    }
  }
}

private const val SHORTEST_MS = 50.0

/** A running command's whole seconds */
@Composable
private fun CommandSeconds(since: Double) {
  val now = LocalClock.current
  var tick by remember { mutableLongStateOf(now()) }
  LaunchedEffect(since) {
    while (true) {
      tick = now()
      delay(RowMotion.clockTick)
    }
  }
  Text(commandDuration(tick - since, whole = true), maxLines = 1)
}

/**
 * RetryRow.tsx: one run of provider retries, drawn like a tool row. The verb says where the run stands (shimmering
 * while live), the trailing text counts the attempt and the wait while it lasts, then the outcome glyph; the card under
 * it holds every failed attempt in the provider's own words. A lost connection leads with a plug
 */
@Composable
fun RetryRow(id: String, log: RetryLog) {
  val running = log.status == "in_progress"
  val n = log.attempts.size
  val reconnect = log.reconnect == true
  val verb = when {
    running -> t(if (reconnect) "retry.reconnecting" else "retry.retrying")
    log.status == "completed" -> t("retry.recovered", "n" to n)
    log.status == "failed" -> t("retry.failed", "n" to n)
    else -> t("retry.stopped")
  }
  val status = ToolStatus.entries.firstOrNull { it.name == log.status } ?: ToolStatus.in_progress
  val fold = rememberFold("${LocalEntranceScope.current.scope}:retry:$id", false)
  Disclosure(
    fold.open, fold::toggle, icon = if (reconnect) "unplug" else "rotateCw",
    label = { RowLabel(verb, shimmer = running) },
    trailing = {
      if (running) RetryMeta(log.attempts.lastOrNull()?.attempt, log.max, log.nextAt)
      CommandOutcome(status, duration = null, since = null)
    },
  ) {
    RetryOutput(log)
  }
}

/** "Attempt 5/10, in 12s" while the next attempt waits, the bare attempt once it is under way */
@Composable
private fun RetryMeta(attempt: Int?, max: Int?, nextAt: Double?) {
  val now = LocalClock.current
  var tick by remember { mutableLongStateOf(now()) }
  LaunchedEffect(nextAt) {
    while (nextAt != null && nextAt > tick) {
      delay(RowMotion.clockTick)
      tick = now()
    }
  }
  val left = if (nextAt == null) 0 else ceil((nextAt - tick) / 1000).toInt()
  val text = when {
    attempt != null && max != null && left > 0 -> t("retry.attemptIn", "attempt" to attempt, "max" to max, "seconds" to left)
    attempt != null && max != null -> t("retry.attempt", "attempt" to attempt, "max" to max)
    left > 0 -> t("retry.in", "seconds" to left)
    else -> null
  }
  if (text != null) Text(text, maxLines = 1)
}

/** Every attempt on its own line: when it failed and which attempt (muted), then the error verbatim */
@Composable
private fun RetryOutput(log: RetryLog) {
  val faint = Palette.fg3
  val clock = remember { SimpleDateFormat("HH:mm:ss", Locale.ROOT) }
  val lines = remember(log, faint) {
    val rows = log.attempts.flatMap { a ->
      val head = listOfNotNull(
        clock.format(Date(a.at.toLong())),
        if (a.attempt != null && log.max != null) t("retry.attempt", "attempt" to a.attempt!!, "max" to log.max!!) else null,
      ).joinToString("  ")
      val parts = a.text.split('\n')
      listOf(head to parts.first()) + parts.drop(1).map { "" to it }
    }
    buildAnnotatedString {
      rows.forEachIndexed { i, (head, text) ->
        if (head.isEmpty()) append("  ") else { withStyle(SpanStyle(color = faint)) { append(head) }; append("  ") }
        append(text.ifEmpty { " " })
        if (i < rows.size - 1) append("\n")
      }
    }
  }
  CodeFrame(title = t("retry.log"), copy = lines.text, copyLabel = t("retry.copyLog"), full = { RetryOutput(log) }) {
    CodeCap {
      Box(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState())) {
        Text(
          lines, color = Palette.fg2, style = WebType.mono, softWrap = false,
          modifier = Modifier.padding(horizontal = WebMetrics.commandX, vertical = WebMetrics.commandY),
        )
      }
    }
  }
}

/** An execute (or text-only) call: the row opens the command card, which sits in the label column on the row's rail */
@Composable
private fun CommandDisclosure(
  block: ToolCallBlock,
  command: String?,
  items: List<ToolContent>,
  icon: String,
  label: @Composable RowScope.() -> Unit,
  trailing: @Composable RowScope.() -> Unit,
  defaultOpen: Boolean,
) {
  val scope = LocalEntranceScope.current
  val fold = rememberFold("${scope.scope}:command:${block.id}", defaultOpen)
  Disclosure(
    fold.open, fold::toggle, Modifier.padding(bottom = foldAfter(fold.open, WebMetrics.codeCardAfter)),
    icon = icon, bodyTop = WebMetrics.codeCardGap, label = label, trailing = trailing,
  ) {
    Column(verticalArrangement = Arrangement.spacedBy(Metrics.gap)) {
      TaskMeta(block.asyncTask)
      if (block.kind == ToolKind.execute) {
        TerminalOutput(block, command)
        // An image the command produced (screenshot tools) sits below its output (ToolCall.tsx ToolBody)
        for (image in remember(block) { cc.hotic.acpira.store.toolImages(block) }) AgentImageView(image.blob, image.uri)
      } else {
        ToolOutput(items.filterIsInstance<ToolContent.Text>().joinToString("\n") { it.text })
      }
    }
  }
}

/** Diff / list / text bodies, stacked in wire order */
@Composable
private fun ToolBody(block: ToolCallBlock, items: List<ToolContent>) {
  Column(verticalArrangement = Arrangement.spacedBy(Metrics.gap)) {
    for (item in items) {
      when (item) {
        is ToolContent.Diff -> DiffBlockView(item.lines, block.target.orEmpty(), item.source, diffLocation(block, item))
        is ToolContent.ListItems -> ResultList(item.items, block.kind)
        is ToolContent.Text -> ToolOutput(item.text)
        is ToolContent.Image -> if (!cc.hotic.acpira.store.isWidget(item.mimeType)) AgentImageView(item.blob, item.uri)
        is ToolContent.Unknown -> {}
      }
    }
  }
}

// Result list

/**
 * File / hit rows under a read or search: dense action rows, the name in mono fg-2 and the line as a faint note.
 * [images]: a read's saved images, which its file rows open
 */
@Composable
private fun ResultList(items: List<String>, kind: ToolKind, images: List<ImageRef> = emptyList()) {
  val icon = if (kind == ToolKind.fetch) "globe" else "fileText"
  Column {
    for ((i, item) in items.withIndex()) {
      if (kind == ToolKind.read || kind == ToolKind.search) {
        FileRow(item, image = rowImage(item, i, items, images), dense = true, icon = icon)
      } else {
        val hit = splitHit(item)
        AcpRow(
          tone = RowTone.action, dense = true, icon = icon,
          trailing = hit.second?.let { aside -> { Text(aside, maxLines = 1, softWrap = false) } },
        ) {
          RowTarget(hit.first, mono = kind != ToolKind.fetch, ink = Palette.fg2)
        }
      }
    }
  }
}

/**
 * ResultList's pairing of a file row with the call's saved images: the one image of a lone row, else the image whose uri
 * names the row's file, else the image at the row's place when there is one per row or per image row
 */
internal fun rowImage(item: String, index: Int, items: List<String>, images: List<ImageRef>): ImageRef? {
  if (images.isEmpty()) return null
  if (items.size == 1 && images.size == 1) return images[0]
  val main = splitHit(item).first
  val named = images.firstOrNull { image ->
    val uri = image.uri ?: return@firstOrNull false
    val path = cc.hotic.acpira.store.fileOfUri(uri) ?: uri
    uri == item || path == item || (main.isNotEmpty() && (path.endsWith("/$main") || path.endsWith("\\$main")))
  }
  if (named != null) return named
  if (images.size == items.size) return images[index]
  var imageIndex = -1
  var imageCount = 0
  for ((j, hit) in items.withIndex()) {
    val (p, line) = fileReference(hit)
    if (line == null && imageMimeOf(p) != null) {
      if (j == index) imageIndex = imageCount
      imageCount++
    }
  }
  return if (imageIndex >= 0 && imageCount == images.size) images[imageIndex] else null
}

/**
 * What a file row opens in the Lightbox (ToolCall.tsx FileRef): the call's saved copy of the image, else the file itself
 * where the link can stage it. null for a line reference and any other file: the row stays plain, since the phone opens
 * nothing else from a process row
 */
private fun fileRowImage(hit: String, image: ImageRef?, session: String?, canStage: Boolean): MediaSource? {
  val (path, line) = fileReference(hit)
  if (line != null || session == null) return null
  MediaSource.stored(image?.blob, session)?.let { return it }
  if (!canStage || imageMimeOf(path) == null) return null
  return MediaSource.HostFile(session, path)
}

/**
 * A read or search row's file (FileResultRow / FileRef): its name in mono fg-2 and the line as a faint note. A file with
 * an image to show (`fileRowImage`) is underlined, as the desktop underlines it under the pointer (a phone has no hover),
 * and the whole row opens it in the Lightbox. Its pixels may still have to come over the link: the file glyph then turns
 * into the desktop's spinner until they land, and a picture that cannot load says so in a toast
 */
@Composable
private fun FileRow(
  hit: String,
  image: ImageRef? = null,
  dense: Boolean = false,
  icon: String = "fileText",
  label: (@Composable RowScope.() -> Unit)? = null,
  trailing: (@Composable RowScope.() -> Unit)? = null,
) {
  val split = splitHit(hit)
  val media = LocalMedia.current
  val motion = LocalMotion.current
  val source = remember(hit, image, media) { fileRowImage(hit, image, media.session, media.loader.canStage) }
  // The tap's load while the pixels are on their way; the scope ends with the row, so a row that left the screen
  // meanwhile opens nothing (the load still fills the cache)
  val scope = rememberCoroutineScope()
  var pending by remember { mutableStateOf<Job?>(null) }
  val open: (MediaSource) -> Unit = open@{ s ->
    val lightbox = media.lightbox ?: return@open
    if (pending != null) return@open
    media.loader.peek(s)?.let { lightbox.open(LightboxItem(it, split.first, s)); return@open }
    pending = scope.launch {
      val picture = runCatching { media.loader.image(s) }.getOrNull()
      pending = null
      if (picture != null) lightbox.open(LightboxItem(picture, split.first, s))
      else media.model?.showToast(t("attach.loadFailed", "name" to split.first))
    }
  }
  // The `popup` launch extra `readimage` taps the first file row with an image, for screenshots
  LaunchedEffect(source) {
    if (source == null || media.model?.options?.popup != "readimage" || ReadImageProbe.claimed) return@LaunchedEffect
    ReadImageProbe.claimed = true
    delay(PROBE_DELAY_MS)
    open(source)
  }
  val loading by animateFloatAsState(if (pending != null) 1f else 0f, tween(if (motion) QUICK_MS else 0), label = "fileLoad")
  val ink = Palette.actionFg
  AcpRow(
    if (source == null) Modifier else Modifier.clickable(remember { MutableInteractionSource() }, indication = null, role = Role.Button) { open(source) },
    tone = RowTone.action, dense = dense, icon = icon, trailing = trailing,
    // The glyph stays in place, hidden, so the rail keeps measuring the file icon under the spinner
    iconTint = if (loading > 0f) ink.copy(alpha = ink.alpha * (1 - loading)) else null,
    iconOverlay = if (loading > 0f) ({ LoadingGlyph(WebMetrics.icon, Modifier.alpha(loading)) }) else null,
  ) {
    label?.invoke(this)
    TargetWithAside(split.first, split.second, mono = true, ink = Palette.fg2, underline = source != null)
  }
}

/** `popup readimage` opens once per launch */
private object ReadImageProbe {
  var claimed = false
}

private const val PROBE_DELAY_MS = 1_000L

/**
 * The desktop's loading glyph (Loader2 under animate-spin: one turn a second, linear) at a row icon's size and stroke.
 * It keeps turning with motion off, as the list's working ring does: a still one reads as stuck
 */
@Composable
private fun LoadingGlyph(size: Dp, modifier: Modifier = Modifier) {
  val turn = rememberInfiniteTransition(label = "loading")
  val angle by turn.animateFloat(0f, FULL_TURN, infiniteRepeatable(tween(SPIN_MS, easing = LinearEasing)), label = "spin")
  Glyph(
    "loaderCircle", LocalContentColor.current, size, IconStroke.icon,
    modifier.graphicsLayer { rotationZ = angle }.semantics { contentDescription = t("attach.loading") },
  )
}

private const val FULL_TURN = 360f
private const val SPIN_MS = 1_000

// Groups

/** A run of consecutive reads or edits under one head: the verb, the files it touched, an edit run's total change */
@Composable
fun ToolGroupView(kind: GroupKind, blocks: List<ToolCallBlock>) {
  val current = blocks.lastOrNull { it.status == ToolStatus.pending || it.status == ToolStatus.in_progress } ?: blocks.last()
  val names = groupNames(blocks)
  val stat = if (kind == GroupKind.edit) diffStatOf(blocks) else null
  val scope = LocalEntranceScope.current
  val fold = rememberFold("${scope.scope}:group:${kind.name}:${blocks.first().id}", false)
  Disclosure(
    fold.open, fold::toggle,
    icon = if (kind == GroupKind.read) "fileText" else "pencil", indent = false, rail = RailMode.rows, bodyTop = Metrics.s0,
    label = {
      RowLabel(toolVerb(current), shimmer = current.status == ToolStatus.pending || current.status == ToolStatus.in_progress)
      if (names.isNotEmpty()) RowTarget(namesLabel(names), mono = true)
    },
    trailing = stat?.let { s -> { DiffStatText(s.add, s.del) } },
  ) {
    // `tool-results`: file rows enter after the head
    CompositionLocalProvider(LocalRowEnterDelay provides RowMotion.rowEnterFile) {
      Column {
        if (kind == GroupKind.read) {
          for (b in blocks) androidx.compose.runtime.key(b.id) { EntranceOnce(itemEntrance(b.id)) { ReadEntry(b) } }
        } else {
          for ((key, entry) in editEntries(blocks)) androidx.compose.runtime.key(key) {
            EntranceOnce(itemEntrance(entry.first().id)) { EditEntry(entry) }
          }
        }
      }
    }
  }
}

@Composable
private fun ReadEntry(block: ToolCallBlock) {
  val files = remember(block) { toolFiles(block) }
  if (files.isNotEmpty()) {
    ResultList(files, ToolKind.read, images = remember(block) { toolImages(block) })
  } else {
    AcpRow(tone = RowTone.action, dense = true, icon = "fileText") {
      Shimmer(
        block.target ?: toolVerb(block), block.status == ToolStatus.pending || block.status == ToolStatus.in_progress,
        Modifier.weight(1f, fill = false).alignByBaseline(), WebType.mono,
      )
    }
  }
}

/** One file of an edit group: its changed lines and total change; the diffs open on a tap */
@Composable
private fun EditEntry(blocks: List<ToolCallBlock>) {
  val stat = diffStatOf(blocks)
  val span = editSpan(blocks)
  // Each item with its call, which locates a diff for the engine
  val diffs = blocks.flatMap { b -> visibleToolContents(b).filter { it is ToolContent.Diff || it is ToolContent.Text }.map { b to it } }
  val name = fileName(blocks[0])
  val label: @Composable RowScope.() -> Unit = { TargetWithAside(name, span, mono = true, ink = Palette.fg2) }
  val trailing: (@Composable RowScope.() -> Unit)? = stat?.let { s -> { DiffStatText(s.add, s.del) } }
  if (diffs.isEmpty()) {
    AcpRow(tone = RowTone.action, dense = true, icon = "pencil", trailing = trailing, content = label)
    return
  }
  val scope = LocalEntranceScope.current
  val fold = rememberFold("${scope.scope}:edit:${blocks.first().id}", false)
  // No rail root of its own, so the group's rail runs on through its icon. The diff is a code card in the label column
  // like every other (entries stack without the process rows' gap, so it takes the whole gap)
  Disclosure(
    fold.open, fold::toggle, Modifier.padding(bottom = foldAfter(fold.open, WebMetrics.codeCardGap)),
    dense = true, icon = "pencil", root = false, bodyTop = WebMetrics.codeCardGap, label = label, trailing = trailing,
  ) {
    Column(verticalArrangement = Arrangement.spacedBy(Metrics.gap)) {
      for ((block, item) in diffs) {
        when (item) {
          is ToolContent.Diff -> DiffBlockView(item.lines, name, item.source, diffLocation(block, item))
          is ToolContent.Text -> ToolOutput(item.text)
          else -> {}
        }
      }
    }
  }
}

// Code cards

/**
 * The `code-output` frame (rounded-md, a hairline, the code background) around a card that opens whole in the code
 * sheet on a tap; inside the sheet the card stays as it is
 */
@Composable
private fun CodeFrame(
  title: String,
  copy: String,
  copyLabel: String,
  full: @Composable () -> Unit,
  /** The text copy takes when it has to be fetched first (a diff's whole file); [copy] until then */
  read: (suspend () -> String)? = null,
  content: @Composable () -> Unit,
) {
  val sheet = LocalCodeSheet.current
  var shown by remember { mutableStateOf(false) }
  val shape = RoundedCornerShape(Metrics.rMd)
  Box(
    Modifier.fillMaxWidth().clip(shape).background(Palette.bgCode).border(Metrics.hairline, Palette.line, shape).padding(Metrics.hairline)
      .then(if (sheet) Modifier else Modifier.clickable(remember { MutableInteractionSource() }, indication = null) { shown = true }),
  ) { content() }
  if (shown) CodeSheet(title, copy, copyLabel, onDismiss = { shown = false }, read = read) { full() }
}

/** The code sheet: one card whole, both ways scrollable, under a title naming it, with copy and close in its bar */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun CodeSheet(
  title: String,
  copy: String,
  copyLabel: String,
  onDismiss: () -> Unit,
  read: (suspend () -> String)? = null,
  content: @Composable () -> Unit,
) {
  val state = rememberModalBottomSheetState(skipPartiallyExpanded = false)
  val context = LocalContext.current
  val scope = rememberCoroutineScope()
  val haptics = LocalHapticFeedback.current
  var copied by remember { mutableIntStateOf(0) }
  LaunchedEffect(copied) {
    if (copied > 0) { delay(COPIED_MS); copied = 0 }
  }
  ModalBottomSheet(onDismissRequest = onDismiss, sheetState = state, containerColor = Palette.bg0, dragHandle = null) {
    Column(Modifier.fillMaxWidth().fillMaxHeight().navigationBarsPadding().semantics { testTagsAsResourceId = true }.testTag("code.sheet")) {
      Row(
        Modifier.fillMaxWidth().heightIn(min = Metrics.barCtl + Metrics.s2).padding(horizontal = Metrics.s1),
        verticalAlignment = Alignment.CenterVertically,
      ) {
        IconButton(onClick = onDismiss, modifier = Modifier.size(Metrics.barCtl).testTag("code.close")) {
          Lucide("x", Palette.fg1, size = Metrics.iconCtl, contentDescription = t("common.close"))
        }
        Text(
          title, color = Palette.fgStrong, fontSize = Type.text1, fontWeight = FontWeight.SemiBold, maxLines = 1,
          overflow = TextOverflow.MiddleEllipsis, textAlign = TextAlign.Center, modifier = Modifier.weight(1f),
        )
        IconButton(
          onClick = {
            if (read == null) { copyText(context, copy); haptics.play(Haptic.Copy); copied += 1 }
            else scope.launch { copyText(context, read()); haptics.play(Haptic.Copy); copied += 1 }
          },
          modifier = Modifier.size(Metrics.barCtl).testTag("code.copy"),
        ) {
          Lucide(if (copied > 0) "check" else "copy", Palette.fg1, size = Metrics.iconCtl, contentDescription = if (copied > 0) t("code.copied") else copyLabel)
        }
      }
      CompositionLocalProvider(LocalCodeSheet provides true) {
        Box(Modifier.weight(1f).verticalScroll(rememberScrollState()).padding(horizontal = Metrics.pad, vertical = Metrics.s2)) { content() }
      }
    }
  }
}

private const val COPIED_MS = 1500L

/**
 * A code card's body cut at --code-output-max in the transcript, whole in the code sheet. The cut does not scroll
 * vertically (long lines still scroll sideways); a fade at the cut shows there is more, and a tap opens the whole card
 */
@Composable
private fun CodeCap(content: @Composable () -> Unit) {
  if (LocalCodeSheet.current) { content(); return }
  var natural by remember { mutableIntStateOf(0) }
  val max = with(LocalDensity.current) { WebMetrics.codeOutputMax.toPx() }
  val fade = with(LocalDensity.current) { WebMetrics.scrollFade.toPx() }
  val bg = Palette.bgCode
  Box(
    Modifier.fillMaxWidth().heightIn(max = WebMetrics.codeOutputMax).clipToBounds().drawWithContent {
      drawContent()
      if (natural > max) {
        drawRect(
          Brush.verticalGradient(listOf(bg.copy(alpha = 0f), bg), startY = size.height - fade, endY = size.height),
          topLeft = Offset(0f, size.height - fade), size = Size(size.width, fade),
        )
      }
    },
  ) {
    Box(Modifier.fillMaxWidth().wrapContentHeight(Alignment.Top, unbounded = true).onSizeChanged { natural = it.height }) { content() }
  }
}

/** Command output (Codex-style Shell card): the full command behind a muted `$`, then the output */
@Composable
private fun TerminalOutput(block: ToolCallBlock, command: String?) {
  val text = outputOf(block)
  if (text.isEmpty() && command == null) return
  val sheet = LocalCodeSheet.current
  CodeFrame(
    title = command ?: toolVerb(block),
    copy = listOfNotNull(command?.let { "$ $it" }, text).joinToString("\n"),
    copyLabel = t("code.copyOutput"),
    full = { TerminalOutput(block, command) },
  ) {
    Column {
      if (command != null) CommandPrompt(command, last = text.isEmpty())
      if (text.isNotEmpty()) {
        val ok = Palette.ok
        val faint = Palette.fg3
        val lines = remember(text, sheet) { terminalLines(text, if (sheet) null else PREVIEW_LINES, ok, faint) }
        CodeCap {
          Box(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState())) {
            Text(
              lines, color = Palette.fg2, style = WebType.mono, softWrap = false,
              modifier = Modifier.padding(
                start = WebMetrics.commandX, end = WebMetrics.commandX,
                top = if (command != null) Metrics.gapHalf else WebMetrics.commandY, bottom = WebMetrics.commandY,
              ),
            )
          }
        }
      }
    }
  }
}

/**
 * The lines the transcript's card can show: it is cut at --code-output-max and never scrolls there, so a running
 * command's streamed output is laid out only down to the cut; one line more keeps the card past the cut so its fade
 * still shows. The code sheet shows every line
 */
private val PREVIEW_LINES = ceil(WebMetrics.codeOutputMax.value / WebType.mono.lineHeight.value).toInt() + 1

/** npm banners muted, a leading ✓ in the success colour; the first [limit] lines only when given */
private fun terminalLines(text: String, limit: Int?, ok: Color, faint: Color): AnnotatedString {
  var lines = text.trimEnd().split('\n')
  if (limit != null && lines.size > limit) lines = lines.take(limit)
  return buildAnnotatedString {
    lines.forEachIndexed { i, line ->
      val end = if (i < lines.size - 1) "\n" else ""
      val check = line.indexOf('✓')
      when {
        line.startsWith("> ") || line.startsWith("Done in ") -> withStyle(SpanStyle(color = faint)) { append(line + end) }
        check >= 0 && line.substring(0, check).isBlank() -> {
          append(line.substring(0, check))
          withStyle(SpanStyle(color = ok)) { append("✓") }
          append(line.substring(check + 1) + end)
        }
        else -> append(line + end)
      }
    }
  }
}

/**
 * The full command, cut to three lines until expanded; wrapped lines hang under the command, not the `$`. The height
 * eases between the cut and the whole command as a fold's reveal does, the two cross-fading; a cut command expands
 * from anywhere on its lines, ahead of the card's own tap
 */
@Composable
private fun CommandPrompt(command: String, last: Boolean) {
  var expanded by remember { mutableStateOf(false) }
  var clipped by remember { mutableStateOf(false) }
  val motion = LocalMotion.current
  val progress by animateFloatAsState(if (expanded) 1f else 0f, if (motion) RowMotion.fold(PROMPT_STEP) else snap(), label = "prompt")
  val style = WebType.mono
  Row(
    Modifier.fillMaxWidth()
      .then(if (clipped) Modifier.clickable(remember { MutableInteractionSource() }, indication = null) { expanded = !expanded } else Modifier)
      .padding(start = WebMetrics.commandX, end = WebMetrics.commandX, top = WebMetrics.commandY, bottom = if (last) WebMetrics.commandY else Metrics.s0),
  ) {
    Text("$ ", color = Palette.fg3, style = style)
    Column {
      Layout(
        content = {
          Text(
            command, color = Palette.fg3, style = style,
            onTextLayout = { clipped = it.lineCount > WebMetrics.commandPromptLines },
            modifier = Modifier.graphicsLayer { alpha = if (clipped) progress else 1f },
          )
          if (clipped) {
            Text(
              command, color = Palette.fg3, style = style, maxLines = WebMetrics.commandPromptLines, overflow = TextOverflow.Ellipsis,
              modifier = Modifier.graphicsLayer { alpha = 1 - progress },
            )
          }
        },
        modifier = Modifier.clipToBounds(),
      ) { measurables, constraints ->
        val free = constraints.copy(minHeight = 0, maxHeight = Constraints.Infinity)
        val whole = measurables[0].measure(free)
        val cut = measurables.getOrNull(1)?.measure(free)
        val height = if (cut == null) whole.height else (cut.height + (whole.height - cut.height) * progress.coerceIn(0f, 1f)).roundToInt()
        layout(max(whole.width, cut?.width ?: 0), height) {
          whole.placeRelative(0, 0)
          cut?.placeRelative(0, 0)
        }
      }
      if (clipped) {
        Box(
          Modifier.padding(top = Metrics.s0_5).background(Palette.fg3.copy(alpha = CHEVRON_FILL), RoundedCornerShape(Metrics.rSm))
            .padding(horizontal = Metrics.gapHalf),
        ) {
          FoldCaretSmall(expanded)
        }
      }
    }
  }
}

private const val PROMPT_STEP = 0.001f
private const val CHEVRON_FILL = 0.14f

@Composable
private fun FoldCaretSmall(open: Boolean) {
  val motion = LocalMotion.current
  val turn by animateFloatAsState(if (open) HALF_TURN else 0f, if (motion) RowMotion.fold(CARET_STEP) else snap(), label = "promptCaret")
  Glyph("chevronDown", Palette.fg3, WebMetrics.chevron, IconStroke.chevron, Modifier.graphicsLayer { rotationZ = turn })
}

private const val HALF_TURN = 180f
private const val CARET_STEP = 0.1f

/** Text a non-command tool returned, wrapped (reports and prose, not column-aligned logs) */
@Composable
private fun ToolOutput(text: String) {
  val body = text.trimEnd()
  if (body.isBlank()) return
  CodeFrame(t("code.copyToolOutput"), body, t("code.copyToolOutput"), full = { ToolOutput(text) }) {
    CodeCap {
      Text(
        body, color = Palette.fg2, style = WebType.mono,
        modifier = Modifier.fillMaxWidth().padding(horizontal = WebMetrics.commandX, vertical = WebMetrics.commandY),
      )
    }
  }
}

// Diff

/**
 * DiffBlock: line numbers and signs in a gutter, tinted add / del rows, `···` between hunks. The rows fill at least the
 * card's width, so a tint runs edge to edge however short its line is. Code is coloured by the file's grammar (the
 * visible segments, as on iOS). Copy takes the new file (CodeBlock.tsx): the one the diff carries, else asked of the
 * engine when the view left the files out ([locate] says where the diff sits), else the visible new code
 */
@Composable
private fun DiffBlockView(lines: List<DiffLine>, title: String, source: DiffSource?, locate: DiffLocation? = null) {
  val sheet = LocalCodeSheet.current
  val display = remember(lines, sheet) { if (sheet) lines else diffPreview(lines) }
  val tokens = rememberDiffTokens(display, source?.path ?: title, sheet)
  val copy = remember(lines, source) { diffCopyText(lines, source) }
  val fetch = LocalDiffSource.current
  val fetchable = source?.omitted == true && locate != null && fetch != null
  val full = source != null && source.omitted != true && source.newText != null
  // The new file from the engine; the visible code when it no longer has the call
  val read: (suspend () -> String)? = if (fetchable) ({ fetch!!(locate!!)?.newText ?: copy }) else null
  CodeFrame(
    title, copy, t(if (full || fetchable) "code.copySource" else "code.copyVisible"),
    full = { DiffBlockView(lines, title, source, locate) }, read = read,
  ) {
    CodeCap {
      BoxWithConstraints(Modifier.fillMaxWidth()) {
        val viewport = maxWidth
        Box(Modifier.horizontalScroll(rememberScrollState())) {
          Column(Modifier.width(IntrinsicSize.Max).widthIn(min = viewport).padding(vertical = Metrics.gapHalf)) {
            DiffRows(display, tokens)
          }
        }
      }
    }
  }
}

@Composable
private fun DiffRows(display: List<DiffLine>, tokens: List<List<CodeToken>?>? = null) {
  val density = LocalDensity.current
  val ch = with(density) { (WebType.mono.fontSize.toPx() * WebType.ch).toDp() }
  val number = ch * NUMBER_CH
  val sign = ch * SIGN_CH
  val line = Palette.line
  val hairPx = with(density) { Metrics.hairline.toPx() }
  display.forEachIndexed { i, d ->
    if (d.kind == DiffLine.Kind.hunk) {
      // A leading or trailing hunk header is not drawn
      if (i == 0 || i == display.size - 1) return@forEachIndexed
      Box(
        Modifier.fillMaxWidth().padding(vertical = Metrics.gapHalf).height(WebMetrics.diffHunk).drawBehind {
          drawRect(line, size = Size(size.width, hairPx))
          drawRect(line, topLeft = Offset(0f, size.height - hairPx), size = Size(size.width, hairPx))
        },
        contentAlignment = Alignment.CenterStart,
      ) {
        Text("···", color = Palette.fg3, style = WebType.text3, textAlign = TextAlign.Center, modifier = Modifier.width(number + sign))
      }
    } else {
      val tint = when (d.kind) {
        DiffLine.Kind.add -> Palette.diffAdd
        DiffLine.Kind.del -> Palette.diffDel
        else -> Color.Transparent
      }
      Row(
        Modifier.fillMaxWidth().heightIn(min = WebMetrics.codeOutputLine).background(tint),
        verticalAlignment = Alignment.CenterVertically,
      ) {
        Text(
          ((if (d.kind == DiffLine.Kind.del) d.oldLine else d.newLine)?.toString()).orEmpty(),
          color = Palette.fg3, style = WebType.codeOutput.copy(fontFeatureSettings = TABULAR), textAlign = TextAlign.End,
          maxLines = 1, softWrap = false, modifier = Modifier.widthIn(min = number),
        )
        Text(
          when (d.kind) { DiffLine.Kind.add -> "+"; DiffLine.Kind.del -> "−"; else -> " " },
          color = when (d.kind) { DiffLine.Kind.add -> Palette.ok; DiffLine.Kind.del -> Palette.danger; else -> Palette.fg3 },
          style = WebType.codeOutput, textAlign = TextAlign.Center, maxLines = 1, modifier = Modifier.width(sign),
        )
        val runs = tokens?.getOrNull(i)
        Text(
          if (runs != null && diffCode(d).isNotEmpty()) codeRuns(runs) else AnnotatedString(diffCode(d).ifEmpty { "​" }),
          color = Palette.fg1, style = WebType.codeOutput, maxLines = 1, softWrap = false,
          modifier = Modifier.padding(end = Metrics.pad),
        )
      }
    }
  }
}

private const val NUMBER_CH = 3.7f
private const val SIGN_CH = 2.2f

/** `DiffLine.text` carries its sign column (transcript/diff.rs): the gutter draws the sign, the code drops it */
private fun diffCode(line: DiffLine) = if (line.kind == DiffLine.Kind.hunk) line.text else line.text.drop(1)

/** Enough rows to paint past the transcript's cut, including a leading hunk header (which is not drawn) */
private fun diffPreview(lines: List<DiffLine>): List<DiffLine> {
  val shortest = minOf(WebMetrics.codeOutputLine.value, (WebMetrics.diffHunk + Metrics.gapHalf * 2).value)
  val limit = ceil(WebMetrics.codeOutputMax.value / shortest).toInt() + 1
  return lines.take(limit + if (lines.firstOrNull()?.kind == DiffLine.Kind.hunk) 1 else 0)
}

/** Where a diff sits in the session: its tool call and which of the call's diffs (CodeBlock.tsx DiffLocation) */
data class DiffLocation(val toolCallId: String, val nth: Int)

/**
 * The location of this diff, counted among the call's diffs (`contents`, or `content` alone). Found by identity first:
 * two files given identical hunks compare equal by their lines, and the second would ask for the first one's source
 */
private fun diffLocation(block: ToolCallBlock, diff: ToolContent.Diff): DiffLocation? {
  val diffs = (block.contents ?: listOfNotNull(block.content)).filterIsInstance<ToolContent.Diff>()
  val at = diffs.indexOfFirst { it === diff }.takeIf { it >= 0 } ?: diffs.indexOfFirst { it == diff }
  return if (at < 0) null else DiffLocation(block.id, at)
}

/** Asks the engine for a diff's whole files in the transcript's session (null: no engine to ask) */
val LocalDiffSource = compositionLocalOf<(suspend (DiffLocation) -> FullDiffSource?)?> { null }

/** codeDiff.ts diffCopyText: the new file when the diff carries it, else the visible new code without signs */
private fun diffCopyText(lines: List<DiffLine>, source: DiffSource?): String {
  if (source != null && source.omitted != true && source.newText != null) return source.newText!!
  return lines.filter { it.kind == DiffLine.Kind.add || it.kind == DiffLine.Kind.ctx }.joinToString("\n") { diffCode(it) }
}

// Helpers (Folding.swift, MediaLoader.swift)

private fun outputOf(b: ToolCallBlock): String {
  val texts = b.contents.orEmpty().filterIsInstance<ToolContent.Text>()
  if (texts.isNotEmpty()) return texts.joinToString("\n") { it.text }
  return when (val c = b.content) {
    is ToolContent.Text -> c.text
    is ToolContent.ListItems -> c.items.joinToString("\n")
    is ToolContent.Diff -> c.lines.joinToString("\n") { diffCode(it) }
    else -> ""
  }
}

/** imageTools.ts isImageResultBlock: generated and shown images, and widgets, are results for the reader */
private fun isImageResult(b: ToolCallBlock) = b.verbKey in IMAGE_VERBS

private val IMAGE_VERBS = setOf("verb.imagegen", "verb.showImage", "verb.showVideo", "verb.showWidget")

private val RANGE_SUFFIX = Regex("""^(.+?):(\d+(?:[–-]\d+)?)(?::\d+)?$""")
private val EXPLICIT_PATH = Regex("""^(?:\.{0,2}/|[A-Za-z]:[\\/])""")

/** splitHit: a reference as the name shown and its faint line note */
fun splitHit(hit: String): Pair<String, String?> {
  RANGE_SUFFIX.find(hit)?.let { return lastComponent(it.groupValues[1]) to "L${it.groupValues[2]}" }
  if (EXPLICIT_PATH.containsMatchIn(hit)) return lastComponent(hit) to null
  val uri = runCatching { java.net.URI(hit) }.getOrNull()
  val host = uri?.host
  if (uri?.scheme != null && host != null) {
    val path = uri.path.orEmpty()
    return (if (path.isEmpty() || path == "/") host else host + path) to host
  }
  return hit to null
}

/** editEntries: consecutive edits of one file share an entry */
private fun editEntries(blocks: List<ToolCallBlock>): List<Pair<String, List<ToolCallBlock>>> {
  val map = LinkedHashMap<String, MutableList<ToolCallBlock>>()
  for (b in blocks) map.getOrPut(filePath(b).ifEmpty { b.id }) { ArrayList() } += b
  return map.map { it.key to it.value }
}

/** editSpan: first to last changed line, e.g. L535–549 */
private fun editSpan(blocks: List<ToolCallBlock>): String? {
  var lo = Int.MAX_VALUE
  var hi = Int.MIN_VALUE
  for (b in blocks) {
    for (c in visibleToolContents(b)) {
      if (c !is ToolContent.Diff) continue
      val adds = c.lines.filter { it.kind == DiffLine.Kind.add }.mapNotNull { it.newLine }
      for (n in adds.ifEmpty { c.lines.filter { it.kind == DiffLine.Kind.del }.mapNotNull { it.oldLine } }) {
        lo = minOf(lo, n)
        hi = maxOf(hi, n)
      }
    }
  }
  if (lo == Int.MAX_VALUE) return null
  return if (lo == hi) "L$lo" else "L$lo–$hi"
}

private fun diffStatOf(blocks: List<ToolCallBlock>): DiffStat? {
  if (blocks.none { it.diffStat != null }) return null
  return DiffStat(blocks.sumOf { it.diffStat?.add ?: 0 }, blocks.sumOf { it.diffStat?.del ?: 0 })
}
