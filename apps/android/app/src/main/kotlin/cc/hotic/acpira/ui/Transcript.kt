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
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.layout.boundsInRoot
import androidx.compose.foundation.text.contextmenu.builder.item
import androidx.compose.foundation.text.contextmenu.modifier.appendTextContextMenuComponents
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
import cc.hotic.acpira.logic.absorbedNotices
import cc.hotic.acpira.logic.displayTurn
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
import androidx.compose.ui.draw.clip
import androidx.compose.foundation.clickable
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

/** The names whose tokens get pills in a sent prompt (PromptMarks.swift PromptMarkNames): the agent's commands and personas */
data class PromptMarkNames(val commands: List<String> = emptyList(), val summons: List<String> = emptyList())

/** Set along a chat's thread: the session's advertised commands and the summonable personas */
val LocalPromptMarks = androidx.compose.runtime.staticCompositionLocalOf { PromptMarkNames() }

/**
 * The prompt: a right-aligned bubble that always leaves `bubbleInset` free on its left; attachments ride above it. Its
 * `/command` and `@persona` tokens are painted as the composer painted them (Cards.swift UserMessageCard markedPrompt).
 * A long press opens its menu when [onToast] is given: Edit and resend ([onEdit], while the session is idle), Copy, Set
 * as goal ([onGoal], on the newest typed prompt where the agent takes a goal) and Select text, which hands the bubble
 * to the system's range selection with every word selected (history.selectText)
 */
@OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)
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
    if (attachments.isNotEmpty()) TurnAttachments(attachments)
    val shown = turn.text.trimEnd()
    if (shown.isNotBlank()) {
      val context = LocalContext.current
      val haptics = LocalHapticFeedback.current
      var menu by remember { mutableStateOf(false) }
      // The bubble handed to range selection (Select text) until its selection is dropped
      var selecting by remember { mutableStateOf(false) }
      // A recorded command keeps its pill even after the agent stops advertising it
      val names = LocalPromptMarks.current.let { m -> m.copy(commands = m.commands + listOfNotNull(turn.command)) }
      val bounds = remember { arrayOfNulls<androidx.compose.ui.geometry.Rect>(1) }
      Box {
        Box(
          Modifier.onGloballyPositioned { bounds[0] = it.boundsInRoot() }
            .background(Palette.bubbleBg, RoundedCornerShape(Metrics.rBubble))
            .then(if (onToast == null || selecting) Modifier else Modifier.pointerInput(Unit) {
              detectTapGestures(onLongPress = { haptics.performHapticFeedback(HapticFeedbackType.LongPress); menu = true })
            })
            .padding(horizontal = Metrics.pad + Metrics.s1, vertical = Metrics.gap)
            .testTag("bubble"),
        ) {
          if (selecting) {
            val state = androidx.compose.foundation.text.selection.rememberSelectionState()
            // Every word selected, then back to the plain bubble once the reader drops the selection
            cc.hotic.acpira.ui.prose.SelectAllWhile(true, state, { bounds[0] }) { selecting = false }
            androidx.compose.foundation.text.selection.SelectionContainer(
              state,
              Modifier.appendTextContextMenuComponents {
                // The prompt's own actions stay next to the selection's (Cards.swift promptActions)
                onEdit?.let { edit -> item(BubbleEditKey, t("history.edit")) { state.clear(); close(); edit() } }
                onGoal?.let { goal -> item(BubbleGoalKey, t("goal.setFromPrompt")) { state.clear(); close(); goal() } }
              },
            ) { MarkedPrompt(shown, names) }
          } else {
            MarkedPrompt(shown, names)
          }
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
              if (copyText(context, shown)) onToast?.invoke(t("code.copied"))
            },
            modifier = Modifier.testTag("bubble.copy"),
            icon = { MenuGlyph("copy") },
          )
          onGoal?.let { goal ->
            AppMenuItem(
              t("goal.setFromPrompt"), onClick = { menu = false; goal() }, modifier = Modifier.testTag("bubble.goal"), icon = { MenuGlyph("goal") },
            )
          }
          AppMenuItem(
            t("history.selectText"), onClick = { menu = false; selecting = true }, modifier = Modifier.testTag("bubble.select"), icon = { MenuGlyph("textSelect") },
          )
        }
      }
    }
  }
}

private object BubbleEditKey
private object BubbleGoalKey

/**
 * A sent prompt's text with its marks (Turns.tsx UserMessage): each `/command` and `@persona` in its colour over a
 * tinted pill, drawn from the text's own line layout as the composer's field draws them
 */
