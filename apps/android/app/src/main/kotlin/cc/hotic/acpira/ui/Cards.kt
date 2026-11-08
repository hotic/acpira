package cc.hotic.acpira.ui

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.wrapContentHeight
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.layout.SubcomposeLayout
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.dp
import cc.hotic.acpira.design.IconStore
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.design.VendorMark
import cc.hotic.acpira.design.WebMetrics
import cc.hotic.acpira.design.dockGap
import cc.hotic.acpira.design.optionKey
import cc.hotic.acpira.design.optionKeyLine
import cc.hotic.acpira.design.planBody
import cc.hotic.acpira.design.popActive
import cc.hotic.acpira.design.popFg2
import cc.hotic.acpira.design.popFg3
import cc.hotic.acpira.design.questionDescriptionGap
import cc.hotic.acpira.design.questionOptionPad
import cc.hotic.acpira.design.questionSectionGap
import cc.hotic.acpira.design.questionTopPad
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.logic.NoticeState
import cc.hotic.acpira.logic.NoticeWay
import cc.hotic.acpira.logic.PlanExecutor
import cc.hotic.acpira.logic.firstLine
import cc.hotic.acpira.logic.isContextLengthError
import cc.hotic.acpira.logic.permissionOption
import cc.hotic.acpira.logic.planHeading
import cc.hotic.acpira.logic.reviseOption
import cc.hotic.acpira.logic.variantName
import cc.hotic.acpira.model.AgentTurn
import cc.hotic.acpira.model.FailureAction
import cc.hotic.acpira.model.ModelPick
import cc.hotic.acpira.model.PermissionBlock
import cc.hotic.acpira.model.PermissionKind
import cc.hotic.acpira.model.PlanDocumentBlock
import cc.hotic.acpira.model.Question
import cc.hotic.acpira.model.QuestionBlock
import cc.hotic.acpira.model.QuestionKind
import cc.hotic.acpira.model.SessionControls
import cc.hotic.acpira.model.SessionOption
import cc.hotic.acpira.model.TurnStop
import cc.hotic.acpira.model.UserTurn
import kotlinx.coroutines.delay
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonPrimitive

// The chat's cards (apps/ios Chat/QuestionCard.swift, Chat/AlertCard.swift, Chat/SessionNotice.swift,
// Chat/PromptEdit.swift, Transcript/PlanDocumentCard.swift) on the composer bar's material: the question waiting for an
// answer, a turn that stopped short, a session that is not ready, the prompt being rewritten, and a saved plan.

// Surfaces

/**
 * The dock cards' surface (CardSurface): the composer bar's own material and corners, so a card floating over the thread
 * reads as the same thing as the bar under it
 */
@Composable
fun DockCard(modifier: Modifier = Modifier, content: @Composable ColumnScope.() -> Unit) {
  val shape = RoundedCornerShape(Metrics.rCardRound)
  Surface(
    color = Palette.bg1,
    shape = shape,
    shadowElevation = Metrics.barShadow,
    border = BorderStroke(Metrics.hairline, Palette.line),
    modifier = modifier.fillMaxWidth(),
  ) { Column(content = content) }
}

/**
 * A dock card's action (CardButton): a capsule; the way on is prominent in the send button's ink, and falls back to the
 * plain fill with faint text while it cannot run, so a disabled step never reads as a lit one. One line, never wrapped
 */
@Composable
fun CardButton(
  label: String,
  primary: Boolean,
  modifier: Modifier = Modifier,
  enabled: Boolean = true,
  onClick: () -> Unit,
  lead: (@Composable () -> Unit)? = null,
  trailing: (@Composable RowScope.() -> Unit)? = null,
) {
  val lit = primary && enabled
  Button(
    onClick = onClick,
    enabled = enabled,
    shape = CircleShape,
    colors = ButtonDefaults.buttonColors(
      containerColor = if (lit) Palette.fgStrong else Palette.active,
      contentColor = if (lit) Palette.bg0 else Palette.fg1,
      disabledContainerColor = Palette.active,
      disabledContentColor = Palette.popFg3,
    ),
    contentPadding = PaddingValues(horizontal = Metrics.pad, vertical = Metrics.s0),
    modifier = modifier.heightIn(min = Metrics.ctl),
  ) {
    if (lead != null) { lead(); Spacer(Modifier.width(Metrics.s1_5)) }
    Text(label, fontSize = Type.text2, fontWeight = if (lit) FontWeight.Medium else FontWeight.Normal, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f, fill = false))
    trailing?.invoke(this)
  }
}

