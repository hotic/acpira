package cc.hotic.acpira.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTagsAsResourceId
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.window.Popup
import androidx.compose.ui.window.PopupProperties
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.model.Draft

// Ported from apps/ios Chat/Quotes.swift (Quotes.tsx): text selected in a reply and quoted with "Add to chat" waits in
// the composer's strip as one "N annotations" chip; its card lists each quote with an optional comment and its removal.
// Quotes travel as `quote` drafts, which the engine writes into the prompt.

/** What selected reply text quotes into: the chat's composer. Set along the chat's thread; none elsewhere */
val LocalQuote = compositionLocalOf<((String) -> Unit)?> { null }

/** A quote as the strip stages it */
fun quoteStaged(text: String): Staged = Staged(Draft.Quote(text), "")

/**
 * The staged quotes as one chip, leading the strip (Quotes.tsx QuoteChip): a tap opens their card, its mark drops them
 * all. Nothing while no quote is staged
 */
@Composable
fun QuoteChip(staged: List<Staged>, onStaged: (List<Staged>) -> Unit) {
  val count = staged.count { it.draft is Draft.Quote }
  if (count == 0) return
  var open by remember { mutableStateOf(false) }
  val shape = RoundedCornerShape(Metrics.rLg)
  Box {
    Row(
      verticalAlignment = Alignment.CenterVertically,
      horizontalArrangement = Arrangement.spacedBy(Metrics.s1_5),
      modifier = Modifier.height(Metrics.stagedTile).background(Palette.chip, shape).clip(shape)
        .clickable { open = true }.padding(start = Metrics.s3, end = Metrics.s3 + Metrics.stagedRemove).testTag("composer.quotes"),
    ) {
      Lucide("messageSquareText", Palette.fg3, size = Metrics.icon)
      Text(
        if (count == 1) t("quote.chipOne") else t("quote.chip", "n" to count),
        color = Palette.fg2, fontSize = Type.text3, fontWeight = FontWeight.Medium, maxLines = 1,
      )
    }
    Box(
      Modifier.align(Alignment.TopEnd).padding(Metrics.s1).size(Metrics.stagedRemove)
        .background(Palette.scrim, CircleShape).clip(CircleShape)
        .clickable { onStaged(staged.filter { it.draft !is Draft.Quote }) },
      contentAlignment = Alignment.Center,
    ) { Lucide("x", Palette.fgStrong, size = Metrics.chevron, contentDescription = t("quote.removeAll")) }
    if (open) {
      Popup(
        alignment = Alignment.BottomStart,
        offset = with(LocalDensity.current) { IntOffset(0, -(Metrics.stagedTile + Metrics.s2).roundToPx()) },
        onDismissRequest = { open = false },
        properties = PopupProperties(focusable = true),
      ) { QuoteCard(staged, onStaged) { open = false } }
    }
  }
}

/** The quotes' card: numbered entries, each its text (five lines at most), a comment field and its removal */
@OptIn(androidx.compose.ui.ExperimentalComposeUiApi::class)
@Composable
private fun QuoteCard(staged: List<Staged>, onStaged: (List<Staged>) -> Unit, onEmpty: () -> Unit) {
  val places = staged.indices.filter { staged[it].draft is Draft.Quote }
  LaunchedEffect(places.isEmpty()) { if (places.isEmpty()) onEmpty() }
  val shape = RoundedCornerShape(Metrics.rGooPanel)
  Column(
    Modifier.width(Metrics.popLg).heightIn(max = Metrics.popLg)
      .background(Palette.menuBg, shape).border(Metrics.hairline, Palette.menuLine, shape).clip(shape)
      .verticalScroll(rememberScrollState()).padding(Metrics.s3)
      // A popup is a window of its own: its test tags need the resource-id switch again
      .semantics { testTagsAsResourceId = true }
      .testTag("quotes.card"),
    verticalArrangement = Arrangement.spacedBy(Metrics.s3),
  ) {
    places.forEachIndexed { n, at ->
      val quote = staged[at].draft as Draft.Quote
      Row(horizontalArrangement = Arrangement.spacedBy(Metrics.gapHalf)) {
        Text("${n + 1}.", color = Palette.fg3, fontSize = Type.text3, lineHeight = Type.text3Line, style = TextStyle(fontFeatureSettings = "tnum"))
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(Metrics.s0_5)) {
          Text(t("quote.selected"), color = Palette.fg3, fontSize = Type.text3, lineHeight = Type.text3Line)
          Text(quote.text, color = Palette.fg1, fontSize = Type.text2, lineHeight = Type.text2Line, maxLines = QUOTE_LINES, overflow = TextOverflow.Ellipsis)
          // The comment as the field edits it; cleared, the quote goes out without one
          Box(Modifier.padding(top = Metrics.s1)) {
            val comment = quote.comment.orEmpty()
            if (comment.isEmpty()) Text(t("quote.commentPlaceholder"), color = Palette.fg3, fontSize = Type.text2, lineHeight = Type.text2Line)
            BasicTextField(
              value = comment,
              onValueChange = { v ->
                onStaged(staged.mapIndexed { i, s -> if (i == at) Staged(Draft.Quote(quote.text, v.ifEmpty { null }), s.name) else s })
              },
              textStyle = TextStyle(color = Palette.fgStrong, fontSize = Type.text2, lineHeight = Type.text2Line),
              cursorBrush = SolidColor(Palette.fgStrong),
              maxLines = COMMENT_LINES,
              modifier = Modifier.fillMaxWidth().testTag("quotes.comment.$n"),
            )
          }
        }
        Box(
          Modifier.size(Metrics.ctlSm).clip(CircleShape).clickable { onStaged(staged.filterIndexed { i, _ -> i != at }) },
          contentAlignment = Alignment.Center,
        ) { Lucide("trash2", Palette.fg2, size = Metrics.icon, contentDescription = t("quote.remove")) }
      }
    }
  }
}

private const val QUOTE_LINES = 5
private const val COMMENT_LINES = 4
