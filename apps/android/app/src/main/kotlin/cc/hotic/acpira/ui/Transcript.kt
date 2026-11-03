package cc.hotic.acpira.ui

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.ui.graphics.Color
import cc.hotic.acpira.logic.permissionOption
import cc.hotic.acpira.logic.planApprovalTitle
import android.content.ClipData
import android.content.ClipboardManager
import androidx.compose.ui.layout.layout
import androidx.compose.material3.IconButton
import androidx.compose.ui.platform.LocalContext
import kotlinx.coroutines.delay
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.logic.GroupKind
import cc.hotic.acpira.logic.LiveActivity
import cc.hotic.acpira.logic.ProcessItem
import cc.hotic.acpira.logic.clockStart
import cc.hotic.acpira.logic.durationText
import cc.hotic.acpira.logic.elapsedLabel
import cc.hotic.acpira.logic.groupNames
import cc.hotic.acpira.logic.groupProcess
import cc.hotic.acpira.logic.liveActivity
import cc.hotic.acpira.logic.namesLabel
import cc.hotic.acpira.logic.retiredProcess
import cc.hotic.acpira.logic.splitCodexBlocks
import cc.hotic.acpira.logic.toolIcon
import cc.hotic.acpira.logic.toolVerb
import cc.hotic.acpira.logic.turnOutcome
import cc.hotic.acpira.logic.visibleToolContents
import cc.hotic.acpira.model.AgentBlock
import cc.hotic.acpira.model.AgentTurn
import cc.hotic.acpira.model.Attachment
import cc.hotic.acpira.model.CompactionBlock
import cc.hotic.acpira.model.DiffLine
import cc.hotic.acpira.model.GoalBlock
import cc.hotic.acpira.model.NoticeBlock
import cc.hotic.acpira.model.PermissionBlock
import cc.hotic.acpira.model.PermissionKind
import cc.hotic.acpira.model.PlanBlock
import cc.hotic.acpira.model.PlanStatus
import cc.hotic.acpira.model.QuestionBlock
import cc.hotic.acpira.model.SteerBlock
import cc.hotic.acpira.model.TextBlock
import cc.hotic.acpira.model.ThoughtBlock
import cc.hotic.acpira.model.ToolCallBlock
import cc.hotic.acpira.model.ToolContent
import cc.hotic.acpira.model.ToolKind
import cc.hotic.acpira.model.ToolStatus
import cc.hotic.acpira.model.UnknownBlock
import cc.hotic.acpira.model.UserTurn

// The transcript's pieces (apps/ios Transcript/, chat/Turns.tsx under fold = codex): the prompt as a right-hand bubble,
// each agent turn as one process fold whose head carries the live activity or the finished time, the reply outside
// it, permission cards after the reply, the outcome row last.

/** The prompt: a right-aligned bubble that always leaves `bubbleInset` free on its left; attachments ride above it */
@Composable
fun UserBubble(turn: UserTurn, pending: Boolean = false) {
  if (turn.auto == true) return
  Column(
    Modifier.fillMaxWidth().padding(start = Metrics.bubbleInset, end = Metrics.pad).alpha(if (pending) 0.55f else 1f),
    horizontalAlignment = Alignment.End,
    verticalArrangement = Arrangement.spacedBy(Metrics.s2),
  ) {
    val attachments = turn.attachments.orEmpty()
    if (attachments.isNotEmpty()) {
      Row(horizontalArrangement = Arrangement.spacedBy(Metrics.s2), modifier = Modifier.horizontalScroll(rememberScrollState())) {
        for (a in attachments) AttachmentChip(a)
      }
    }
    if (turn.text.isNotBlank()) {
      Box(
        Modifier.background(Palette.bubbleBg, RoundedCornerShape(Metrics.rBubble))
          .padding(horizontal = Metrics.pad + Metrics.s1, vertical = Metrics.gap)
          .testTag("bubble"),
      ) {
        Text(turn.text, color = Palette.fg1, fontSize = Type.body, lineHeight = Type.bodyLine)
      }
    }
  }
}