/** A dock card's corner control (close, retry): a small round button, as a system sheet's close button */
@Composable
fun CardCornerButton(icon: String, label: String, modifier: Modifier = Modifier, onClick: () -> Unit) {
  Box(
    modifier.size(Metrics.ctlSm).clip(CircleShape).background(Palette.active).clickable(onClick = onClick),
    contentAlignment = Alignment.Center,
  ) {
    Lucide(icon, Palette.fg1, size = Metrics.icon, contentDescription = label)
  }
}

/**
 * A dock card's action row (CardActions): the quiet copy button at the start, the ways on at the end where a thumb
 * reaches. The copy label falls back to its shorter forms before the row breaks ([copy]: longest first, empty for no
 * copy button); when no form fits one row the ways drop under the copy button, still at the end
 */
@Composable
fun CardActions(copy: List<String>, onCopy: () -> Unit, modifier: Modifier = Modifier, ways: @Composable RowScope.() -> Unit) {
  val gap = with(LocalDensity.current) { Metrics.s2.roundToPx() }
  SubcomposeLayout(modifier.fillMaxWidth()) { constraints ->
    val loose = constraints.copy(minWidth = 0, minHeight = 0)
    val width = constraints.maxWidth
    val waysP = subcompose("ways") { Row(horizontalArrangement = Arrangement.spacedBy(Metrics.s2), verticalAlignment = Alignment.CenterVertically, content = ways) }
      .map { it.measure(loose) }
    val waysW = waysP.maxOfOrNull { it.width } ?: 0
    val waysH = waysP.maxOfOrNull { it.height } ?: 0
    if (copy.isEmpty()) {
      return@SubcomposeLayout layout(width, waysH) { waysP.forEach { it.place(width - it.width, 0) } }
    }
    // The longest label that leaves the ways room on its row, else the longest that fits a row of its own
    var chosen: androidx.compose.ui.layout.Placeable? = null
    var stacked = false
    for ((i, label) in copy.withIndex()) {
      val p = subcompose("copy$i") { CardButton(label, primary = false, onClick = onCopy) }.first().measure(loose)
      if (p.width + gap + waysW <= width) { chosen = p; break }
    }
    if (chosen == null) {
      stacked = true
      for ((i, label) in copy.withIndex()) {
        val p = subcompose("stacked$i") { CardButton(label, primary = false, onClick = onCopy) }.first()
          .measure(loose.copy(maxWidth = width))
        chosen = p
        if (p.width <= width) break
      }
    }
    val c = chosen!!
    if (!stacked) {
      val h = maxOf(c.height, waysH)
      layout(width, h) {
        c.place(0, (h - c.height) / 2)
        waysP.forEach { it.place(width - it.width, (h - it.height) / 2) }
      }
    } else {
      layout(width, c.height + gap + waysH) {
        c.place(0, 0)
        waysP.forEach { it.place(width - it.width, c.height + gap) }
      }
    }
  }
}

/** A dock card's title line: the glyph, the title, and an optional corner control pulled into the card's padding */
@Composable
private fun CardHead(icon: String, ink: androidx.compose.ui.graphics.Color, title: String, oneLine: Boolean, corner: (@Composable () -> Unit)?) {
  Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(Metrics.s2)) {
    Lucide(icon, ink, size = Metrics.icon)
    Text(
      title, color = Palette.fgStrong, fontSize = Type.text2, lineHeight = Type.text2Line, fontWeight = FontWeight.SemiBold,
      maxLines = if (oneLine) 1 else Int.MAX_VALUE, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f),
    )
    corner?.invoke()
  }
}

/** "Copied" for a moment after a copy */
@Composable
private fun rememberCopied(): Pair<Boolean, () -> Unit> {
  var copied by remember { mutableIntStateOf(0) }
  LaunchedEffect(copied) {
    if (copied > 0) { delay(COPIED_MS); copied = 0 }
  }
  return (copied > 0) to { copied += 1 }
}

// Question