@Composable
private fun MarkedPrompt(text: String, names: PromptMarkNames) {
  val command = Palette.command
  val summon = Palette.summon
  val marks = remember(text, names) { cc.hotic.acpira.logic.SlashCommands.promptMarks(names.commands, names.summons, text) }
  val styled = remember(text, marks, command, summon) {
    buildAnnotatedString {
      append(text)
      for (m in marks) addStyle(SpanStyle(color = if (m.summon) summon else command), m.start, (m.start + m.length).coerceAtMost(text.length))
    }
  }
  var layout by remember { mutableStateOf<androidx.compose.ui.text.TextLayoutResult?>(null) }
  Text(
    styled, color = Palette.fg1, fontSize = Type.body, lineHeight = Type.bodyLine,
    onTextLayout = { layout = it },
    modifier = if (marks.isEmpty()) Modifier else Modifier.drawBehind {
      pills(layout, text, names.commands, names.summons, 0, command, summon, Type.body)
    },
  )
}

/** A queued prompt's attachments, laid out as the sent prompt's above its bubble */
@Composable
internal fun BubbleAttachments(attachments: List<Attachment>) = TurnAttachments(attachments)

/** Puts [text] on the clipboard; true when the app should confirm it, as Android 13+ shows its own confirmation */
internal fun copyText(context: Context, text: String): Boolean {
  context.getSystemService(ClipboardManager::class.java)?.setPrimaryClip(ClipData.newPlainText(t("common.copy"), text))
  return Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU
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
  /** The subagents announced during this turn (`nodesByTurn`) */
  subagents: List<cc.hotic.acpira.model.SubagentSummary> = emptyList(),
  /** The session's newest turn: only there, once settled, do failure notices offer their remedies */
  last: Boolean = false,
) {
  // A Revise end reads as settled, and error notices the closing row absorbs leave the blocks (TurnOutcome.swift)
  val absorbed = remember(turn) { absorbedNotices(turn) }
  val shown = remember(turn, absorbed) { displayTurn(turn, absorbed) }
  // A call whose observation was lost reads as settled before anything groups or draws it
  val split = remember(shown) { splitCodexBlocks(shown.blocks.map(::observedBlock)) }
  // Content already there when the turn comes into view stays still: only what arrives afterwards enters. A new turn
  // is first composed with at most its first block, which keeps its entrance
  remember(key) { if (shown.blocks.size > 1) seedEntrances(shown, split.process, key) }
  // Subagent rows sit where they were delegated: a placed delegation row stays in the turn's process, and its node with
  // that node's same-turn descendants renders at that point. The others follow the turn's process (AgentMessage.swift)
  val placed = remember(shown, subagents) { cc.hotic.acpira.logic.delegatedIds(shown.blocks, subagents) }
  val placement = remember(subagents, placed) { cc.hotic.acpira.logic.placeNodes(subagents, placed) }
  val readOnly = cc.hotic.acpira.ui.subagents.LocalSubagentTranscript.current
  CompositionLocalProvider(
    LocalRowHeight provides WebMetrics.chatRow,
    LocalEntranceScope provides EntranceScope(live = running, scope = key),
    LocalClock provides now,
    cc.hotic.acpira.ui.subagents.LocalDelegations provides placement.byId,
    LocalNoticeScope provides NoticeScope(showActions = last && !running && !readOnly, suppressId = turn.error?.failureId, absorbed = absorbed),
  ) {
    Column(Modifier.fillMaxWidth().padding(horizontal = Metrics.pad + Metrics.threadInset), verticalArrangement = Arrangement.spacedBy(Metrics.gap)) {
      // A saved plan cuts the turn: its card stands between the stretch before it and the continuation after it
      // (AgentMessage.swift, planSections.ts)
      val sections = remember(shown, placed) { splitPlanSections(shown.blocks, placed) }
      val outcome = turnOutcome(shown) != null
      val handlers = LocalChatHandlers.current
      sections.forEachIndexed { i, section ->
        androidx.compose.runtime.key(section.key) {
          val lastSection = i == sections.size - 1
          // A plan approval renders on its card. A stretch holding nothing else while the card waits has no working
          // row to show: the fold head there would say "awaiting approval" under the card that already asks
          val visible = section.blocks.any { it !is PermissionBlock || it.planId == null }
          val planWait = section.blocks.any { it is PermissionBlock && it.planId != null }
          // A child row that never renders anywhere would be worse than a turn with nothing else to show
          if (visible || (lastSection && ((running && !planWait) || outcome || placement.rest.isNotEmpty()))) {
            val part = remember(shown, section) { sectionTurn(shown, section, lastSection, sections.size > 1) }
            CodexMessage(
              part, if (sections.size > 1 && i > 0) "$key/${section.key}" else key, lastSection && running, now, onPermission,
              subagents = if (lastSection) placement.rest else emptyList(),
            )
          }
          section.plan?.let { plan ->
            PlanDocumentCard(
              plan,
              permission = shown.blocks.firstOrNull { it is PermissionBlock && it.planId == plan.id } as PermissionBlock?,
              onBuild = { option, model -> handlers?.buildPlan?.invoke(plan.id, option, model) },
              onPermission = onPermission,
            )
          }
        }
      }
      // A child's transcript offers no action row: fork there would act on the parent session
      if (showsTurnActions(turn, running) && !readOnly) {
        TurnActions(turn, index, retry, settings, controls, actions)
      }
    }
  }
}

