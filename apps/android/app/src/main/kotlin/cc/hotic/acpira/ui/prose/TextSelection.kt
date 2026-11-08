package cc.hotic.acpira.ui.prose

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

// Ported from apps/ios Prose/TextSelection.swift and the quote half of Chat/Quotes.swift (QuoteToolbar.tsx on a touch
// screen). One Markdown block owns its selection, as one block owns its menu on iOS: a long press selects within the
// block, and the selection's menu quotes just the selected text into the composer ("Add to chat") besides Copy. Copy
// and Quote take the source text: the renderer's pads and break hints never reach the clipboard.

/** The selected source text, excluding the runs the renderer added (pads, break hints), not the source's own */
internal fun selectionText(texts: List<AnnotatedString>): String = texts.joinToString("\n") { text ->
  val marks = BooleanArray(text.length)
  for (d in text.getStringAnnotations(DECORATION_TAG, 0, text.length)) for (k in d.start until d.end) marks[k] = true
  buildString { text.forEachIndexed { k, c -> if (!marks[k] && c != '​' && c != '⁠') append(c) } }
}

/**
 * A block's selection surface. `onQuote` is what selected reply text quotes into (the chat's composer); without one the
 * menu offers only the system's own actions. Diagrams, formulas and image-only blocks have no text to select
 */
@Composable
fun BlockSelection(block: MdBlock, onQuote: ((String) -> Unit)?, content: @Composable () -> Unit) {
  if (block is MdBlock.Math || block is MdBlock.Rule || (block is MdBlock.Code && block.lang == "mermaid")) {
    content()
    return
  }
  val state = rememberSelectionState()
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