/**
 * The open question card docked above the composer (QuestionCard.swift, chat/Questions.tsx): one question per page,
 * lettered options, an Other field, Skip on the left and Next / Submit on the right of the footer
 */
@Composable
fun QuestionCard(block: QuestionBlock, onAnswer: (Map<String, JsonElement>, Boolean) -> Unit) {
  val qs = block.questions
  if (qs.isEmpty()) return
  val picks = remember(block.id) { mutableStateMapOf<String, Pick>() }
  var page by remember(block.id) { mutableIntStateOf(0) }
  val index = page.coerceIn(0, qs.size - 1)
  val current = qs[index]
  val last = index == qs.size - 1
  val answers = collected(qs, picks)
  val canSubmit = answers.isNotEmpty() && qs.all { it.required != true || answers[it.id] != null }
  val canAdvance = if (last) canSubmit else current.required != true || answers[current.id] != null
  // --question-body-max: min(--row × 8, 35dvh)
  val maxBody = minOf(Metrics.row * QUESTION_ROWS, LocalConfiguration.current.screenHeightDp.dp * DOCK_SHARE)
  DockCard(Modifier.testTag("question")) {
    Column(
      Modifier.heightIn(max = maxBody).verticalScroll(rememberScrollState())
        .padding(start = Metrics.pad, end = Metrics.pad, top = Metrics.questionTopPad, bottom = Metrics.questionSectionGap),
      verticalArrangement = Arrangement.spacedBy(Metrics.questionSectionGap),
    ) {
      AcpRow(
        dense = true, leadTop = true, icon = "messageCircleQuestion",
        trailing = { Text(t("question.of", "n" to index + 1, "total" to qs.size), maxLines = 1, softWrap = false) },
      ) {
        Text(current.text, color = Palette.fgStrong, fontSize = Type.text2, lineHeight = Type.text2Line, fontWeight = FontWeight.Medium)
      }
      if (index == 0) block.message?.let { Text(it, color = Palette.popFg2, fontSize = Type.text2, lineHeight = Type.text2Line) }
      QuestionOptions(current, picks)
    }
    Row(
      Modifier.fillMaxWidth().padding(start = Metrics.pad, end = Metrics.pad, top = Metrics.s1, bottom = Metrics.pad),
      horizontalArrangement = Arrangement.spacedBy(Metrics.gap),
      verticalAlignment = Alignment.CenterVertically,
    ) {
      CardButton(t("question.skip"), primary = false, modifier = Modifier.testTag("question.skip"), onClick = { onAnswer(answers, true) })
      Spacer(Modifier.weight(1f))
      if (index > 0) CardButton(t("question.prev"), primary = false, onClick = { page = index - 1 })
      CardButton(
        t(if (last) "question.submit" else "question.next"), primary = true, enabled = canAdvance,
        modifier = Modifier.testTag(if (last) "question.submit" else "question.next"),
        onClick = { if (last) onAnswer(answers, false) else page = index + 1 },
      )
    }
  }
}

/** One question's picks: chosen option ids, the Other field's state and its text */
data class Pick(val options: List<String> = emptyList(), val other: Boolean = false, val text: String = "")

private val KEYS = ('A'..'Z').map { it.toString() }

@Composable
private fun QuestionOptions(q: Question, picks: MutableMap<String, Pick>) {
  val pick = picks[q.id] ?: Pick()
  Column {
    if (q.kind == QuestionKind.multiple) {
      Text(t("question.multiple"), color = Palette.popFg2, fontSize = Type.text3, lineHeight = Type.text3Line, modifier = Modifier.padding(bottom = Metrics.questionSectionGap))
    }
    q.options.forEachIndexed { i, option ->
      val selected = option.id in pick.options
      OptionRow(KEYS.getOrElse(i) { "${i + 1}" }, selected, Modifier.testTag("question.option:$i"), onClick = {
        picks[q.id] = if (q.kind == QuestionKind.multiple) {
          pick.copy(options = if (selected) pick.options - option.id else pick.options + option.id)
        } else pick.copy(other = false, options = listOf(option.id))
      }) {
        Column(verticalArrangement = Arrangement.spacedBy(Metrics.questionDescriptionGap)) {
          Text(option.label, color = Palette.fg1, fontSize = Type.text2, lineHeight = Type.text2Line)
          option.description?.let { Text(it, color = Palette.popFg2, fontSize = Type.text3, lineHeight = Type.text3Line) }
        }
      }
    }
    if (q.kind != QuestionKind.text && q.other == true) {
      OptionRow(KEYS.getOrElse(q.options.size) { "" }, pick.other, leadTop = false, onClick = null) {
        AnswerField(pick.text, t("question.other"), Modifier.testTag("question.other")) { text ->
          picks[q.id] = pick.copy(text = text, other = true, options = if (q.kind == QuestionKind.single) emptyList() else pick.options)
        }
      }
    }
    if (q.kind == QuestionKind.text) {
      val shape = RoundedCornerShape(Metrics.rMd)
      Box(
        Modifier.fillMaxWidth().heightIn(min = Metrics.ctl).border(Metrics.hairline, Palette.lineStrong, shape).padding(horizontal = Metrics.gap),
        contentAlignment = Alignment.CenterStart,
      ) {
        AnswerField(pick.text, t(if (q.numeric == true) "question.numberPlaceholder" else "question.textPlaceholder"), Modifier.testTag("question.text")) {
          picks[q.id] = pick.copy(text = it)
        }
      }
    }
  }
}

