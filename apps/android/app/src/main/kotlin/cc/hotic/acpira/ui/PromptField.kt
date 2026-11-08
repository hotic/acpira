package cc.hotic.acpira.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.input.InputTransformation
import androidx.compose.foundation.text.input.OutputTransformation
import androidx.compose.foundation.text.input.TextFieldLineLimits
import androidx.compose.foundation.text.input.TextFieldState
import androidx.compose.foundation.text.input.rememberTextFieldState
import androidx.compose.foundation.text.input.setTextAndPlaceCursorAtEnd
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.drawscope.clipRect
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalWindowInfo
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.style.TextOverflow
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.logic.PastedText
import cc.hotic.acpira.logic.SlashCommands
import cc.hotic.acpira.model.Draft
import kotlin.math.abs
import kotlin.math.max
import kotlin.math.min

// The prompt field the composer's card and its whole-screen editor share (apps/ios Chat/Composer.swift `field`,
// PromptMarks.swift, PastedText.swift): the text with the caret the `/` and `@` lists read, the command and summon
// pills, a long paste turned into a text attachment, and the field growing line by line up to its budget, then
// scrolling inside.

/**
 * The prompt as the field edits it, caret included. The app model holds the text alone; a change that did not come
 * from the field (a restored draft, a sent prompt, an edit taken in) puts the caret at the end, as the iOS field does
 */
@Composable
fun rememberPromptState(text: String, onText: (String) -> Unit): TextFieldState {
  val state = rememberTextFieldState(text)
  // What the field last handed out: the model's text catches up with it a frame later, which is no outside change
  val reported = remember { arrayOf(text) }
  val report by rememberUpdatedState(onText)
  LaunchedEffect(state) {
    snapshotFlow { state.text.toString() }.collect {
      if (it == reported[0]) return@collect
      reported[0] = it
      report(it)
    }
  }
  LaunchedEffect(text) {
    if (text == reported[0] || text == state.text.toString()) return@LaunchedEffect
    reported[0] = text
    state.setTextAndPlaceCursorAtEnd(text)
  }
  return state
}

/**
 * What a paste of pasted size does (PastedText.swift attachLongPastes): long text goes into the strip as a text
 * attachment and stays out of the field, too much is refused with a notice; true when the paste was taken
 */
@Composable
fun rememberLongPaste(staged: List<Staged>, onStaged: (List<Staged>) -> Unit, onToast: (String) -> Unit): (String) -> Boolean {
  val current by rememberUpdatedState(staged)
  val stage by rememberUpdatedState(onStaged)
  val toast by rememberUpdatedState(onToast)
  return remember {
    { pasted ->
      val name = t("attach.pastedText")
      when (PastedText.collect(pasted)) {
        PastedText.Result.Inline -> false
        PastedText.Result.Attachment -> { stage(current + Staged(Draft.Text(name, pasted), name)); true }
        PastedText.Result.TooBig -> { toast(t("attach.tooBigFile", "name" to name, "mb" to (PastedText.MAX_BYTES shr 20))); true }
      }
    }
  }
}

/**
 * The lines a growing field may take (LayoutRoom.lines): [most], or fewer where the keyboard leaves little room
 * (landscape), never under two, so the card stays under the top bar
 */
@Composable
fun composerLines(most: Int): Int {
  val density = LocalDensity.current
  val insets = WindowInsets.safeDrawing
  val visible = LocalWindowInfo.current.containerSize.height - insets.getTop(density) - insets.getBottom(density)
  val line = with(density) { Type.fieldLine.toPx() }
  if (visible <= 0 || line <= 0f) return most
  return min(most, max(2, (visible * Metrics.composerShare / line).toInt()))
}

/**
 * The field: as tall as its lines up to [maxLines], then scrolling inside (the field keeps the caret in view).
 * [onLongPaste] sees every user insertion of pasted size before it lands (a keyboard's clipboard chip pastes by
 * committing text, so the insertion is what counts; programmatic changes such as a restored draft never pass here).
 * [fill]: the whole-screen editor, the field taking the height it is given. [below] is drawn under the text (the
 * command's input hint). [onLines] reports the laid-out line count
 */