@Composable
private fun AttachmentChip(a: Attachment) {
  val (icon, label) = when (a) {
    is Attachment.Image -> "image" to (a.name ?: "")
    is Attachment.Text -> "fileText" to a.name
    is Attachment.File -> "file" to a.name
    is Attachment.Selection -> "fileCode" to "${a.name}:${a.startLine}"
    is Attachment.Quote -> "messageSquareText" to a.text.take(QUOTE_CHIP)
    is Attachment.Unknown -> "file" to a.kind
  }
  Row(
    verticalAlignment = Alignment.CenterVertically,
    horizontalArrangement = Arrangement.spacedBy(Metrics.s1_5),
    modifier = Modifier.background(Palette.chip, RoundedCornerShape(Metrics.rMd)).padding(horizontal = Metrics.gap, vertical = Metrics.s1_5),
  ) {
    Lucide(icon, Palette.fg2, size = Metrics.icon)
    Text(label, color = Palette.fg2, fontSize = Type.text2, maxLines = 1, overflow = TextOverflow.Ellipsis)
  }
}

/** One agent turn: the process fold, notices, the reply, permission cards, the outcome */
@Composable
fun AgentMessage(
  turn: AgentTurn,
  running: Boolean,
  key: String,
  now: () -> Long,
  onPermission: (blockId: String, optionId: String) -> Unit,
) {
  val split = remember(turn) { splitCodexBlocks(turn.blocks) }
  Column(Modifier.fillMaxWidth().padding(horizontal = Metrics.pad + Metrics.threadInset), verticalArrangement = Arrangement.spacedBy(Metrics.gap)) {
    for (goal in split.goalsLead) GoalRow(goal)
    ProcessFold(turn, split.process, running, key, now)
    for (n in split.notices) NoticeRow(n)
    for (reply in split.reply) {
      if (reply.markdown.isNotBlank()) Prose(reply.markdown, Modifier.testTag("reply"))
    }
    for (goal in split.goals) GoalRow(goal)
    for (p in split.permissions.filter { it.planId == null }) PermissionCard(p) { onPermission(p.id, it) }
    if (!running) turnOutcome(turn)?.let { OutcomeRow(it) }
    if (!running) TurnActions(turn)
  }
}

/**
 * The reply's actions under it (Cards.swift TurnActions), hugging the reply: copy for now. Android 13+ confirms a copy
 * itself, so the glyph only turns into a tick for a moment
 */
@Composable
private fun TurnActions(turn: AgentTurn) {
  val reply = remember(turn) { replyMarkdown(turn) }
  if (reply.isEmpty()) return
  val context = LocalContext.current
  var copied by remember { mutableStateOf(false) }
  LaunchedEffect(copied) {
    if (copied) { delay(COPIED_MS); copied = false }
  }
  // Hugs the reply (-mt-gap): the row gives a gap of its height back to the column's spacing
  Row(
    Modifier.fillMaxWidth().layout { measurable, constraints ->
      val row = measurable.measure(constraints)
      val pull = Metrics.gap.roundToPx()
      layout(row.width, maxOf(row.height - pull, 0)) { row.place(0, -pull) }
    },
    horizontalArrangement = Arrangement.End,
  ) {
    IconButton(
      onClick = {
        context.getSystemService(ClipboardManager::class.java)?.setPrimaryClip(ClipData.newPlainText(t("turn.copy"), reply))
        copied = true
      },
      modifier = Modifier.size(Metrics.ctlSm).testTag("turn.copy"),
    ) {
      Lucide(if (copied) "check" else "copy", Palette.fg2, size = Metrics.chevron, contentDescription = t(if (copied) "code.copied" else "turn.copy"))
    }
  }
}