/** A one-line answer field with its hint drawn in place */
@Composable
private fun AnswerField(text: String, hint: String, modifier: Modifier = Modifier, onText: (String) -> Unit) {
  Box(modifier.fillMaxWidth(), contentAlignment = Alignment.CenterStart) {
    if (text.isEmpty()) Text(hint, color = Palette.popFg2, fontSize = Type.text2, maxLines = 1, overflow = TextOverflow.Ellipsis)
    BasicTextField(
      value = text, onValueChange = onText, singleLine = true,
      textStyle = TextStyle(color = Palette.fg1, fontSize = Type.text2),
      cursorBrush = SolidColor(Palette.fgStrong),
      modifier = Modifier.fillMaxWidth(),
    )
  }
}

/**
 * A lettered option (OptionRow): the key ring (a check once picked) in the lead slot, the label column beside it. Its
 * fill bleeds outward, so its text stays on the header's text line instead of the fill pushing it in
 */
@Composable
private fun OptionRow(
  key: String,
  selected: Boolean,
  modifier: Modifier = Modifier,
  leadTop: Boolean = true,
  onClick: (() -> Unit)?,
  content: @Composable () -> Unit,
) {
  val fill = Palette.active
  val density = LocalDensity.current
  val bleed = with(density) { (Metrics.questionOptionPad + (Metrics.lead - WebMetrics.icon) / 2).toPx() }
  val radius = with(density) { Metrics.rMd.toPx() }
  val source = remember { MutableInteractionSource() }
  val pressed by source.collectIsPressedAsState()
  val on = selected || pressed
  CompositionLocalProvider(LocalRowHeight provides Metrics.row - Metrics.questionOptionPad * 2) {
    AcpRow(
      modifier
        .drawBehind { if (on) drawRoundRect(fill, Offset(-bleed, 0f), Size(size.width + bleed * 2, size.height), CornerRadius(radius)) }
        .then(if (onClick != null) Modifier.clickable(source, indication = null, onClick = onClick) else Modifier)
        .padding(vertical = Metrics.questionOptionPad),
      leadTop = leadTop,
      lead = {
        Box(Modifier.size(Metrics.icon).border(Metrics.hairline, Palette.lineStrong, CircleShape), contentAlignment = Alignment.Center) {
          if (selected) Glyph("check", Palette.fg1, WebMetrics.chevron, cc.hotic.acpira.design.IconStroke.outcome)
          else Text(key, color = Palette.popFg2, fontSize = Type.optionKey, lineHeight = Type.optionKeyLine, fontWeight = FontWeight.Medium)
        }
      },
    ) { Box(Modifier.weight(1f)) { content() } }
  }
}

/** What the card sends: answered questions only, a string or a list of option ids (and Other's text) each */
private fun collected(qs: List<Question>, picks: Map<String, Pick>): Map<String, JsonElement> {
  val out = LinkedHashMap<String, JsonElement>()
  for (q in qs) {
    val p = picks[q.id] ?: Pick()
    val text = p.text.trim()
    when (q.kind) {
      QuestionKind.text -> if (text.isNotEmpty()) out[q.id] = JsonPrimitive(text)
      QuestionKind.single -> if (p.other) { if (text.isNotEmpty()) out[q.id] = JsonPrimitive(text) } else p.options.firstOrNull()?.let { out[q.id] = JsonPrimitive(it) }
      QuestionKind.multiple -> {
        val list = p.options + if (p.other && text.isNotEmpty()) listOf(text) else emptyList()
        if (list.isNotEmpty()) out[q.id] = JsonArray(list.map(::JsonPrimitive))
      }
    }
  }
  return out
}

