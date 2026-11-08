package cc.hotic.acpira.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
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
import android.content.Context
import android.os.Build
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.height
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTagsAsResourceId
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.TextStyle
import androidx.compose.material3.LocalTextStyle
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntRect
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.window.Popup
import androidx.compose.ui.window.PopupPositionProvider
import androidx.compose.ui.window.PopupProperties
import cc.hotic.acpira.logic.hasTurnContent
import cc.hotic.acpira.logic.modelLabel
import cc.hotic.acpira.logic.showsTurnActions
import cc.hotic.acpira.logic.turnStatsContent
import cc.hotic.acpira.model.SessionControls
import cc.hotic.acpira.model.TurnSettings
import androidx.compose.ui.layout.layout
import androidx.compose.material3.IconButton
import androidx.compose.ui.platform.LocalContext
import kotlinx.coroutines.delay
import androidx.compose.foundation.background
import androidx.compose.foundation.border
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
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.logic.LiveActivity
import cc.hotic.acpira.logic.ProcessItem
import cc.hotic.acpira.logic.clockStart
import cc.hotic.acpira.logic.elapsedLabel
import cc.hotic.acpira.logic.groupProcess
import cc.hotic.acpira.logic.PlanSection
import cc.hotic.acpira.logic.splitPlanSections
import cc.hotic.acpira.logic.liveActivity
import cc.hotic.acpira.logic.splitCodexBlocks
import cc.hotic.acpira.logic.toolVerb
import cc.hotic.acpira.logic.turnOutcome
import cc.hotic.acpira.model.AgentBlock
import cc.hotic.acpira.model.AgentTurn
import cc.hotic.acpira.model.Attachment
import cc.hotic.acpira.model.CompactionBlock
import cc.hotic.acpira.model.PermissionBlock
import cc.hotic.acpira.model.PermissionKind
import cc.hotic.acpira.model.SteerBlock
import cc.hotic.acpira.model.TextBlock
import cc.hotic.acpira.model.ThoughtBlock
import cc.hotic.acpira.model.ToolCallBlock
import cc.hotic.acpira.model.ToolStatus
import cc.hotic.acpira.model.UserTurn
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.material3.LocalContentColor
import cc.hotic.acpira.design.WebMetrics
import cc.hotic.acpira.model.TurnStop

// The transcript's pieces (apps/ios Transcript/, chat/Turns.tsx under fold = codex): the prompt as a right-hand bubble,
// each agent turn as one process fold whose head carries the live activity or the finished time, the reply outside
// it, permission cards after the reply, the outcome row last.

/**
 * The prompt: a right-aligned bubble that always leaves `bubbleInset` free on its left; attachments ride above it. A
 * long press opens its menu (Cards.swift UserMessageCard) when [onToast] is given: Edit and resend ([onEdit], while the
 * session is idle), Copy, and Set as goal ([onGoal], on the newest typed prompt where the agent takes a goal)
 */
@Composable
fun UserBubble(
  turn: UserTurn,
  pending: Boolean = false,
  onToast: ((String) -> Unit)? = null,
  onEdit: (() -> Unit)? = null,
  onGoal: (() -> Unit)? = null,
) {
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
      val context = LocalContext.current
      val haptics = LocalHapticFeedback.current
      var menu by remember { mutableStateOf(false) }
      Box {
        Box(
          Modifier.background(Palette.bubbleBg, RoundedCornerShape(Metrics.rBubble))
            .then(if (onToast == null) Modifier else Modifier.pointerInput(Unit) {
              detectTapGestures(onLongPress = { haptics.performHapticFeedback(HapticFeedbackType.LongPress); menu = true })
            })
            .padding(horizontal = Metrics.pad + Metrics.s1, vertical = Metrics.gap)
            .testTag("bubble"),
        ) {
          Text(turn.text, color = Palette.fg1, fontSize = Type.body, lineHeight = Type.bodyLine)
        }
        AppMenu(expanded = menu, onDismiss = { menu = false }) {
          onEdit?.let { edit ->
            AppMenuItem(
              t("history.edit"), onClick = { menu = false; edit() }, modifier = Modifier.testTag("bubble.edit"), icon = { MenuGlyph("pencil") },
            )
          }
          AppMenuItem(
            t("common.copy"),
            onClick = {
              menu = false
              if (copyText(context, turn.text.trimEnd())) onToast?.invoke(t("code.copied"))
            },
            modifier = Modifier.testTag("bubble.copy"),
            icon = { MenuGlyph("copy") },
          )
          onGoal?.let { goal ->
            AppMenuItem(
              t("goal.setFromPrompt"), onClick = { menu = false; goal() }, modifier = Modifier.testTag("bubble.goal"), icon = { MenuGlyph("goal") },
            )
          }
        }
      }
    }
  }
}