/** The text a copy takes: the turn's final prose, else its plain text blocks */
private fun replyMarkdown(turn: AgentTurn): String {
  val reply = splitCodexBlocks(turn.blocks).reply.joinToString("\n\n") { it.markdown }.trim()
  if (reply.isNotEmpty()) return reply
  return turn.blocks.filterIsInstance<TextBlock>().joinToString("\n\n") { it.markdown }.trim()
}

private const val COPIED_MS = 1500L

/**
 * The process fold: open while the turn works, closed once it is done; a tap on the head always wins (Turns.tsx
 * CodexFold). A turn with nothing but a reply shows no head
 */
@Composable
private fun ProcessFold(turn: AgentTurn, blocks: List<AgentBlock>, running: Boolean, key: String, now: () -> Long) {
  if (retiredProcess(running, turn, blocks)) return
  var manual by rememberSaveable(key) { mutableStateOf<Boolean?>(null) }
  val open = manual ?: running
  val items = remember(blocks) { groupProcess(blocks) }
  Column(Modifier.fillMaxWidth()) {
    FoldHead(turn, running, open, foldable = items.isNotEmpty(), now = now) { manual = !open }
    AnimatedVisibility(open && items.isNotEmpty(), enter = expandVertically() + fadeIn(), exit = shrinkVertically() + fadeOut()) {
      Column(Modifier.padding(top = Metrics.s1, bottom = Metrics.s1_5)) {
        for (item in items) ProcessItemView(item)
      }
    }
  }
}

/** The fold's head: the orb (or hand, plan, question) and the activity while live; a tick and the time once done */
@Composable
private fun FoldHead(turn: AgentTurn, running: Boolean, open: Boolean, foldable: Boolean, now: () -> Long, onToggle: () -> Unit) {
  val activity = liveActivity(turn)
  val outcome = turnOutcome(turn)
  val label = if (running) activity.label else outcome ?: t("turns.done")
  Row(
    verticalAlignment = Alignment.CenterVertically,
    modifier = Modifier.fillMaxWidth().heightIn(min = Metrics.chatRow).clickable(enabled = foldable, onClick = onToggle).testTag("fold.head"),
  ) {
    Box(Modifier.width(Metrics.lead + Metrics.gap), contentAlignment = Alignment.CenterStart) {
      when {
        !running -> Lucide("check", Palette.fg3, size = Metrics.icon)
        activity.lead == LiveActivity.Lead.hand -> Lucide("hand", Palette.fg3, size = Metrics.icon)
        activity.lead == LiveActivity.Lead.question -> Lucide("messageCircleQuestion", Palette.fg3, size = Metrics.icon)
        activity.lead == LiveActivity.Lead.plan -> Lucide("listTodo", Palette.fg3, size = Metrics.icon)
        else -> Orb()
      }
    }
    if (running && activity.active) Shimmer(label) else Text(label, color = Palette.fg2, fontSize = Type.text1, lineHeight = Type.text2Line)
    val time = if (running) liveClock(turn, now) else if (outcome == null) elapsedLabel(turn) else ""
    if (time.isNotEmpty()) {
      Spacer(Modifier.width(Metrics.s2))
      Text(time, color = Palette.fg3, fontSize = Type.text1, lineHeight = Type.text2Line)
    }
    if (foldable) {
      Spacer(Modifier.width(Metrics.s1_5))
      Lucide("chevronRight", Palette.fg3, Modifier.rotate(if (open) 90f else 0f), size = Metrics.chevron)
    }
  }
}

/** The live turn's clock: hidden for its first second, then whole seconds */
@Composable
private fun liveClock(turn: AgentTurn, now: () -> Long): String {
  val start = clockStart(turn) ?: return ""
  var tick by remember { mutableLongStateOf(now()) }
  LaunchedEffect(start) {
    while (true) {
      tick = now()
      kotlinx.coroutines.delay(CLOCK_TICK_MS)
    }
  }
  val ms = tick - start
  return if (ms < CLOCK_TICK_MS) "" else durationText(ms.toDouble())
}

private const val CLOCK_TICK_MS = 1000L