// Alert

/**
 * A turn that stopped short (AlertCard.swift, Alert.tsx): what happened and ✕ on the first line, the agent's words under
 * it, then the actions on their own row: the copyable detail as a quiet button at the start, the way on as the one
 * filled button at the end. The way on is retry for an error (plus "continue in a new session" when the failure names
 * it), carry on for a limit, nothing for a refusal; a context that grew too long offers compaction instead
 */
@Composable
fun AlertCard(
  turn: AgentTurn,
  onDismiss: () -> Unit,
  onRetry: () -> Unit,
  onContinue: () -> Unit,
  /** Only when the agent advertises `/compact` */
  onCompact: (() -> Unit)?,
  onNewSession: () -> Unit,
) {
  val context = LocalContext.current
  val stop = turn.stop ?: TurnStop.error
  val err = turn.error
  val contextTooLong = stop == TurnStop.error && isContextLengthError(err)
  val detail = listOfNotNull(err?.code?.toString(), err?.kind).filter { it.isNotEmpty() }.joinToString(t("common.metaSep"))
  val copyable = listOfNotNull(err?.message, detail).filter { it.isNotEmpty() }.joinToString("\n")
  val message = when {
    contextTooLong -> t(if (onCompact != null) "alert.contextLength.text" else "alert.contextLength.unsupported")
    stop == TurnStop.error -> err?.message?.takeIf { it.isNotEmpty() } ?: t("alert.error.unknown")
    stop == TurnStop.refusal -> t("alert.refusal.text")
    stop == TurnStop.max_tokens -> t("alert.maxTokens.text")
    stop == TurnStop.max_turn_requests -> t("alert.maxTurns.text")
    else -> null
  }
  val title = when {
    contextTooLong -> t("alert.contextLength.title")
    stop == TurnStop.refusal -> t("alert.refusal.title")
    stop == TurnStop.max_tokens -> t("alert.maxTokens.title")
    stop == TurnStop.max_turn_requests -> t("alert.maxTurns.title")
    else -> t("alert.error.title")
  }
  val actions = err?.actions
  val retry = stop == TurnStop.error && !contextTooLong && (actions == null || FailureAction.retry in actions)
  val fresh = stop == TurnStop.error && !contextTooLong && actions?.contains(FailureAction.new_session) == true
  val carry = stop == TurnStop.max_tokens || stop == TurnStop.max_turn_requests
  val compact = if (contextTooLong) onCompact else null
  val (copied, markCopied) = rememberCopied()
  DockCard(Modifier.testTag("alert")) {
    Column(Modifier.padding(horizontal = Metrics.pad, vertical = Metrics.s3), verticalArrangement = Arrangement.spacedBy(Metrics.s2)) {
      CardHead("triangleAlert", Palette.warn, title, oneLine = false) {
        CardCornerButton("x", t("common.close"), Modifier.testTag("alert.close"), onDismiss)
      }
      message?.let { Text(it, color = Palette.popFg2, fontSize = Type.text3, lineHeight = Type.text3Line) }
      if (copyable.isNotEmpty() || retry || fresh || carry || compact != null) {
        val labels = if (copyable.isEmpty()) emptyList() else if (copied) listOf(t("alert.copied")) else listOfNotNull(
          if (detail.isEmpty()) null else t("alert.copyWith", "detail" to detail), t("alert.copy"),
        )
        CardActions(labels, onCopy = { copyText(context, copyable); markCopied() }, Modifier.padding(top = Metrics.s1)) {
          if (fresh) CardButton(t("notice.continueNew"), primary = false, onClick = onNewSession)
          if (retry) CardButton(t("common.retry"), primary = true, modifier = Modifier.testTag("alert.retry"), onClick = onRetry)
          if (carry) CardButton(t("alert.continue"), primary = true, modifier = Modifier.testTag("alert.continue"), onClick = onContinue)
          if (compact != null) CardButton(t("alert.contextLength.compact"), primary = true, onClick = compact)
        }
      }
    }
  }
}

