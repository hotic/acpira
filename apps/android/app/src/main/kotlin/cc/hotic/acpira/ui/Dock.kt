package cc.hotic.acpira.ui

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.snap
import androidx.compose.animation.core.tween
import androidx.compose.animation.expandVertically
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.animation.shrinkVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
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
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withStyle
import cc.hotic.acpira.design.IconStroke
import cc.hotic.acpira.design.LocalMotion
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.QUEUED_BUBBLE
import cc.hotic.acpira.design.RowMotion
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.design.WebMetrics
import cc.hotic.acpira.design.WebType
import cc.hotic.acpira.design.dockGap
import cc.hotic.acpira.design.glassCtl
import cc.hotic.acpira.design.planEntryPadY
import cc.hotic.acpira.design.planMax
import cc.hotic.acpira.design.planPadY
import cc.hotic.acpira.design.planSectionGap
import cc.hotic.acpira.design.popFg2
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.logic.firstLine
import cc.hotic.acpira.logic.goalEventLabel
import cc.hotic.acpira.logic.goalEventStatus
import cc.hotic.acpira.logic.goalHeadActions
import cc.hotic.acpira.logic.goalHeadMeta
import cc.hotic.acpira.logic.goalRowDetail
import cc.hotic.acpira.logic.goalSpend
import cc.hotic.acpira.logic.goalStatusLabel
import cc.hotic.acpira.logic.noticeState
import cc.hotic.acpira.logic.openQuestion
import cc.hotic.acpira.logic.shortStop
import cc.hotic.acpira.model.AgentTurn
import cc.hotic.acpira.model.GoalAction
import cc.hotic.acpira.model.GoalBlock
import cc.hotic.acpira.model.GoalStatus
import cc.hotic.acpira.model.ModelPick
import cc.hotic.acpira.model.PlanBlock
import cc.hotic.acpira.model.PlanDocumentBlock
import cc.hotic.acpira.model.PlanEntry
import cc.hotic.acpira.model.PlanStatus
import cc.hotic.acpira.model.QueuedPrompt
import cc.hotic.acpira.model.SessionGoal
import cc.hotic.acpira.model.SessionView
import cc.hotic.acpira.model.ToolCallBlock
import cc.hotic.acpira.model.ToolStatus
import cc.hotic.acpira.model.Turn
import cc.hotic.acpira.logic.NoticeWay
import cc.hotic.acpira.store.AppModel
import cc.hotic.acpira.store.answer
import cc.hotic.acpira.store.cancelEdit
import cc.hotic.acpira.store.carryOn
import cc.hotic.acpira.store.compact
import cc.hotic.acpira.store.continueNew
import cc.hotic.acpira.store.login
import cc.hotic.acpira.store.restart
import cc.hotic.acpira.store.takeOver
import kotlinx.coroutines.delay

// chat/Shell.tsx's DockStack at phone width (apps/ios Chat/Dock.swift): every surface under the thread (question,
// alert, session notice, the prompt being edited, composer) shares one --dock-gap rhythm, and the live to-do card
// (PlanDock, headed by the session goal) floats just above it, over the bottom of the thread. Queued prompts are faint
// bubbles at the thread's end (QueueBubble).

/** What the thread's own controls do, made once per session (apps/ios `sessionActions`) */
class ChatHandlers(
  val permission: (blockId: String, option: String) -> Unit,
  val buildPlan: (plan: String, option: String?, model: ModelPick?) -> Unit,
  /** Takes the sent prompt at this index into the composer (null: prompts cannot be edited now) */
  val editPrompt: (Int) -> Unit,
  val goal: (GoalAction, String?) -> Unit,
  val sendQueued: (String) -> Unit,
  val editQueued: (String) -> Unit,
  val removeQueued: (String) -> Unit,
  /** A failure notice's remedy (null: the transcript offers none, as a subagent's) */
  val failureAction: ((cc.hotic.acpira.model.FailureAction) -> Unit)? = null,
  /** Stop on a background task's row, by task id (null: no Stop, as in a subagent's transcript) */
  val stopTask: ((String) -> Unit)? = null,
)