/** The working orb: a ring turning once a second, the head's live lead */
@Composable
fun Orb(size: Dp = Metrics.icon) {
  val loop = rememberInfiniteTransition(label = "orb")
  val turn by loop.animateFloat(0f, 360f, infiniteRepeatable(tween(1000, easing = LinearEasing)), label = "spin")
  Lucide("loaderCircle", Palette.fg2, Modifier.rotate(turn), size = size)
}

/** A live label: its ink breathes between the shimmer base and the full ink (ui/Shimmer) */
@Composable
fun Shimmer(text: String) {
  val loop = rememberInfiniteTransition(label = "shimmer")
  val a by loop.animateFloat(0.45f, 1f, infiniteRepeatable(tween(900), androidx.compose.animation.core.RepeatMode.Reverse), label = "ink")
  Text(text, color = Palette.fg2, fontSize = Type.text1, lineHeight = Type.text2Line, modifier = Modifier.alpha(a))
}

/** One process item: a block row, or a group of reads / edits under one head */
@Composable
private fun ProcessItemView(item: ProcessItem) {
  when (item) {
    is ProcessItem.Group -> {
      var open by rememberSaveable(item.id) { mutableStateOf(false) }
      val first = item.blocks.first()
      ProcessRow(
        icon = if (item.kind == GroupKind.read) "files" else "pencil",
        verb = toolVerb(first),
        target = namesLabel(groupNames(item.blocks)),
        mono = false,
        onTap = { open = !open },
      )
      AnimatedVisibility(open) {
        Column(Modifier.padding(start = Metrics.lead + Metrics.gap)) {
          for (b in item.blocks) ToolRow(b)
        }
      }
    }
    is ProcessItem.Block -> when (val b = item.block) {
      is ToolCallBlock -> ToolRow(b)
      is ThoughtBlock -> ThoughtRow(b)
      is PlanBlock -> PlanList(b)
      is TextBlock -> if (b.markdown.isNotBlank()) Prose(b.markdown, Modifier.padding(vertical = Metrics.s1_5), color = Palette.fg2)
      is CompactionBlock -> ProcessRow(
        "chevronsDownUp",
        t(when (b.status) {
          cc.hotic.acpira.model.CompactionStatus.in_progress -> "turns.compacting"
          cc.hotic.acpira.model.CompactionStatus.completed -> "turns.compacted"
          cc.hotic.acpira.model.CompactionStatus.failed -> "turns.compactFailed"
          cc.hotic.acpira.model.CompactionStatus.cancelled -> "turns.compactCancelled"
        }),
        null, false, running = b.status == cc.hotic.acpira.model.CompactionStatus.in_progress,
      )
      is SteerBlock -> Box(Modifier.padding(vertical = Metrics.processProseGap)) { UserBubble(UserTurn(text = b.text)) }
      is QuestionBlock -> ProcessRow("messageCircleQuestion", t("question.title"), b.questions.firstOrNull()?.text, false)
      is GoalBlock -> GoalRow(b)
      is UnknownBlock -> {}
      else -> {}
    }
  }
}

/** One tool call: its row, and its output under it once tapped open */
@Composable
private fun ToolRow(block: ToolCallBlock) {
  var open by rememberSaveable(block.id) { mutableStateOf(false) }
  val contents = visibleToolContents(block)
  val target = block.mcpServer?.let { "${it} ${block.target.orEmpty()}" } ?: block.target
  ProcessRow(
    icon = toolIcon(block),
    verb = toolVerb(block),
    target = target,
    mono = block.targetMono == true,
    failed = block.status == ToolStatus.failed,
    running = block.status.live,
    onTap = if (contents.isNotEmpty()) ({ open = !open }) else null,
  )
  AnimatedVisibility(open) {
    Column(Modifier.padding(start = Metrics.lead + Metrics.gap, top = Metrics.gapHalf, bottom = Metrics.gapHalf), verticalArrangement = Arrangement.spacedBy(Metrics.s2)) {
      for (c in contents) ToolOutput(c)
    }
  }
}

