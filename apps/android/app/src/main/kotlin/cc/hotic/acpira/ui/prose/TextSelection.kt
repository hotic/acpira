package cc.hotic.acpira.ui.prose

import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionState
import androidx.compose.material3.Text
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.waitForUpOrCancellation
import androidx.compose.ui.composed
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Rect
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.layout.LayoutCoordinates
import androidx.compose.ui.layout.boundsInRoot
import androidx.compose.ui.layout.onGloballyPositioned
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.launch
import androidx.compose.runtime.movableContentOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.style.TextOverflow
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.logic.MdInline
import cc.hotic.acpira.ui.AppMenu
import cc.hotic.acpira.ui.AppMenuGap
import cc.hotic.acpira.ui.AppMenuItem
import cc.hotic.acpira.ui.MenuGlyph
import androidx.compose.foundation.text.contextmenu.builder.item
import androidx.compose.foundation.text.contextmenu.data.TextContextMenuKeys
import androidx.compose.foundation.text.contextmenu.modifier.appendTextContextMenuComponents
import androidx.compose.foundation.text.contextmenu.modifier.filterTextContextMenuComponents
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.text.selection.rememberSelectionState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.AnnotatedString
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.logic.Markdown
import cc.hotic.acpira.logic.MdBlock
import cc.hotic.acpira.ui.copyText

// Ported from apps/ios Prose/TextSelection.swift and Chat/Quotes.swift (QuoteToolbar.tsx on a touch screen). One
// Markdown block owns its menu: a long press on a reply's block opens its whole-source actions (Quote, Copy) over a
// preview of it, and Select text hands the block to range selection, whose menu quotes just the selected text into the
// composer ("Add to chat") besides Copy. Copy and Quote take the source text: the renderer's pads and break hints never
// reach the clipboard.

/** The selected source text, excluding the runs the renderer added (pads, break hints), not the source's own */
internal fun selectionText(texts: List<AnnotatedString>): String = texts.joinToString("\n") { text ->
  val marks = BooleanArray(text.length)
  for (d in text.getStringAnnotations(DECORATION_TAG, 0, text.length)) for (k in d.start until d.end) marks[k] = true
  buildString { text.forEachIndexed { k, c -> if (!marks[k] && c != '\u200B' && c != '\u2060') append(c) } }
}

/** Blocks with text to select: diagrams, formulas and image-only blocks have none (Quotes.swift offersSelection) */
private fun offersSelection(block: MdBlock): Boolean = when (block) {
  is MdBlock.Math, MdBlock.Rule -> false
  is MdBlock.Code -> block.lang != "mermaid"
  is MdBlock.Paragraph -> block.inlines.none { it is MdInline.Image }
  else -> true
}

/**
 * A block's menu and selection surface. `onQuote` is what reply text quotes into (the chat's composer): with one, a
 * long press opens the block's menu and range selection waits for Select text; without one (a subagent's transcript) a
 * long press selects within the block and the menu offers Copy alone
 */
@Composable
fun BlockSelection(block: MdBlock, onQuote: ((String) -> Unit)?, content: @Composable () -> Unit) {
  if (block is MdBlock.Rule) { content(); return }
  val selectable = offersSelection(block)
  if (onQuote == null) {
    if (selectable) SelectableBlock(block, null, rememberSelectionState(), content) else content()
    return
  }
  val haptics = LocalHapticFeedback.current
  val context = LocalContext.current
  var menu by remember { mutableStateOf(false) }
  // The block handed to range selection (Select text) until its selection is dropped
  var selecting by remember { mutableStateOf(false) }
  val state = rememberSelectionState()
  // The block's views move between the plain and the selecting tree, so a selection keeps their state (a code card's
  // scroll, a loaded image)
  val current by rememberUpdatedState(content)
  val body = remember { movableContentOf { current() } }
  val bounds = remember { arrayOfNulls<Rect>(1) }
  Box(
    Modifier.onGloballyPositioned { bounds[0] = it.boundsInRoot() }.then(if (selecting) Modifier else Modifier.pointerInput(Unit) {
      detectTapGestures(onLongPress = { haptics.performHapticFeedback(HapticFeedbackType.LongPress); menu = true })
    }),
  ) {
    if (selecting) SelectableBlock(block, onQuote, state, body) else body()
    AppMenu(expanded = menu, onDismiss = { menu = false }) {
      // Only the preview is shortened: Copy and Quote take the complete block
      Text(
        QuoteText.of(block), color = Palette.fg1, fontSize = Type.text2, lineHeight = Type.text2Line,
        maxLines = Metrics.messagePreviewLines, overflow = TextOverflow.Ellipsis,
        modifier = Modifier.padding(horizontal = Metrics.s2).background(Palette.bg2, RoundedCornerShape(Metrics.rMd))
          .padding(Metrics.s3).testTag("block.preview"),
      )
      AppMenuGap()
      AppMenuItem(t("quote.add"), onClick = { menu = false; onQuote(QuoteText.of(block)) }, modifier = Modifier.testTag("block.quote"), icon = { MenuGlyph("messageSquareText") })
      AppMenuItem(t("common.copy"), onClick = { menu = false; copyText(context, QuoteText.of(block)) }, modifier = Modifier.testTag("block.copy"), icon = { MenuGlyph("copy") })
      if (selectable) {
        AppMenuItem(t("history.selectText"), onClick = { menu = false; selecting = true }, modifier = Modifier.testTag("block.select"), icon = { MenuGlyph("textSelect") })
      }
    }
  }
  SelectAllWhile(selecting, state, { bounds[0] }) { selecting = false }
}