val LocalChatHandlers = androidx.compose.runtime.compositionLocalOf<ChatHandlers?> { null }

/**
 * The surfaces under the thread, top to bottom; [composer] is the chat's composer, handed the placeholder the dock
 * wants (revising a waiting plan) or null for its own
 */
@Composable
fun Dock(model: AppModel, id: String, session: SessionView, editing: cc.hotic.acpira.model.UserTurn?, composer: @Composable () -> Unit) {
  val question = remember(session.turns.lastOrNull()) { openQuestion(session) }
  val alertTurn = remember(session.turns.lastOrNull(), session.running, session.status) { shortStop(session) }
  // The ✕ keeps the transcript row and drops the card, for this many turns
  var dismissed by rememberSaveable(id) { mutableStateOf<Int?>(null) }
  val notice = noticeState(session, model.agents.firstOrNull { it.id == session.agent })
  // The composer's own top inset is the gap between the last card and its bar
  Column {
    Column(Modifier.padding(horizontal = Metrics.pad), verticalArrangement = Arrangement.spacedBy(Metrics.dockGap)) {
      if (question != null) {
        androidx.compose.runtime.key(question.id) {
          QuestionCard(question) { answers, skipped -> model.answer(id, question.id, answers, skipped) }
        }
      }
      if (alertTurn != null && dismissed != session.turns.size) {
        AlertCard(
          alertTurn,
          onDismiss = { dismissed = session.turns.size },
          onRetry = { model.retryTurn(id) },
          onContinue = { model.carryOn(id) },
          onCompact = if (session.commands.any { it.name == "compact" }) ({ model.compact(id) }) else null,
          onNewSession = { model.continueNew(id) },
        )
      }
      notice?.let { state ->
        SessionNoticeCard(state) { way ->
          when (way) {
            NoticeWay.Restart -> model.restart(id)
            NoticeWay.TakeOver -> model.takeOver(id)
            NoticeWay.NewSession -> model.continueNew(id)
            is NoticeWay.Login -> model.login(id, way.id)
          }
        }
      }
      // While a sent prompt is being rewritten, the same composer (so focus holds) edits it
      if (editing != null) EditingBar(editing) { model.cancelEdit(id) }
    }
    // The composer pads itself to the screen's edges
    composer()
  }
}

// Plan dock

/**
 * dockPlan: open entries stay docked across turns; a finished list stays only while its own turn still runs. A turn's
 * list is its `plan` block (updated in place, so always the turn's newest plan), else its last completed todo tool row:
 * the built-in agent and OpenCode report the list only through their todo tool
 */
fun dockPlan(turns: List<Turn>, running: Boolean): List<PlanEntry>? {
  for (i in turns.indices.reversed()) {
    val agent = turns[i] as? AgentTurn ?: continue
    val plan = agent.blocks.lastOrNull { it is PlanBlock }?.let { (it as PlanBlock).entries }
    val todos = agent.blocks.lastOrNull { it is ToolCallBlock && it.verbKey == "verb.todo" && it.status == ToolStatus.completed && it.todoEntries != null }
      ?.let { (it as ToolCallBlock).todoEntries }
    val entries = plan ?: todos ?: continue
    // A list written while planning is part of the plan: the dock waits until that turn's plan is built
    if (agent.blocks.any { it is PlanDocumentBlock && it.status != "approved" && it.status != "executing" }) return null
    if (entries.any { it.status != PlanStatus.completed }) return entries
    return if (running && i == turns.size - 1) entries else null
  }
  return null
}