/** A queued prompt's attachments, laid out as the sent prompt's above its bubble */
@Composable
internal fun BubbleAttachments(attachments: List<Attachment>) {
  Row(horizontalArrangement = Arrangement.spacedBy(Metrics.s2), modifier = Modifier.horizontalScroll(rememberScrollState())) {
    for (a in attachments) AttachmentChip(a)
  }
}

/** Puts [text] on the clipboard; true when the app should confirm it, as Android 13+ shows its own confirmation */
internal fun copyText(context: Context, text: String): Boolean {
  context.getSystemService(ClipboardManager::class.java)?.setPrimaryClip(ClipData.newPlainText(t("common.copy"), text))
  return Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU
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
  /** The turn's place in the session (fork from here) */
  index: Int = 0,
  /** The reply failed in a way Retry answers (`retryableTurn`): its action row offers it */
  retry: Boolean = false,
  /** The selections the prompt went out with: the statistics name the model from them when the peer did not */
  settings: TurnSettings? = null,
  controls: SessionControls = SessionControls(),
  actions: TurnHandlers? = null,
) {
  // A call whose observation was lost reads as settled before anything groups or draws it
  val split = remember(turn) { splitCodexBlocks(turn.blocks.map(::observedBlock)) }
  // Content already there when the turn comes into view stays still: only what arrives afterwards enters. A new turn
  // is first composed with at most its first block, which keeps its entrance
  remember(key) { if (turn.blocks.size > 1) seedEntrances(turn, split.process, key) }
  CompositionLocalProvider(
    LocalRowHeight provides WebMetrics.chatRow,
    LocalEntranceScope provides EntranceScope(live = running, scope = key),
    LocalClock provides now,
  ) {
    Column(Modifier.fillMaxWidth().padding(horizontal = Metrics.pad + Metrics.threadInset), verticalArrangement = Arrangement.spacedBy(Metrics.gap)) {
      // A saved plan cuts the turn: its card stands between the stretch before it and the continuation after it
      // (AgentMessage.swift, planSections.ts)
      val sections = remember(turn) { splitPlanSections(turn.blocks) }
      val outcome = turnOutcome(turn) != null
      val handlers = LocalChatHandlers.current
      sections.forEachIndexed { i, section ->
        androidx.compose.runtime.key(section.key) {
          val lastSection = i == sections.size - 1
          // A plan approval renders on its card. A stretch holding nothing else while the card waits has no working
          // row to show: the fold head there would say "awaiting approval" under the card that already asks
          val visible = section.blocks.any { it !is PermissionBlock || it.planId == null }
          val planWait = section.blocks.any { it is PermissionBlock && it.planId != null }
          if (visible || (lastSection && ((running && !planWait) || outcome))) {
            val part = remember(turn, section) { sectionTurn(turn, section, lastSection, sections.size > 1) }
            CodexMessage(part, if (sections.size > 1 && i > 0) "$key/${section.key}" else key, lastSection && running, now, onPermission)
          }
          section.plan?.let { plan ->
            PlanDocumentCard(
              plan,
              permission = turn.blocks.firstOrNull { it is PermissionBlock && it.planId == plan.id } as PermissionBlock?,
              onBuild = { option, model -> handlers?.buildPlan?.invoke(plan.id, option, model) },
              onPermission = onPermission,
            )
          }
        }
      }
      if (showsTurnActions(turn, running)) TurnActions(turn, index, retry, settings, controls, actions)
    }
  }
}