@OptIn(androidx.compose.foundation.ExperimentalFoundationApi::class)
@Composable
fun PromptField(
  state: TextFieldState,
  commands: List<String>,
  summons: List<String>,
  onLongPaste: (String) -> Boolean,
  maxLines: Int,
  modifier: Modifier = Modifier,
  fieldModifier: Modifier = Modifier,
  fill: Boolean = false,
  below: String? = null,
  onLines: (Int) -> Unit = {},
) {
  val density = LocalDensity.current
  val line = with(density) { Type.fieldLine.toDp() }
  val scroll = rememberScrollState()
  var layout by remember { mutableStateOf<TextLayoutResult?>(null) }
  val command = Palette.command
  val summon = Palette.summon
  val paste by rememberUpdatedState(onLongPaste)
  val lines by rememberUpdatedState(onLines)
  // One insertion of pasted size goes to the strip instead, the field left as it was
  val input = remember {
    InputTransformation {
      if (changes.changeCount != 1) return@InputTransformation
      val r = changes.getRange(0)
      if (r.length < PastedText.LINES - 1) return@InputTransformation
      if (paste(asCharSequence().substring(r.min, r.max))) revertAllChanges()
    }
  }
  // The marks' own colour over the text (the command blue, the summon violet)
  val tint = remember(commands, summons, command, summon) {
    OutputTransformation {
      for (m in SlashCommands.promptMarks(commands, summons, asCharSequence().toString())) {
        addStyle(SpanStyle(color = if (m.summon) summon else command), m.start, (m.start + m.length).coerceAtMost(length))
      }
    }
  }
  Column(modifier) {
    BasicTextField(
      state = state,
      inputTransformation = input,
      outputTransformation = tint,
      textStyle = TextStyle(color = Palette.fgStrong, fontSize = Type.field, lineHeight = Type.fieldLine),
      cursorBrush = SolidColor(Palette.fgStrong),
      keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Sentences),
      lineLimits = TextFieldLineLimits.MultiLine(1, if (fill) Int.MAX_VALUE else maxLines),
      scrollState = scroll,
      onTextLayout = { result -> result()?.let { layout = it; lines(it.lineCount) } },
      modifier = fieldModifier.fillMaxWidth().then(if (fill) Modifier.fillMaxSize() else Modifier.heightIn(min = line)),
      decorator = { inner ->
        Box(Modifier.drawBehind { pills(layout, state.text.toString(), commands, summons, scroll.value, command, summon) }) { inner() }
      },
    )
    // The command's own wording for what follows it, a faint second line while its arguments are empty
    if (below != null) {
      Text(
        below, color = Palette.fg3, fontSize = Type.mono, lineHeight = Type.monoLine, fontFamily = FontFamily.Monospace,
        maxLines = 1, overflow = TextOverflow.Ellipsis,
      )
    }
  }
}

/**
 * The pills behind the marks (PromptMarks.swift): bg-command/15 and bg-summon/15, one per line a token runs over, each
 * reaching past the name as far as the whitespace beside it allows (markReach). [scroll] is the field's own offset,
 * [font] the text's size (the field's, or a sent prompt's)
 */
internal fun androidx.compose.ui.graphics.drawscope.DrawScope.pills(
  layout: TextLayoutResult?, text: String, commands: List<String>, summons: List<String>, scroll: Int, command: Color, summon: Color,
  font: androidx.compose.ui.unit.TextUnit = Type.field,
) {
  val l = layout ?: return
  if (l.layoutInput.text.length != text.length) return
  val marks = SlashCommands.promptMarks(commands, summons, text)
  if (marks.isEmpty()) return
  val overhang = Metrics.markOverhang.toPx()
  val space = font.toPx() * SPACE_SHARE
  val clear = Metrics.markClear.toPx()
  val pad = Metrics.s0_5.toPx()
  val radius = CornerRadius(Metrics.rSm.toPx())
  // Clipped to the field's height only: the reach may overhang the text's sides into the card's padding
  clipRect(left = -size.width, right = size.width * 2) {
    marks.forEachIndexed { i, m ->
      val reach = SlashCommands.markReach(text, marks, i, overhang, space, clear)
      val ink = (if (m.summon) summon else command).copy(alpha = PILL_ALPHA)
      val end = (m.start + m.length).coerceAtMost(text.length)
      val first = l.getLineForOffset(m.start)
      val last = l.getLineForOffset((end - 1).coerceAtLeast(m.start))
      for (ln in first..last) {
        val from = if (ln == first) m.start else l.getLineStart(ln)
        val to = if (ln == last) end else l.getLineEnd(ln, visibleEnd = true)
        if (to <= from) continue
        val left = l.getBoundingBox(from).left
        val right = l.getBoundingBox(to - 1).right
        val mid = (l.getLineTop(ln) + l.getLineBottom(ln)) / 2 - scroll
        val h = min(l.getLineBottom(ln) - l.getLineTop(ln), font.toPx() * GLYPH_SHARE + pad * 2)
        drawRoundRect(ink, Offset(min(left, right) - reach, mid - h / 2), Size(abs(right - left) + reach * 2, h), radius)
      }
    }
  }
}

/** bg-command/15, bg-summon/15 */
private const val PILL_ALPHA = 0.15f
/** A space's width against the font size, and the glyphs' height (ascender to descender) against it */
private const val SPACE_SHARE = 0.25f
private const val GLYPH_SHARE = 1.2f