/** The plan card floating over the bottom of the thread, --dock-gap above the dock; the session goal heads it */
@Composable
fun PlanDock(session: SessionView, onGoal: (GoalAction) -> Unit) {
  val entries = remember(session.turns, session.running) { dockPlan(session.turns, session.running) }
  val goal = session.goal
  if (entries == null && goal == null) return
  Box(Modifier.padding(start = Metrics.pad, end = Metrics.pad, bottom = Metrics.dockGap)) {
    PlanCard(entries, session.running, goal, goal?.let { goalHeadActions(it, session.goalActions) }.orEmpty(), onGoal)
  }
}

/**
 * PlanBar's card: the to-do title (shimmering while the turn works) with the done count and a caret; folded it shows the
 * current entry only (or "all n done"), unfolded every entry, scrolling past seven rows. A session goal takes the head
 * row (GoalHead), so goal and to-dos share one card. Every row stays mounted: folding collapses the rows around the
 * current one, so the current entry never moves
 */
@Composable
private fun PlanCard(entries: List<PlanEntry>?, live: Boolean, goal: SessionGoal?, goalActions: List<GoalAction>, onGoal: (GoalAction) -> Unit) {
  var open by rememberSaveable { mutableStateOf(false) }
  val done = entries?.count { it.status == PlanStatus.completed } ?: 0
  val current = entries?.let { e -> e.indexOfFirst { it.status == PlanStatus.in_progress }.takeIf { it >= 0 } ?: e.indexOfFirst { it.status == PlanStatus.pending }.takeIf { it >= 0 } }
  // A single entry always shows itself, so there is nothing to fold: no summary, caret or toggle
  val foldable = (entries?.size ?: 0) > 1
  val toggle = { open = !open }
  DockCard(Modifier.testTag("plan.dock")) {
    Column(
      Modifier.padding(horizontal = Metrics.pad, vertical = Metrics.planPadY),
      verticalArrangement = Arrangement.spacedBy(Metrics.planSectionGap),
    ) {
      if (goal != null) {
        GoalHead(goal, goalActions, live, onGoal, onToggle = if (foldable) toggle else null) {
          if (entries != null) PlanCount(done, entries.size, foldable, open)
        }
      } else if (entries != null) {
        AcpRow(
          (if (foldable) Modifier.rowButton(open, toggle) else Modifier).testTag("plan.dock.head"),
          dense = true, icon = "listTodo",
          trailing = { PlanCount(done, entries.size, foldable, open) },
        ) { RowLabel(t("plan.todoTitle"), shimmer = live && current != null) }
      }
      if (entries != null) PlanList(entries, current, foldable, open)
    }
  }
}

/** Every entry, those away from the current one collapsed while folded; "all n done" in their place once all are */
@Composable
private fun PlanList(entries: List<PlanEntry>, current: Int?, foldable: Boolean, open: Boolean) {
  val motion = LocalMotion.current
  @Composable
  fun Fold(shown: Boolean, content: @Composable () -> Unit) {
    AnimatedVisibility(
      visible = shown,
      enter = if (motion) expandVertically(RowMotion.fold(), expandFrom = Alignment.Top) + fadeIn(RowMotion.fold()) else androidx.compose.animation.EnterTransition.None,
      exit = if (motion) shrinkVertically(RowMotion.fold(), shrinkTowards = Alignment.Top) + fadeOut(RowMotion.fold()) else androidx.compose.animation.ExitTransition.None,
    ) { content() }
  }
  Column(Modifier.heightIn(max = Metrics.planMax).then(if (open) Modifier.verticalScroll(rememberScrollState()) else Modifier)) {
    Fold(foldable && !open && current == null) {
      androidx.compose.runtime.CompositionLocalProvider(LocalRowDenseHeight provides with(androidx.compose.ui.platform.LocalDensity.current) { WebType.text2.lineHeight.toDp() }) {
        AcpRow(Modifier.padding(vertical = Metrics.planEntryPadY), dense = true, leadTop = true, lead = { PlanDot(PlanStatus.completed) }) {
          Text(t("plan.completed", "n" to entries.size), color = Palette.fg2)
        }
      }
    }
    entries.forEachIndexed { i, entry ->
      Fold(open || !foldable || i == current) { PlanEntries(listOf(entry)) }
    }
  }
}