/**
 * One stretch of a turn as its own message (CodexMessage): goal milestones, the process fold, notices, the reply,
 * permission cards (a plan's approval renders on the plan card) and the outcome row
 */
@Composable
private fun CodexMessage(turn: AgentTurn, key: String, running: Boolean, now: () -> Long, onPermission: (blockId: String, optionId: String) -> Unit) {
  val split = remember(turn) { splitCodexBlocks(turn.blocks.map(::observedBlock)) }
  val hasTools = remember(turn) { turn.blocks.any { it is ToolCallBlock } }
  // Goal milestones before anything else open the turn above the fold
  for (goal in split.goalsLead) GoalMilestone(goal)
  CodexFold(turn, split.process, running, hasTools, key, now)
  for (n in split.notices) NoticeRow(n)
  for (reply in split.reply) {
    if (reply.markdown.isNotBlank()) StreamProse(reply, Modifier.testTag("reply"))
  }
  // Later milestones (paused, met, cleared …) close the reply they followed
  for (goal in split.goals) GoalMilestone(goal)
  for (p in split.permissions.filter { it.planId == null }) PermissionCard(p) { onPermission(p.id, it) }
  if (!running) turnOutcome(turn)?.let { OutcomeRow(turn, it) }
}

/**
 * A stretch as its own message: only the continuation owns the live activity and the turn's outcome, and with more than
 * one stretch none has timing of its own (repeating the whole turn's duration would lie)
 */
private fun sectionTurn(turn: AgentTurn, section: PlanSection, last: Boolean, split: Boolean): AgentTurn {
  var content = turn.copy(blocks = section.blocks)
  if (!last) content = content.copy(stop = null, error = null, command = null)
  if (split) content = content.copy(startedAt = null, respondedAt = null, endedAt = null)
  return content
}

/**
 * The stage holding the reply the thread draws for a held prompt (`heldStage`): its working row says it in place of
 * "Working"
 */
val LocalLiveStage = androidx.compose.runtime.compositionLocalOf<String?> { null }

/**
 * A call whose observation was lost must not pass for running work (ToolCall.tsx): it shows settled, so nothing
 * shimmers or ticks, and `toolVerb` says its result is unconfirmed
 */
private fun observedBlock(block: AgentBlock): AgentBlock =
  if (block is ToolCallBlock && block.observation == "unknown" && (block.status == ToolStatus.pending || block.status == ToolStatus.in_progress)) {
    block.copy(status = ToolStatus.cancelled)
  } else block

/** Marks every entrance identity of [turn] as entered (EntranceLedger.seed) */
private fun seedEntrances(turn: AgentTurn, process: List<AgentBlock>, scope: String) {
  fun claim(id: String) = EntranceLedger.claim("$scope\u0000$id")
  claim("fold-head")
  for (item in groupProcess(process)) claim(itemEntrance(item.id))
  for (block in turn.blocks) {
    when (block) {
      is ThoughtBlock -> claim("thought:${block.startedAt?.toLong() ?: ""}")
      is ToolCallBlock -> {
        claim(itemEntrance(block.id))
        claim("${block.id}:tool")
        claim("${block.id}:files")
      }
      else -> {}
    }
  }
}

/** What a settled turn's actions do (apps/ios `sessionActions`); the chat wires them to its session */
class TurnHandlers(val retry: () -> Unit, val fork: (turnIndex: Int) -> Unit, val toast: (String) -> Unit)

/**
 * The reply's actions under it (Cards.swift TurnActions), hugging the reply: retry, copy, fork from this turn, the
 * statistics. Retry shows only on the newest reply when it failed in a way Retry answers; a reply with nothing in it
 * offers no copy or statistics. Android 13+ confirms a copy itself, so the glyph only turns into a tick for a moment
 */