// Session notice

/**
 * The session card above the composer (SessionNotice.swift, Notice.tsx) in the failure card's layout: what is wrong and
 * retry on the first line, the explanation under it, then the actions on their own row, the copyable detail first and
 * the way on last
 */
@Composable
fun SessionNoticeCard(state: NoticeState, onWay: (NoticeWay) -> Unit) {
  val context = LocalContext.current
  val (copied, markCopied) = rememberCopied()
  DockCard(Modifier.testTag("notice")) {
    Column(Modifier.padding(horizontal = Metrics.pad, vertical = Metrics.s3), verticalArrangement = Arrangement.spacedBy(Metrics.s2)) {
      CardHead(state.icon, Palette.popFg3, state.title, oneLine = true, corner = if (!state.showRetry) null else ({
        CardCornerButton("rotateCw", t("common.retry"), Modifier.testTag("notice.retry")) { onWay(NoticeWay.Restart) }
      }))
      Text(state.text, color = Palette.popFg2, fontSize = Type.text3, lineHeight = Type.text3Line)
      if (state.detail != null || state.primary != null || state.others.isNotEmpty()) {
        val labels = if (state.detail == null) emptyList() else listOf(t(if (copied) "alert.copied" else "alert.copy"))
        CardActions(labels, onCopy = { copyText(context, state.detail.orEmpty()); markCopied() }, Modifier.padding(top = Metrics.s1)) {
          if (state.others.size == 1) {
            val other = state.others[0]
            CardButton(other.label, primary = false, onClick = { onWay(other) })
          } else if (state.others.size > 1) {
            var menu by remember { mutableStateOf(false) }
            Box {
              CardButton(t("notice.moreWays"), primary = false, onClick = { menu = true }, trailing = {
                Spacer(Modifier.width(Metrics.s1))
                Lucide("chevronDown", Palette.popFg3, size = Metrics.chevron)
              })
              AppMenu(expanded = menu, onDismiss = { menu = false }) {
                for (way in state.others) AppMenuItem(way.label, onClick = { menu = false; onWay(way) })
              }
            }
          }
          state.primary?.let { p -> CardButton(p.label, primary = true, modifier = Modifier.testTag("notice.primary"), onClick = { onWay(p) }) }
        }
      }
    }
  }
}

// Prompt edit

/**
 * The line above the composer while a sent prompt is being rewritten (PromptEdit.swift EditingBar): what it was, and ✕ to
 * give the composer back
 */
@Composable
fun EditingBar(original: UserTurn, onCancel: () -> Unit) {
  DockCard(Modifier.testTag("edit.bar")) {
    Row(
      Modifier.fillMaxWidth().heightIn(min = Metrics.row).padding(start = Metrics.pad, end = Metrics.s1, top = Metrics.s1, bottom = Metrics.s1),
      verticalAlignment = Alignment.CenterVertically,
      horizontalArrangement = Arrangement.spacedBy(Metrics.s2),
    ) {
      Lucide("pencil", Palette.fg3, size = Metrics.icon)
      Spacer(Modifier.width(Metrics.s1))
      Text(t("mobile.edit.title"), color = Palette.fg1, fontSize = Type.text2, fontWeight = FontWeight.Medium, maxLines = 1, softWrap = false)
      Text(
        firstLine(original.text).orEmpty(), color = Palette.popFg3, fontSize = Type.text2, maxLines = 1,
        overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f),
      )
      CardCornerButton("x", t("history.cancel"), Modifier.testTag("edit.cancel"), onCancel)
    }
  }
}

// Plan document

/** What the plan card reads from its session (PlanDocumentContext) */
data class PlanContext(val controls: SessionControls = SessionControls(), val running: Boolean = false, val ready: Boolean = false)

val LocalPlanContext = androidx.compose.runtime.compositionLocalOf { PlanContext() }

/**
 * A saved plan as its own card between the work before it and the continuation after it (PlanDocumentCard.swift,
 * PlanDocument.tsx). The card is the plan itself, its opening faded at a cap with the whole plan one tap away (openPlan
 * on the phone: the code sheet with a copy), then an action bar while it can still be built: the execution model with
 * the extra approvals behind one chip, and Build. Revising is a message: the composer's send answers the plan's revise
 * option
 */