/** The done count and the caret that folds the list */
@Composable
private fun PlanCount(done: Int, total: Int, foldable: Boolean, open: Boolean) {
  Text("$done/$total", maxLines = 1, softWrap = false)
  if (foldable) {
    val turn by animateFloatAsState(if (open) HALF_TURN else 0f, if (LocalMotion.current) RowMotion.fold() else snap(), label = "caret")
    Glyph("chevronDown", Palette.fg3, Metrics.icon, IconStroke.chevron, Modifier.rotate(turn))
  }
}

// Goal

/** The status glyph: the goal mark (accent while it runs), warn for blocked / limited, a check once met */
@Composable
fun GoalGlyph(status: GoalStatus, quiet: Boolean = false) {
  when (status) {
    GoalStatus.complete -> Glyph("circleCheck", Palette.ok)
    GoalStatus.blocked, GoalStatus.limited -> Glyph("triangleAlert", Palette.warn)
    else -> Glyph("goal", if (status == GoalStatus.active && !quiet) Palette.brand else Palette.fg3)
  }
}

/** A milestone (GoalViews.swift GoalRow): the event, the objective's first line as the target, the spend of a stretch */
@Composable
fun GoalMilestone(block: GoalBlock) {
  val detail = goalRowDetail(block)
  AcpRow(lead = { GoalGlyph(goalEventStatus(block.event), quiet = true) }, trailing = detail?.let { d -> { Text(d, maxLines = 1, softWrap = false) } }) {
    RowLabel(goalEventLabel(block.event))
    block.objective?.let(::firstLine)?.let { RowTarget(it) }
  }
}

/**
 * The head row of the floating card (GoalHead): the glyph, the status and the objective on one truncating line (one
 * shimmer while the goal runs a turn), the controls' menu, then the to-do count and caret when a list shares the card
 * (a tap on the row folds it). The elapsed time or rounds head the menu with the spend; without a menu they stay on
 * the line
 */
@Composable
private fun GoalHead(
  goal: SessionGoal,
  actions: List<GoalAction>,
  running: Boolean,
  onAction: (GoalAction) -> Unit,
  onToggle: (() -> Unit)?,
  trailing: @Composable androidx.compose.foundation.layout.RowScope.() -> Unit,
) {
  val live = goal.status == GoalStatus.active && running
  val muted = goal.status == GoalStatus.paused || goal.status == GoalStatus.complete
  val objective = firstLine(goal.objective).orEmpty()
  val status = goalStatusLabel(goal.status)
  Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(Metrics.s2), modifier = Modifier.testTag("goal.head")) {
    AcpRow(
      Modifier.weight(1f).then(if (onToggle != null) Modifier.rowButton(null, onToggle) else Modifier),
      dense = true,
      lead = { GoalGlyph(goal.status) },
      trailing = if (actions.isEmpty()) goalHeadMeta(goal)?.let { m -> { Text(m, maxLines = 1, softWrap = false) } } else null,
    ) {
      if (live) {
        Shimmer("$status  $objective", modifier = Modifier.weight(1f, fill = false).alignByBaseline())
      } else {
        val fg1 = Palette.fg1
        val tail = if (muted) Palette.fg3 else Palette.fg2
        Text(
          buildAnnotatedString {
            withStyle(SpanStyle(color = fg1)) { append(status) }
            append("  ")
            withStyle(SpanStyle(color = tail)) { append(objective) }
          },
          maxLines = 1, overflow = TextOverflow.Ellipsis, softWrap = false, modifier = Modifier.weight(1f, fill = false).alignByBaseline(),
        )
      }
    }
    if (actions.isNotEmpty()) GoalMenu(goal, actions, onAction)
    Row(horizontalArrangement = Arrangement.spacedBy(Metrics.s2), verticalAlignment = Alignment.CenterVertically) {
      androidx.compose.material3.ProvideTextStyle(WebType.text3.copy(color = Palette.fg3, fontFeatureSettings = TABULAR)) { trailing() }
    }
  }
}