@Composable
private fun TurnActions(
  turn: AgentTurn,
  index: Int,
  retry: Boolean,
  settings: TurnSettings?,
  controls: SessionControls,
  actions: TurnHandlers?,
) {
  val reply = remember(turn) { replyMarkdown(turn) }
  val content = remember(turn) { hasTurnContent(turn) }
  if (actions == null && reply.isEmpty() && !content) return
  val context = LocalContext.current
  var copied by remember { mutableStateOf(false) }
  var stats by remember { mutableStateOf(false) }
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
    horizontalArrangement = Arrangement.spacedBy(Metrics.s0_5, Alignment.End),
  ) {
    if (retry && actions != null) ActionButton("rotateCcw", t("common.retry"), "turn.retry") { actions.retry() }
    if (reply.isNotEmpty()) {
      ActionButton(if (copied) "check" else "copy", t(if (copied) "code.copied" else "turn.copy"), "turn.copy") {
        if (copyText(context, reply)) actions?.toast?.invoke(t("code.copied"))
        copied = true
      }
    }
    if (actions != null) ActionButton("gitBranch", t("turn.fork"), "turn.fork") { actions.fork(index) }
    if (content) {
      Box {
        ActionButton("chartNoAxesColumn", t("turn.stats"), "turn.stats", open = stats) { stats = !stats }
        if (stats) StatsPopup(turn, remember(turn.usage, settings, controls) { modelLabel(turn.usage, settings, controls) }) { stats = false }
      }
    }
  }
}

/** One glyph of the action row; `open` marks the one whose card is showing */
@Composable
private fun ActionButton(icon: String, label: String, tag: String, open: Boolean = false, onClick: () -> Unit) {
  IconButton(
    onClick = onClick,
    colors = IconButtonDefaults.iconButtonColors(containerColor = if (open) Palette.active else Color.Transparent),
    modifier = Modifier.size(Metrics.ctlSm).testTag(tag),
  ) {
    Lucide(icon, if (open) Palette.fg1 else Palette.fg2, size = Metrics.chevron, contentDescription = label)
  }
}

/**
 * The statistics card as a popover on its button (the iOS app's `.popover`): above the button when there is room, else
 * below it; an outside tap or back closes it. A popup is a window of its own, so its test tags need the flag again
 */
@Composable
private fun StatsPopup(turn: AgentTurn, model: String?, onDismiss: () -> Unit) {
  val margin = with(LocalDensity.current) { Metrics.s2.roundToPx() }
  val position = remember(margin) { PopoverPosition(margin) }
  Popup(popupPositionProvider = position, onDismissRequest = onDismiss, properties = PopupProperties(focusable = true)) {
    TurnStatsCard(turn, model, Modifier.semantics { testTagsAsResourceId = true }.testTag("turn.statsCard"))
  }
}

/** Its trailing edge on the anchor's, kept [margin] inside the window; above the anchor when it fits, else below */
private class PopoverPosition(private val margin: Int) : PopupPositionProvider {
  override fun calculatePosition(anchorBounds: IntRect, windowSize: IntSize, layoutDirection: LayoutDirection, popupContentSize: IntSize): IntOffset {
    val maxX = maxOf(margin, windowSize.width - popupContentSize.width - margin)
    val x = (anchorBounds.right - popupContentSize.width).coerceIn(margin, maxX)
    val above = anchorBounds.top - margin - popupContentSize.height
    return IntOffset(x, if (above >= margin) above else anchorBounds.bottom + margin)
  }
}

/**
 * Cards.swift TurnStats (StatsCard): the title with the model, the timing tiles, the token lines with their share
 * chips, the context under a thin divider, and the vendor's request id with its copy action at the foot
 */