@Composable
private fun ToolOutput(content: ToolContent) {
  when (content) {
    is ToolContent.Text -> CodeCard(content.text.trimEnd(), Modifier.heightIn(max = Metrics.codeOutputMax))
    is ToolContent.Diff -> DiffCard(content.lines)
    is ToolContent.ListItems -> CodeCard(content.items.joinToString("\n"), Modifier.heightIn(max = Metrics.codeOutputMax))
    is ToolContent.Image, is ToolContent.Unknown -> {}
  }
}

/** A diff: added lines on the ok wash, removed on the danger wash, hunks faint */
@Composable
private fun DiffCard(lines: List<DiffLine>) {
  Column(
    Modifier.fillMaxWidth().heightIn(max = Metrics.codeOutputMax).background(Palette.bgCode, RoundedCornerShape(Metrics.rLg))
      .horizontalScroll(rememberScrollState()).padding(vertical = Metrics.s2),
  ) {
    for (line in lines.take(DIFF_PREVIEW_LINES)) {
      val (bg, fg, sign) = when (line.kind) {
        DiffLine.Kind.add -> Triple(Palette.diffAdd, Palette.ok, "+")
        DiffLine.Kind.del -> Triple(Palette.diffDel, Palette.danger, "-")
        DiffLine.Kind.hunk -> Triple(androidx.compose.ui.graphics.Color.Transparent, Palette.fg3, "")
        DiffLine.Kind.ctx -> Triple(androidx.compose.ui.graphics.Color.Transparent, Palette.fg2, " ")
      }
      Text(
        "$sign${line.text}", color = fg, fontFamily = FontFamily.Monospace, fontSize = Type.codeOutput, lineHeight = Type.codeOutputLine,
        softWrap = false, modifier = Modifier.background(bg).padding(horizontal = Metrics.pad),
      )
    }
  }
}

private const val DIFF_PREVIEW_LINES = 80

/**
 * ui/Row under process rows: the kind's icon in the lead slot, the verb in the row ink, the target after it (mono for
 * paths and commands), one line, truncated
 */
@Composable
private fun ProcessRow(
  icon: String,
  verb: String,
  target: String?,
  mono: Boolean,
  failed: Boolean = false,
  running: Boolean = false,
  onTap: (() -> Unit)? = null,
) {
  Row(
    verticalAlignment = Alignment.CenterVertically,
    modifier = Modifier.fillMaxWidth().heightIn(min = Metrics.processRow)
      .then(if (onTap != null) Modifier.clickable(onClick = onTap) else Modifier)
      .testTag("row"),
  ) {
    Box(Modifier.width(Metrics.lead + Metrics.gap), contentAlignment = Alignment.CenterStart) {
      Lucide(icon, if (failed) Palette.danger else Palette.fg3, size = Metrics.icon)
    }
    Text(
      buildString {
        append(verb)
      },
      color = if (failed) Palette.danger else Palette.fg2, fontSize = Type.text1, lineHeight = Type.text2Line, maxLines = 1,
      modifier = if (running) Modifier.alpha(RUNNING_ALPHA) else Modifier,
    )
    if (!target.isNullOrBlank()) {
      Spacer(Modifier.width(Metrics.s2))
      Text(
        target.replace('\n', ' '), color = Palette.fg2, fontSize = if (mono) Type.mono else Type.text1,
        fontFamily = if (mono) FontFamily.Monospace else null, lineHeight = Type.text2Line,
        maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f, fill = false),
      )
    }
  }
}

private const val RUNNING_ALPHA = 0.75f
private const val QUOTE_CHIP = 24