@Composable
fun PlanDocumentCard(
  block: PlanDocumentBlock,
  /** The approval the agent waits on for this plan, when it asks for one */
  permission: PermissionBlock?,
  onBuild: (option: String?, model: ModelPick?) -> Unit,
  onPermission: (blockId: String, option: String) -> Unit,
) {
  val (title, body) = remember(block.markdown, block.title) { planHeading(block) }
  DockCard(Modifier.testTag("plan.card")) {
    if (body.isNotEmpty()) PlanBody(body, title, block.markdown)
    if (block.status != "approved" && block.status != "executing") PlanBar(block, permission, onBuild, onPermission)
  }
}

/** The plan's opening at the card's inset, faded at the cap; a tap opens the whole plan with a copy button */
@Composable
private fun PlanBody(text: String, title: String, markdown: String) {
  var natural by remember { mutableIntStateOf(0) }
  var open by remember { mutableStateOf(false) }
  val density = LocalDensity.current
  val cap = with(density) { Metrics.planBody.toPx() }
  val clipped = natural > cap
  val fade = with(density) { WebMetrics.scrollFade.toPx() }
  val foot = with(density) { (Type.text3Line.toDp() + Metrics.s1).toPx() }
  val ground = Palette.bg1
  Box(
    Modifier.fillMaxWidth().clickable(remember { MutableInteractionSource() }, indication = null) { open = true }
      .padding(Metrics.pad).testTag("plan.body"),
  ) {
    Box(
      Modifier.fillMaxWidth().heightIn(max = Metrics.planBody).clipToBounds().drawWithContent {
        drawContent()
        if (clipped) {
          // The cut fades the plan into the card, and the way to the rest sits in the room the fade leaves clear
          drawRect(ground, topLeft = Offset(0f, size.height - foot), size = Size(size.width, foot))
          drawRect(
            Brush.verticalGradient(listOf(ground.copy(alpha = 0f), ground), startY = size.height - foot - fade, endY = size.height - foot),
            topLeft = Offset(0f, size.height - foot - fade), size = Size(size.width, fade),
          )
        }
      },
    ) {
      Box(Modifier.fillMaxWidth().wrapContentHeight(Alignment.Top, unbounded = true).onSizeChanged { natural = it.height }) { Prose(text) }
    }
    if (clipped) {
      Text(
        t("plan.expand"), color = Palette.popFg2, fontSize = Type.text3, lineHeight = Type.text3Line,
        textAlign = androidx.compose.ui.text.style.TextAlign.Center, modifier = Modifier.fillMaxWidth().align(Alignment.BottomCenter),
      )
    }
  }
  if (open) CodeSheet(title, markdown, t("mobile.plan.copy"), onDismiss = { open = false }) { Prose(text) }
}

/** The actions: the executor chip, then Build as the card's one primary action */
@Composable
private fun PlanBar(block: PlanDocumentBlock, permission: PermissionBlock?, onBuild: (String?, ModelPick?) -> Unit, onPermission: (String, String) -> Unit) {
  val ctx = LocalPlanContext.current
  val context = LocalContext.current
  var selected by remember(block.id) { mutableStateOf<String?>(null) }
  val brandOf = remember { { o: SessionOption -> IconStore.brandOf(context, o.id) ?: IconStore.brandOf(context, o.name) } }
  val executor = remember(ctx.controls, selected) { PlanExecutor(ctx.controls, selected, brandOf) }
  val busy = !ctx.ready || (ctx.running && permission == null) || block.status == "draft" || block.status == "executing"
  val canBuild = !busy && block.markdown.isNotEmpty()
  val primary = permission?.options?.firstOrNull { it.kind == PermissionKind.allow_once }
  val revise = permission?.let(::reviseOption)
  val extra = permission?.options.orEmpty().filter { it.id != primary?.id && it.id != revise?.id }
  Row(
    Modifier.fillMaxWidth().padding(start = Metrics.pad, end = Metrics.pad, bottom = Metrics.pad),
    horizontalArrangement = Arrangement.spacedBy(Metrics.gap, Alignment.End),
    verticalAlignment = Alignment.CenterVertically,
  ) {
    if (executor.control != null || extra.isNotEmpty()) {
      Box(Modifier.weight(1f, fill = false)) {
        ExecutorMenu(executor, extra, canBuild, ctx.ready, onPick = { selected = it }, onOption = { option ->
          val allow = option.kind == PermissionKind.allow_once || option.kind == PermissionKind.allow_always
          if (allow) onBuild(option.id, executor.pick) else permission?.let { onPermission(it.id, option.id) }
        })
      }
    }
    CardButton(
      primary?.let { permissionOption(it, plan = true).label } ?: t("plan.build"), primary = true, enabled = canBuild,
      modifier = Modifier.testTag("plan.build"), onClick = { onBuild(primary?.id, executor.pick) },
    )
  }
}