/** The controls the agent advertised, under the full objective and the spend */
@Composable
private fun GoalMenu(goal: SessionGoal, actions: List<GoalAction>, onAction: (GoalAction) -> Unit) {
  var menu by remember { mutableStateOf(false) }
  Box {
    Box(
      Modifier.size(WebMetrics.ctlSm).clip(CircleShape).clickable { menu = true }.testTag("goal.menu"),
      contentAlignment = Alignment.Center,
    ) { Glyph("ellipsis", Palette.fg2) }
    AppMenu(expanded = menu, onDismiss = { menu = false }) {
      // The full objective, then the time or rounds and the spend on one line
      val spend = listOfNotNull(goalHeadMeta(goal), goalSpend(goal.tokensUsed, goal.tokenBudget))
      Column(Modifier.fillMaxWidth().padding(horizontal = Metrics.pad, vertical = Metrics.s2)) {
        Text(goal.objective, color = Palette.fg3, fontSize = Type.text3, lineHeight = Type.text3Line)
        if (spend.isNotEmpty()) Text(spend.joinToString(t("common.metaSep")), color = Palette.fg3, fontSize = Type.text3, lineHeight = Type.text3Line)
      }
      AppMenuGap()
      for (action in actions) {
        when (action) {
          GoalAction.pause -> AppMenuItem(t("goal.pause"), onClick = { menu = false; onAction(action) }, icon = { MenuGlyph("pause") })
          GoalAction.resume -> AppMenuItem(t("goal.resume"), onClick = { menu = false; onAction(action) }, icon = { MenuGlyph("play") })
          GoalAction.clear -> AppMenuItem(t("goal.clear"), onClick = { menu = false; onAction(action) }, danger = true, icon = { MenuGlyph("trash2", danger = true) })
          else -> {}
        }
      }
    }
  }
}

// Queue

/**
 * A queued prompt as the phone shows it (QueueBubble): the sent prompt's bubble, right-aligned after the running turn and
 * faint until it goes, so it reads as a message on its way rather than a control. A tap opens a menu: send now (or
 * steer, when the agent takes it mid-turn), edit and remove, under the line saying when it goes
 */
@Composable
fun QueueBubble(item: QueuedPrompt, steer: Boolean, onSend: () -> Unit, onEdit: () -> Unit, onRemove: () -> Unit) {
  var menu by remember { mutableStateOf(false) }
  val sending = item.sending == true
  Box(Modifier.fillMaxWidth().padding(start = Metrics.bubbleInset, end = Metrics.pad), contentAlignment = Alignment.CenterEnd) {
    Box {
      Box(Modifier.alpha(QUEUED_BUBBLE)) {
        Column(horizontalAlignment = Alignment.End, verticalArrangement = Arrangement.spacedBy(Metrics.s2)) {
          val shown = item.text.trim()
          item.attachments?.takeIf { it.isNotEmpty() }?.let { BubbleAttachments(it) }
          if (shown.isNotEmpty()) {
            Box(
              Modifier.clip(RoundedCornerShape(Metrics.rBubble)).background(Palette.bubbleBg)
                .clickable(enabled = !sending) { menu = true }
                .padding(horizontal = Metrics.pad + Metrics.s1, vertical = Metrics.gap)
                .testTag("queue.bubble"),
            ) {
              Text(shown, color = Palette.fgStrong, fontSize = Type.body, lineHeight = Type.bodyLine, maxLines = QUEUED_LINES, overflow = TextOverflow.Ellipsis)
            }
          }
        }
      }
      AppMenu(expanded = menu, onDismiss = { menu = false }) {
        Text(
          t(if (sending) "queue.sending" else "queue.title"), color = Palette.fg3, fontSize = Type.text3, lineHeight = Type.text3Line,
          modifier = Modifier.fillMaxWidth().padding(horizontal = Metrics.pad, vertical = Metrics.s1_5),
        )
        AppMenuItem(
          t(if (steer) "queue.steer" else "queue.sendNow"), onClick = { menu = false; onSend() },
          modifier = Modifier.testTag("queue.send"), icon = { MenuGlyph(if (steer) "merge" else "arrowUp") },
        )
        AppMenuItem(t("queue.edit"), onClick = { menu = false; onEdit() }, modifier = Modifier.testTag("queue.edit"), icon = { MenuGlyph("pencil") })
        AppMenuItem(t("queue.remove"), onClick = { menu = false; onRemove() }, danger = true, modifier = Modifier.testTag("queue.remove"), icon = { MenuGlyph("trash2", danger = true) })
      }
    }
  }
}