/**
 * One stretch of a turn as its own message (CodexMessage): goal milestones, the process fold, notices, the reply,
 * permission cards (a plan's approval renders on the plan card) and the outcome row
 */
@Composable
private fun CodexMessage(
  turn: AgentTurn, key: String, running: Boolean, now: () -> Long, onPermission: (blockId: String, optionId: String) -> Unit,
  /** The turn's children with no delegation row to sit at: they follow the process, before the reply */
  subagents: List<cc.hotic.acpira.model.SubagentSummary> = emptyList(),
) {
  val split = remember(turn) { splitCodexBlocks(turn.blocks.map(::observedBlock)) }
  val hasTools = remember(turn) { turn.blocks.any { it is ToolCallBlock } }
  // Goal milestones before anything else open the turn above the fold
  for (goal in split.goalsLead) GoalMilestone(goal)
  CodexFold(turn, split.process, running, hasTools, key, now)
  for (n in split.notices) NoticeRow(n)
  if (subagents.isNotEmpty()) {
    cc.hotic.acpira.ui.subagents.SubagentGroup(subagents)
    cc.hotic.acpira.ui.subagents.ChildPermissions(subagents)
  }
  for (reply in split.reply) {
    // A selection in the reply quotes into the chat's composer (ui/Quotes.kt)
    if (reply.markdown.isNotBlank()) {
      CompositionLocalProvider(LocalMarkdownImages provides reply.images) { StreamProse(reply, Modifier.testTag("reply"), onQuote = LocalQuote.current) }
    }
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
    if (retry && actions != null) ActionButton("rotateCcw", t("common.retry"), "turn.retry", hint = t("alert.retryHint")) { actions.retry() }
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

/**
 * One glyph of the action row; `open` marks the one whose card is showing. [hint] says what the tap does beyond the
 * label (TalkBack reads it as the action, as VoiceOver reads iOS's `accessibilityHint`)
 */
@Composable
private fun ActionButton(icon: String, label: String, tag: String, open: Boolean = false, hint: String? = null, onClick: () -> Unit) {
  Box(
    Modifier.size(Metrics.ctlSm).clip(CircleShape).background(if (open) Palette.active else Color.Transparent)
      .clickable(onClickLabel = hint, role = androidx.compose.ui.semantics.Role.Button, onClick = onClick).testTag(tag),
    contentAlignment = Alignment.Center,
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
  // Only items fold; a process of nothing but steered prompts or delegations has no chevron
  val foldable = parts.any { it is FoldPart.Items }
  Column(Modifier.fillMaxWidth()) {
    if (!retired) EntranceOnce("fold-head") { FoldHead(turn, running, open, foldable, now) { manual = !open } }
    CompositionLocalProvider(LocalImageInFold provides false) {
    parts.forEachIndexed { i, part ->
      androidx.compose.runtime.key(part.id) {
        when (part) {
          is FoldPart.Steer -> EntranceOnce(itemEntrance(part.id)) {
            Box(Modifier.padding(top = Metrics.processProseGap)) { SteeredMessage(part.block) }
          }
          // The rows of a delegation stay in view: in the stream while the parts show, under the head once they have
          // folded away, never a fold of their own
          is FoldPart.Subagents -> cc.hotic.acpira.ui.subagents.DelegatedSubagents(part.ids, Modifier.padding(top = Metrics.processProseGap))
          is FoldPart.Items -> Reveal(open) {
            ProcessHistory(
              part.items,
              Modifier.padding(
                top = if (retired) Metrics.s0 else if (i == 0) Metrics.s1 else Metrics.processProseGap,
                bottom = if (!retired && i == parts.size - 1) Metrics.s1_5 else Metrics.s0,
              ),
            )
          }
          // One live result outside every reveal: only the process rows around it collapse
          is FoldPart.Media -> Box(
            Modifier.padding(top = Metrics.gap, bottom = if (!retired && i == parts.size - 1) Metrics.s1_5 else Metrics.s0),
          ) { ImageResults(part.block) }
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
 * The turn's process cut where something has to stay in view while the parts around it fold: a steered prompt and
 * subagent rows. Runs of items fold. A part is keyed by its first item, so content arriving later leaves the earlier
 * parts in place
 */
private sealed interface FoldPart {
  val id: String
  data class Items(override val id: String, val items: List<ProcessItem>) : FoldPart
  data class Steer(override val id: String, val block: SteerBlock) : FoldPart
  data class Media(override val id: String, val block: ToolCallBlock) : FoldPart
  /** Back-to-back delegations (a parallel launch) share one part */
  data class Subagents(override val id: String, val ids: List<String>) : FoldPart
}

private fun processParts(items: List<ProcessItem>): List<FoldPart> {
  val out = ArrayList<FoldPart>()
  var rows = ArrayList<ProcessItem>()
  var children = ArrayList<String>()
  var runId = ""
  fun close() {
    if (rows.isNotEmpty()) { out += FoldPart.Items(runId, rows); rows = ArrayList() }
    if (children.isNotEmpty()) { out += FoldPart.Subagents(runId, children); children = ArrayList() }
  }
  for (item in items) {
    val block = (item as? ProcessItem.Block)?.block
    val steer = block as? SteerBlock
    val child = (block as? ToolCallBlock)?.subagentId
    if (steer != null) {
      close()
      out += FoldPart.Steer(item.id, steer)
    } else if (child != null) {
      if (rows.isNotEmpty()) close()
      if (children.isEmpty()) runId = item.id
      children += child
    } else {
      if (children.isNotEmpty()) close()
      if (rows.isEmpty()) runId = item.id
      rows += item
      val call = (item as? ProcessItem.Block)?.block as? ToolCallBlock
      if (call != null && cc.hotic.acpira.store.isImageResult(call)) {
        // The tool's row folds with the process, but its result keeps a stable identity and is never clipped
        close()
        out += FoldPart.Media("media:${item.id}", call)
      }
    }
  }
  close()
  return out
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
 * and the strong ink. [oneLine]: a menu trigger, exactly --ctl tall; otherwise the answer form, whose label may wrap.
 * [compact]: the small form (--ctl-sm, narrower sides) for remedies inside the transcript, at that height in layout too
 */
@Composable
fun PillButton(
  label: String,
  onClick: () -> Unit,
  modifier: Modifier = Modifier,
  primary: Boolean = false,
  oneLine: Boolean = false,
  compact: Boolean = false,
  trailing: (@Composable () -> Unit)? = null,
) {
  // Material pads a button to its touch target in layout; the compact form keeps the iOS row's height
  CompositionLocalProvider(
    androidx.compose.material3.LocalMinimumInteractiveComponentSize provides
      if (compact) androidx.compose.ui.unit.Dp.Unspecified else androidx.compose.material3.LocalMinimumInteractiveComponentSize.current,
  ) {
  OutlinedButton(
    onClick = onClick,
    shape = CircleShape,
    border = BorderStroke(Metrics.hairline, if (primary) Palette.lineStrong else Palette.line),
    colors = ButtonDefaults.outlinedButtonColors(
      containerColor = if (primary) Palette.active else Color.Transparent,
      contentColor = if (primary) Palette.fgStrong else Palette.fg2,
    ),
    contentPadding = PaddingValues(
      horizontal = (if (compact) Metrics.s2 else Metrics.s3) + Metrics.hairline,
      vertical = if (oneLine || compact) Metrics.s0 else Metrics.gapHalf + Metrics.hairline,
    ),
    modifier = modifier.heightIn(min = if (compact) Metrics.ctlSm else Metrics.ctl),
  ) {
    Text(
      label, fontSize = Type.text2, fontWeight = if (primary) FontWeight.Medium else FontWeight.Normal,
      maxLines = if (oneLine) 1 else Int.MAX_VALUE, softWrap = !oneLine,
    )
    trailing?.let { Spacer(Modifier.width(Metrics.s1_5)); it() }
  }
  }
}