@Composable
private fun TurnStatsCard(turn: AgentTurn, model: String?, modifier: Modifier = Modifier) {
  val content = remember(turn) { turnStatsContent(turn) }
  val shape = RoundedCornerShape(Metrics.rLg)
  Column(
    modifier.width(Metrics.popWide)
      .shadow(Metrics.toastShadow, shape)
      .background(Palette.bg1, shape)
      .border(Metrics.hairline, Palette.conversationLine, shape)
      .padding(horizontal = Metrics.s2, vertical = Metrics.s3),
    verticalArrangement = Arrangement.spacedBy(Metrics.s1),
  ) {
    Row(
      Modifier.fillMaxWidth().heightIn(min = Metrics.ctl).padding(horizontal = Metrics.s2),
      verticalAlignment = Alignment.CenterVertically,
      horizontalArrangement = Arrangement.spacedBy(Metrics.s3),
    ) {
      Text(t("turn.stats"), color = Palette.fg1, fontSize = Type.text2, fontWeight = FontWeight.Medium, maxLines = 1)
      Text(
        model.orEmpty(), color = Palette.fg3, fontSize = Type.text3, maxLines = 1, overflow = TextOverflow.MiddleEllipsis,
        textAlign = TextAlign.End, modifier = Modifier.weight(1f),
      )
    }
    // Inset like the web's mx-1.5, so the tile sheet sits inside the token lines instead of running edge to edge
    if (content.tiles.isNotEmpty()) {
      Row(
        Modifier.padding(horizontal = Metrics.s1_5).fillMaxWidth().height(IntrinsicSize.Min)
          .background(Palette.active, RoundedCornerShape(Metrics.rMd)),
      ) {
        content.tiles.forEachIndexed { i, tile ->
          if (i > 0) Box(Modifier.width(Metrics.hairline).fillMaxHeight().background(Palette.line))
          Column(
            Modifier.weight(1f).padding(horizontal = Metrics.s2, vertical = Metrics.s1_5),
            verticalArrangement = Arrangement.spacedBy(Metrics.s0_5),
          ) {
            Text(figure(tile.value), maxLines = 1, overflow = TextOverflow.Ellipsis, style = tabular)
            Text(tile.label, color = Palette.fg3, fontSize = Type.text4, lineHeight = Type.text4Line, maxLines = 1, overflow = TextOverflow.Ellipsis)
          }
        }
      }
    }
    if (content.tokens.isNotEmpty() || content.context != null) {
      Column {
        for (line in content.tokens) StatRow(line.label, AnnotatedString(line.value), line.chip)
        content.context?.let { c ->
          if (content.tokens.isNotEmpty()) StatDivider(Modifier.padding(horizontal = Metrics.s2, vertical = Metrics.s1))
          val fg3 = Palette.fg3
          StatRow(t("stats.contextShort"), buildAnnotatedString { append(c.used); withStyle(SpanStyle(color = fg3)) { append(" / ${c.size}") } }, c.share)
        }
      }
    }
    turn.usage?.requestId?.let { RequestIdFooter(it) }
  }
}

@Composable
private fun StatDivider(modifier: Modifier = Modifier) {
  Box(modifier.fillMaxWidth().height(Metrics.hairline).background(Palette.line))
}

/** Figure: the digits large, the unit words between them small and faint ("2 分钟 24 秒", "33 tok/s") */
@Composable
private fun figure(text: String): AnnotatedString {
  val unit = SpanStyle(color = Palette.fg3, fontSize = Type.text4)
  val digits = SpanStyle(color = Palette.fg1, fontSize = Type.text2, fontWeight = FontWeight.SemiBold)
  return buildAnnotatedString {
    var at = 0
    for (m in FIGURE_DIGITS.findAll(text)) {
      if (m.range.first > at) withStyle(unit) { append(text.substring(at, m.range.first)) }
      withStyle(digits) { append(m.value) }
      at = m.range.last + 1
    }
    if (at < text.length) withStyle(unit) { append(text.substring(at)) }
  }
}