// Scroll to latest, starting

/**
 * Back to the latest message (ScrollToLatest): a small round drop on the composer's right edge, there while the thread is
 * more than half a screen above its end. Hidden while the thread follows a running turn
 */
@Composable
fun ScrollToLatest(shown: Boolean, modifier: Modifier = Modifier, onClick: () -> Unit) {
  val dur = if (LocalMotion.current) RowMotion.outcomeTime else 0
  AnimatedVisibility(
    visible = shown, modifier = modifier,
    enter = fadeIn(tween(dur)) + scaleIn(tween(dur), initialScale = SCROLL_DROP_FROM),
    exit = fadeOut(tween(dur)) + scaleOut(tween(dur), targetScale = SCROLL_DROP_FROM),
  ) {
    Surface(
      color = Palette.bg1, shape = CircleShape, shadowElevation = Metrics.barShadow,
      modifier = Modifier.size(Metrics.ctlSm).border(Metrics.hairline, Palette.line, CircleShape),
    ) {
      Box(Modifier.clickable(onClick = onClick).testTag("chat.latest"), contentAlignment = Alignment.Center) {
        Lucide("arrowDown", Palette.fg1, size = Metrics.icon, contentDescription = t("mobile.scrollToLatest"))
      }
    }
  }
}

/**
 * ConnectingNotice at phone width (StartingTitle): a pill in the bar's title slot while a session starts with no reply to
 * say it on, once the start has taken a moment, so a warm agent does not flash it. A held prompt makes the thread's view
 * running, and that reply's working row speaks instead (`heldStage`), so the words never show in two places
 */
@Composable
fun StartingTitle(active: Boolean, agentName: String, fallback: @Composable () -> Unit) {
  var late by remember { mutableStateOf(false) }
  LaunchedEffect(active) {
    late = false
    if (!active) return@LaunchedEffect
    delay(STARTING_DELAY_MS)
    late = true
  }
  if (!(active && late)) { fallback(); return }
  Row(
    Modifier.heightIn(min = Metrics.glassCtl).clip(CircleShape).background(Palette.chip).padding(horizontal = Metrics.pad)
      .testTag("chat.starting"),
    verticalAlignment = Alignment.CenterVertically,
    horizontalArrangement = Arrangement.spacedBy(Metrics.s2),
  ) {
    Orb()
    // Named for a reader: "Starting <agent>…"
    val spoken = t("notice.connecting", "agent" to agentName)
    Text(
      t("devices.starting"), color = Palette.popFg2, fontSize = Type.text3, fontWeight = FontWeight.Medium, maxLines = 1,
      modifier = Modifier.semantics { contentDescription = spoken },
    )
  }
}

private const val HALF_TURN = 180f
private const val QUEUED_LINES = 3
private const val STARTING_DELAY_MS = 400L
private const val SCROLL_DROP_FROM = 0.6f