/**
 * Taps on the chat's thread, in root coordinates: a block handed to range selection drops it on a tap elsewhere, which
 * the selection container itself does not notice (it clears only when it loses focus)
 */
class ThreadTaps {
  val taps = MutableSharedFlow<Offset>(extraBufferCapacity = 4)
}

val LocalThreadTaps = staticCompositionLocalOf<ThreadTaps?> { null }

/** Reports the thread's taps (a down and an up within the touch slop, nothing consumed) to [taps], without taking them */
fun Modifier.threadTaps(taps: ThreadTaps): Modifier = composed {
  val coordinates = remember { arrayOfNulls<LayoutCoordinates>(1) }
  onGloballyPositioned { coordinates[0] = it }.pointerInput(taps) {
    awaitEachGesture {
      val down = awaitFirstDown(requireUnconsumed = false, pass = PointerEventPass.Initial)
      val up = waitForUpOrCancellation(PointerEventPass.Initial) ?: return@awaitEachGesture
      if ((up.position - down.position).getDistance() > viewConfiguration.touchSlop) return@awaitEachGesture
      coordinates[0]?.takeIf { it.isAttached }?.let { taps.taps.tryEmit(it.localToRoot(down.position)) }
    }
  }
}

/**
 * While [active]: every word of [state]'s container selected, then [onDropped] once the reader drops the selection, by
 * an action of its menu or a tap on the thread outside [bounds] (root coordinates)
 */
@Composable
internal fun SelectAllWhile(active: Boolean, state: SelectionState, bounds: () -> Rect?, onDropped: () -> Unit) {
  if (!active) return
  val taps = LocalThreadTaps.current
  val dropped by rememberUpdatedState(onDropped)
  LaunchedEffect(state) {
    taps?.let { t -> launch { t.taps.collect { at -> if (bounds()?.contains(at) != true) state.clear() } } }
    selectAllUntilDropped(state)
    dropped()
  }
}

/**
 * Selects every word once the text has joined [state]'s container, and returns once the reader drops the selection (a
 * tap away, or an action of its menu). `selectedTexts` is no snapshot state, so it is read every [SELECTION_POLL_MS]
 */
private suspend fun selectAllUntilDropped(state: SelectionState) {
  withFrameNanos {}
  state.selectAll()
  var seen = false
  while (true) {
    kotlinx.coroutines.delay(SELECTION_POLL_MS)
    if (state.selectedTexts.isNotEmpty()) seen = true else if (seen) return
  }
}

/** The block inside range selection: its menu quotes ([onQuote]) or copies just the selected source text */
@Composable
private fun SelectableBlock(block: MdBlock, onQuote: ((String) -> Unit)?, state: SelectionState, content: @Composable () -> Unit) {
  val context = LocalContext.current
  val quote by rememberUpdatedState(onQuote)
  SelectionContainer(
    state,
    Modifier
      // Copy takes the source text, so the system's copy (which keeps the renderer's pads) gives way to this one
      .filterTextContextMenuComponents { it.key != TextContextMenuKeys.CopyKey }
      .appendTextContextMenuComponents {
        if (quote != null) {
          item(QuoteKey, t("quote.add")) {
            quote?.invoke(selectionText(state.selectedTexts).ifEmpty { QuoteText.of(block) })
            state.clear()
            close()
          }
        }
        item(CopyKey, t("common.copy")) {
          copyText(context, selectionText(state.selectedTexts))
          state.clear()
          close()
        }
      },
  ) { content() }
}

private const val SELECTION_POLL_MS = 100L

private object QuoteKey
private object CopyKey

/** The plain text a quoted block carries: what a selection over it would copy (Quotes.swift QuoteText) */
object QuoteText {
  fun of(block: MdBlock): String = when (block) {
    is MdBlock.Paragraph -> Markdown.plain(block.inlines)
    is MdBlock.Heading -> Markdown.plain(block.inlines)
    is MdBlock.ListBlock -> block.items.mapIndexed { n, item ->
      (if (block.ordered) "${block.start + n}. " else "- ") + item.joinToString("\n", transform = ::of)
    }.joinToString("\n")
    is MdBlock.Quote -> block.blocks.joinToString("\n\n", transform = ::of)
    is MdBlock.Code -> block.text
    is MdBlock.Math -> block.tex
    is MdBlock.Table -> (listOf(block.header) + block.rows).joinToString("\n") { row -> row.joinToString(" | ") { Markdown.plain(it) } }
    MdBlock.Rule -> ""
  }
}
