package cc.hotic.acpira.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.snap
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.Row
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
import cc.hotic.acpira.design.Haptic
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
import cc.hotic.acpira.logic.toolVerb
import cc.hotic.acpira.model.DiffLine
import cc.hotic.acpira.model.DiffSource
import cc.hotic.acpira.model.ToolCallBlock
import cc.hotic.acpira.model.ToolContent
import cc.hotic.acpira.ui.prose.codeRuns
import cc.hotic.acpira.ui.prose.rememberDiffTokens
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.layout.layout
import androidx.compose.ui.semantics.contentDescription
import cc.hotic.acpira.model.FullDiffSource
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlin.math.ceil
import kotlin.math.max
import kotlin.math.roundToInt

// Code cards

/**
 * The `code-output` frame (rounded-md, a hairline, the code background) around a card that opens whole in the code
 * sheet on a tap; inside the sheet the card stays as it is
 */
@Composable
internal fun CodeFrame(
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
internal fun CodeCap(content: @Composable () -> Unit) {
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
internal fun TerminalOutput(block: ToolCallBlock, command: String?) {
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
  val holdScroll = LocalHoldScroll.current
  val progress by animateFloatAsState(if (expanded) 1f else 0f, if (motion) RowMotion.fold(PROMPT_STEP) else snap(), label = "prompt")
  val style = WebType.mono
  Row(
    Modifier.fillMaxWidth()
      .then(if (clipped) Modifier.clickable(remember { MutableInteractionSource() }, indication = null) { holdScroll(); expanded = !expanded } else Modifier)
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
internal fun ToolOutput(text: String) {
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
internal fun DiffBlockView(lines: List<DiffLine>, title: String, source: DiffSource?, locate: DiffLocation? = null) {
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
internal fun diffCode(line: DiffLine) = if (line.kind == DiffLine.Kind.hunk) line.text else line.text.drop(1)

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
internal fun diffLocation(block: ToolCallBlock, diff: ToolContent.Diff): DiffLocation? {
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