/** A label / value line; the chip (cache share, context share) rides with the value on the right */
@Composable
private fun StatRow(label: String, value: AnnotatedString, chip: String?) {
  Row(
    Modifier.fillMaxWidth().padding(horizontal = Metrics.s2, vertical = Metrics.s0_5),
    verticalAlignment = Alignment.CenterVertically,
    horizontalArrangement = Arrangement.spacedBy(Metrics.s2),
  ) {
    Text(label, color = Palette.fg2, fontSize = Type.text3, lineHeight = Type.text3Line, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
    chip?.let {
      Text(
        it, color = Palette.fg2, fontSize = Type.text3, lineHeight = Type.text3Line, maxLines = 1, style = tabular,
        modifier = Modifier.background(Palette.active, RoundedCornerShape(Metrics.rSm)).padding(horizontal = Metrics.s1),
      )
    }
    Text(value, color = Palette.fg1, fontSize = Type.text3, lineHeight = Type.text3Line, maxLines = 1, style = tabular)
  }
}

/** PanelFooter: the id in faint text and the copy button, under one divider across the card's content */
@Composable
private fun RequestIdFooter(requestId: String) {
  val context = LocalContext.current
  var copied by remember { mutableStateOf(false) }
  LaunchedEffect(copied) {
    if (copied) { delay(COPIED_MS); copied = false }
  }
  Column(verticalArrangement = Arrangement.spacedBy(Metrics.s1)) {
    StatDivider()
    Row(Modifier.padding(start = Metrics.s2), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(Metrics.s1)) {
      Text(
        requestId, color = Palette.fg3, fontSize = Type.text3, lineHeight = Type.text3Line, maxLines = 1,
        overflow = TextOverflow.MiddleEllipsis, modifier = Modifier.weight(1f),
      )
      ActionButton(if (copied) "check" else "copy", t(if (copied) "stats.requestIdCopied" else "stats.copyRequestId"), "stats.copyRequestId") {
        copyText(context, requestId)
        copied = true
      }
    }
  }
}

/** Figures line up in columns (the iOS card's monospacedDigit) */
private val tabular: TextStyle @Composable get() = LocalTextStyle.current.copy(fontFeatureSettings = "tnum")
private val FIGURE_DIGITS = Regex("""\d(?:[\d.,\u00a0\u202f]*\d)?""")

/** The text a copy takes: the turn's final prose, else its plain text blocks */
private fun replyMarkdown(turn: AgentTurn): String {
  val reply = splitCodexBlocks(turn.blocks).reply.joinToString("\n\n") { it.markdown }.trim()
  if (reply.isNotEmpty()) return reply
  return turn.blocks.filterIsInstance<TextBlock>().joinToString("\n\n") { it.markdown }.trim()
}

private const val COPIED_MS = 1500L

/**
 * The process fold: open while the turn works, closed once it is done; a tap on the head always wins (Turns.tsx
 * CodexFold). A turn with no process worth a head (a plain reply, or only a compaction status) shows no head. A steered
 * prompt stays in view while the parts around it fold
 */
@Composable
private fun CodexFold(turn: AgentTurn, blocks: List<AgentBlock>, running: Boolean, hasTools: Boolean, key: String, now: () -> Long) {
  val retired = !running && !hasTools && blocks.all { it is CompactionBlock }
  if (retired && blocks.isEmpty()) return
  var manual by rememberSaveable(key) { mutableStateOf<Boolean?>(null) }
  val open = retired || (manual ?: running)
  val items = remember(blocks) { groupProcess(blocks) }
  val parts = remember(items) { processParts(items) }
  // Only items fold; a process of nothing but steered prompts has no chevron
  val foldable = parts.any { it is FoldPart.Items }
  Column(Modifier.fillMaxWidth()) {
    if (!retired) EntranceOnce("fold-head") { FoldHead(turn, running, open, foldable, now) { manual = !open } }
    parts.forEachIndexed { i, part ->
      androidx.compose.runtime.key(part.id) {
        when (part) {
          is FoldPart.Steer -> EntranceOnce(itemEntrance(part.id)) {
            Box(Modifier.padding(top = Metrics.processProseGap)) { SteeredMessage(part.block) }
          }
          is FoldPart.Items -> Reveal(open) {
            ProcessHistory(
              part.items,
              Modifier.padding(
                top = if (retired) Metrics.s0 else if (i == 0) Metrics.s1 else Metrics.processProseGap,
                bottom = if (!retired && i == parts.size - 1) Metrics.s1_5 else Metrics.s0,
              ),
            )
          }
        }
      }
    }
  }
}

/** The fold's head: the Orb (or hand, plan, question) and the activity while live; a tick, ✗ or warning and the time once done */
@Composable
private fun FoldHead(turn: AgentTurn, running: Boolean, open: Boolean, foldable: Boolean, now: () -> Long, onToggle: () -> Unit) {
  // The stage takes the plain working label only; approvals, questions and retries keep their own
  val stage = LocalLiveStage.current
  val activity = liveActivity(turn).let { a -> if (stage != null && a.lead == LiveActivity.Lead.orb && turn.retry == null) a.copy(label = stage) else a }
  val outcome = turnOutcome(turn)
  val label = if (running) activity.label else outcome ?: t("turns.done")
  // A turn that rounds to 0s shows the bare Done label, as the live counter stays hidden for its first second
  val since = clockStart(turn)
  val end = turn.endedAt
  val elapsed = if (!running && turn.startedAt != null && end != null && (since == null || end - since >= HALF_SECOND_MS)) elapsedLabel(turn) else null
  AcpRow(
    Modifier.rowButton(if (foldable) open else null, onToggle).testTag("fold.head"),
    lead = {
      val ink = LocalContentColor.current
      when {
        !running -> Glyph(if (turn.stop == TurnStop.cancelled) "x" else if (outcome != null) "triangleAlert" else "check", ink, Metrics.icon)
        activity.lead == LiveActivity.Lead.hand -> Glyph("hand", ink)
        activity.lead == LiveActivity.Lead.question -> Glyph("messageCircleQuestion", ink)
        activity.lead == LiveActivity.Lead.plan -> Glyph("listTodo", ink)
        else -> Orb()
      }
    },
  ) {
    RowLabel(label, shimmer = running && activity.active)
    if (running && since != null) LiveElapsed(since, now)
    if (elapsed != null) Text(elapsed, color = Palette.fg3, maxLines = 1, softWrap = false, modifier = Modifier.alignByBaseline())
    if (foldable) FoldChevron(open, Palette.fg2, markOnBaseline())
  }
}

private const val HALF_SECOND_MS = 500

/**
 * The turn's process cut where something has to stay in view while the parts around it fold: a steered prompt. Runs
 * of items fold. A part is keyed by its first item, so content arriving later leaves the earlier parts in place
 */
private sealed interface FoldPart {
  val id: String
  data class Items(override val id: String, val items: List<ProcessItem>) : FoldPart
  data class Steer(override val id: String, val block: SteerBlock) : FoldPart
}

private fun processParts(items: List<ProcessItem>): List<FoldPart> {
  val out = ArrayList<FoldPart>()
  var rows = ArrayList<ProcessItem>()
  var runId = ""
  fun close() {
    if (rows.isNotEmpty()) { out += FoldPart.Items(runId, rows); rows = ArrayList() }
  }
  for (item in items) {
    val steer = (item as? ProcessItem.Block)?.block as? SteerBlock
    if (steer != null) {
      close()
      out += FoldPart.Steer(item.id, steer)
    } else {
      if (rows.isEmpty()) runId = item.id
      rows += item
    }
  }
  close()
  return out
}

private const val QUOTE_CHIP = 24

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
            // The everyday grants first, the bypassing ones after a gap
            AppMenu(expanded = menu, onDismiss = { menu = false }) {
              val plain = more.filter { !it.bypass }
              val bypass = more.filter { it.bypass }
              for (o in plain) AppMenuItem(o.label, onClick = { menu = false; onPick(o.id) }, subtitle = o.detail)
              if (plain.isNotEmpty() && bypass.isNotEmpty()) AppMenuGap()
              for (o in bypass) AppMenuItem(o.label, onClick = { menu = false; onPick(o.id) }, subtitle = o.detail)
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