/** A thought: its row reads 思考, a tap shows the reasoning under it */
@Composable
private fun ThoughtRow(block: ThoughtBlock) {
  var open by rememberSaveable(block.startedAt ?: block.text.hashCode()) { mutableStateOf(false) }
  ProcessRow("brain", t("thought.label"), null, false, running = block.streaming == true, onTap = if (block.text.isNotBlank()) ({ open = !open }) else null)
  AnimatedVisibility(open) {
    Text(
      block.text, color = Palette.fg3, fontSize = Type.text2, lineHeight = Type.text2Line,
      modifier = Modifier.padding(start = Metrics.lead + Metrics.gap, bottom = Metrics.s2),
    )
  }
}

/** The agent's to-do list: one row per entry with its status mark */
@Composable
private fun PlanList(block: PlanBlock) {
  Column(Modifier.padding(vertical = Metrics.s1)) {
    for (e in block.entries) {
      Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.heightIn(min = Metrics.rowDense)) {
        Box(Modifier.width(Metrics.lead + Metrics.gap), contentAlignment = Alignment.CenterStart) {
          when (e.status) {
            PlanStatus.completed -> Lucide("circleCheck", Palette.ok, size = Metrics.icon)
            PlanStatus.in_progress -> Lucide("circleDot", Palette.fg1, size = Metrics.icon)
            PlanStatus.pending -> Lucide("circle", Palette.fg3, size = Metrics.icon)
          }
        }
        Text(
          e.title, color = if (e.status == PlanStatus.completed) Palette.fg3 else Palette.fg1, fontSize = Type.text2, lineHeight = Type.text2Line,
        )
      }
    }
  }
}

@Composable
private fun GoalRow(block: GoalBlock) {
  ProcessRow("goal", t("goal.event.${block.event.name}"), block.objective, false)
}

@Composable
private fun NoticeRow(block: NoticeBlock) {
  val tint = when {
    block.severity == "error" -> Palette.danger
    block.info == true -> Palette.fg3
    else -> Palette.warn
  }
  Row(verticalAlignment = Alignment.Top, modifier = Modifier.fillMaxWidth().padding(vertical = Metrics.s1)) {
    Box(Modifier.width(Metrics.lead + Metrics.gap).heightIn(min = Metrics.s6), contentAlignment = Alignment.CenterStart) {
      Lucide(if (block.severity == "error") "circleAlert" else "info", tint, size = Metrics.icon)
    }
    Column {
      Text(block.title, color = Palette.fg1, fontSize = Type.text2, lineHeight = Type.text2Line)
      block.details?.takeIf { it.isNotBlank() }?.let { Text(it, color = Palette.fg3, fontSize = Type.text3, lineHeight = Type.text3Line, maxLines = NOTICE_LINES, overflow = TextOverflow.Ellipsis) }
    }
  }
}

private const val NOTICE_LINES = 4

@Composable
private fun OutcomeRow(text: String) {
  ProcessRow("ban", text, null, false)
}