/**
 * The executor chip: model families (a family with several variants opens its variants), then the approvals beyond
 * Build. An allow option builds with the chosen executor; any other answers the approval as it is
 */
@Composable
private fun ExecutorMenu(
  executor: PlanExecutor,
  extra: List<cc.hotic.acpira.model.PermissionOption>,
  canBuild: Boolean,
  ready: Boolean,
  onPick: (String) -> Unit,
  onOption: (cc.hotic.acpira.model.PermissionOption) -> Unit,
) {
  var menu by remember { mutableStateOf(false) }
  // The family whose variants are open, null at the top level
  var level by remember { mutableStateOf<String?>(null) }
  val family = executor.family
  CardButton(
    family?.name ?: executor.label ?: t("plan.approvals"), primary = false,
    modifier = Modifier.semantics { contentDescription = t("plan.approvalsAria") }.testTag("plan.executor"),
    onClick = { level = null; menu = true },
    lead = family?.let { f -> { VendorMark(f.brand, f.name, Palette.fg1, size = Metrics.icon) } },
    trailing = executor.meta?.let { meta -> {
      Spacer(Modifier.width(Metrics.s1_5))
      Text(meta, color = Palette.popFg3, fontSize = Type.text2, maxLines = 1, softWrap = false)
    } },
  )
  AppMenu(expanded = menu, onDismiss = { menu = false }) {
    val open = executor.families.firstOrNull { it.key == level }
    if (open != null) {
      AppMenuBack(open.name, onClick = { level = null })
      for (v in open.variants) AppMenuItem(variantName(v), onClick = { menu = false; onPick(v.id) }, checked = v.id == executor.value)
      return@AppMenu
    }
    if (executor.families.isNotEmpty()) {
      MenuCaption(t("plan.executor"))
      for (f in executor.families) {
        val on = f.key == family?.key
        if (f.variants.size > 1) {
          AppMenuItem(f.name, onClick = { level = f.key }, checked = on, icon = { MenuGlyph("chevronRight") })
        } else f.variants.firstOrNull()?.let { only ->
          AppMenuItem(f.name, onClick = { menu = false; onPick(only.id) }, checked = only.id == executor.value)
        }
      }
    }
    if (extra.isNotEmpty()) {
      if (executor.families.isNotEmpty()) AppMenuGap()
      MenuCaption(t("plan.moreApprovals"))
      for (option in extra) {
        val allow = option.kind == PermissionKind.allow_once || option.kind == PermissionKind.allow_always
        val enabled = if (allow) canBuild else ready
        AppMenuItem(
          permissionOption(option, plan = true).label,
          onClick = { if (enabled) { menu = false; onOption(option) } },
          modifier = Modifier.then(if (enabled) Modifier else Modifier.padding(Metrics.s0)),
        )
      }
    }
  }
}

/** A menu section's header, as UIMenu's section titles */
@Composable
private fun MenuCaption(text: String) {
  Text(
    text, color = Palette.fg3, fontSize = Type.text3, lineHeight = Type.text3Line, maxLines = 1, overflow = TextOverflow.Ellipsis,
    modifier = Modifier.fillMaxWidth().padding(horizontal = Metrics.pad, vertical = Metrics.s1_5),
  )
}

@Composable
private fun androidx.compose.ui.unit.TextUnit.toDp() = with(LocalDensity.current) { this@toDp.toDp() }

/** The share of the window's height the docked cards' bodies may take (Metrics.dockShare) */
internal const val DOCK_SHARE = 0.35f
private const val QUESTION_ROWS = 8
private const val COPIED_MS = 1500L