/**
 * A permission request (Cards.swift PermissionCard): the request's title and command on a card, Allow once as the
 * filled action, Reject beside it, the other grants in a menu
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun PermissionCard(block: PermissionBlock, onPick: (String) -> Unit) {
  val options = block.options.map { permissionOption(it, plan = block.planId != null) }
  // Several allow_once and no allow_always: a list of answers, every option an equal button
  val flat = options.none { it.kind == PermissionKind.allow_always } && options.count { it.kind == PermissionKind.allow_once } > 1
  val once = if (flat) null else options.firstOrNull { it.kind == PermissionKind.allow_once }
  val reject = if (flat) null else options.firstOrNull { it.kind == PermissionKind.reject_once }
  val more = if (flat) emptyList() else options.filter { it.id != once?.id && it.id != reject?.id }
  val title = if (block.command != null) t("permission.run") else planApprovalTitle(block.title)
  val noFirst = block.defaultToNo == true
  Column(
    Modifier.fillMaxWidth()
      .background(Palette.bg1, RoundedCornerShape(Metrics.rLg))
      .border(Metrics.hairline, Palette.conversationLine, RoundedCornerShape(Metrics.rLg))
      .padding(Metrics.pad + Metrics.hairline)
      .testTag("permission"),
    verticalArrangement = Arrangement.spacedBy(Metrics.gap),
  ) {
    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(Metrics.gap)) {
      if (block.command != null) Lucide("terminal", Palette.fg2, size = Metrics.icon)
      Text(title, color = Palette.fg2, fontSize = Type.text2, fontWeight = FontWeight.Medium, maxLines = 2, overflow = TextOverflow.Ellipsis)
    }
    block.command?.let { Text(it, color = Palette.fg1, fontFamily = FontFamily.Monospace, fontSize = Type.mono, lineHeight = Type.monoLine) }
    block.description?.takeIf { it.isNotBlank() }?.let { Text(it, color = Palette.fg2, fontSize = Type.text3, lineHeight = Type.text3Line) }
    if (flat) {
      FlowRow(horizontalArrangement = Arrangement.spacedBy(Metrics.gap), verticalArrangement = Arrangement.spacedBy(Metrics.gap)) {
        for (o in options) PillButton(o.label, onClick = { onPick(o.id) })
      }
    } else {
      // The extra grants left, the answers pushed right, one gap between them at the least
      Row(verticalAlignment = Alignment.CenterVertically) {
        if (more.isNotEmpty()) {
          var menu by remember { mutableStateOf(false) }
          Box {
            PillButton(t("permission.more"), onClick = { menu = true }, modifier = Modifier.testTag("permission.more"), oneLine = true) {
              Lucide("chevronDown", Palette.fg2, size = Metrics.chevron)
            }
            DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
              for (o in more.filter { !it.bypass } + more.filter { it.bypass }) {
                DropdownMenuItem(
                  text = {
                    Column {
                      Text(o.label, fontSize = Type.text2)
                      o.detail?.let { Text(it, color = Palette.fg3, fontSize = Type.text3) }
                    }
                  },
                  onClick = { menu = false; onPick(o.id) },
                )
              }
            }
          }
        }
        Spacer(Modifier.weight(1f).widthIn(min = if (more.isEmpty()) Metrics.s0 else Metrics.gap))
        Row(horizontalArrangement = Arrangement.spacedBy(Metrics.gap)) {
          reject?.let { PillButton(it.label, primary = noFirst, onClick = { onPick(it.id) }, modifier = Modifier.testTag("permission.reject")) }
          once?.let { PillButton(it.label, primary = !noFirst, onClick = { onPick(it.id) }, modifier = Modifier.testTag("permission.allow")) }
        }
      }
    }
  }
}

/**
 * Components/Buttons.swift AcpButtonStyle: a capsule with a hairline, text-2; the primary answer takes the active fill
 * and the strong ink. [oneLine]: a menu trigger, exactly --ctl tall; otherwise the answer form, whose label may wrap
 */
@Composable
fun PillButton(
  label: String,
  onClick: () -> Unit,
  modifier: Modifier = Modifier,
  primary: Boolean = false,
  oneLine: Boolean = false,
  trailing: (@Composable () -> Unit)? = null,
) {
  OutlinedButton(
    onClick = onClick,
    shape = CircleShape,
    border = BorderStroke(Metrics.hairline, if (primary) Palette.lineStrong else Palette.line),
    colors = ButtonDefaults.outlinedButtonColors(
      containerColor = if (primary) Palette.active else Color.Transparent,
      contentColor = if (primary) Palette.fgStrong else Palette.fg2,
    ),
    contentPadding = PaddingValues(horizontal = Metrics.s3 + Metrics.hairline, vertical = if (oneLine) Metrics.s0 else Metrics.gapHalf + Metrics.hairline),
    modifier = modifier.heightIn(min = Metrics.ctl),
  ) {
    Text(
      label, fontSize = Type.text2, fontWeight = if (primary) FontWeight.Medium else FontWeight.Normal,
      maxLines = if (oneLine) 1 else Int.MAX_VALUE, softWrap = !oneLine,
    )
    trailing?.let { Spacer(Modifier.width(Metrics.s1_5)); it() }
  }
}

