package cc.hotic.acpira.ui.prose

import androidx.compose.foundation.border
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Constraints
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.WebType
import cc.hotic.acpira.logic.MdAlign
import cc.hotic.acpira.logic.MdInline

// Ported from apps/ios Prose/Prose.swift ProseTable (prose.css tables): a header row on --hover, text-2 cells padded
// gap × pad, a hairline under every row but the last, the frame rounded and ringed, at least the column's width and
// scrolling sideways when wider, so a wide table never widens the reading column.

@Composable
fun ProseTable(
  align: List<MdAlign>,
  header: List<List<MdInline>>,
  rows: List<List<List<MdInline>>>,
  inks: InlineInks,
  headInk: InlineInks,
  modifier: Modifier = Modifier,
) {
  val shape = RoundedCornerShape(Metrics.rLg)
  val hover = cc.hotic.acpira.design.ProseInk.hover
  val line = Palette.line
  BoxWithConstraints(
    modifier.fillMaxWidth().clip(shape).border(Metrics.hairline, Palette.conversationLine, shape)
      // The frame's 1 px border takes room around the scrollport
      .padding(Metrics.hairline),
  ) {
    val viewport = constraints.maxWidth
    val columns = header.size
    val all = listOf(header) + rows
    val geometry = remember { TableRows() }
    Layout(
      modifier = Modifier.horizontalScroll(rememberScrollState()).drawBehind {
        // The header's fill and the hairline under every row but the last
        val hs = geometry.heights
        if (hs.isEmpty()) return@drawBehind
        val hair = Metrics.hairline.toPx()
        drawRect(hover, size = Size(size.width, hs[0].toFloat()))
        var y = 0f
        for (r in 0 until hs.size - 1) {
          y += hs[r]
          drawRect(line, topLeft = Offset(0f, y - hair), size = Size(size.width, hair))
        }
      },
      content = {
        all.forEachIndexed { r, row ->
          for (c in 0 until columns) {
            val head = r == 0
            InlineText(
              row.getOrElse(c) { emptyList() }, WebType.text2, if (head) headInk else inks,
              weight = if (head) FontWeight.Medium else FontWeight.Normal, softWrap = false,
              modifier = Modifier.padding(vertical = Metrics.gap, horizontal = Metrics.pad),
            )
          }
        }
      },
    ) { measurables, _ ->
      // Every cell at its own width (fixedSize); a column takes its widest cell, a row its tallest
      val cells = measurables.map { it.measure(Constraints()) }
      val widths = IntArray(columns)
      val heights = IntArray(all.size)
      val hair = Metrics.hairline.roundToPx()
      cells.forEachIndexed { k, p ->
        widths[k % columns] = maxOf(widths[k % columns], p.width)
        heights[k / columns] = maxOf(heights[k / columns], p.height + if (k / columns < all.size - 1) hair else 0)
      }
      // min-width: 100%: spare room goes to the columns in proportion, as a table at width: max-content stretched
      val natural = widths.sum()
      if (natural in 1 until viewport) {
        var left = viewport - natural
        for (c in 0 until columns) {
          val add = if (c == columns - 1) left else ((viewport - natural).toLong() * widths[c] / natural).toInt()
          widths[c] += add
          left -= add
        }
      }
      val width = maxOf(widths.sum(), 0)
      val height = heights.sum()
      geometry.heights = heights
      layout(width, height) {
        var y = 0
        for (r in all.indices) {
          var x = 0
          for (c in 0 until columns) {
            val p = cells[r * columns + c]
            val dx = when (align.getOrElse(c) { MdAlign.None }) {
              MdAlign.Center -> (widths[c] - p.width) / 2
              MdAlign.Right -> widths[c] - p.width
              else -> 0
            }
            p.place(x + dx, y)
            x += widths[c]
          }
          y += heights[r]
        }
      }
    }
  }
}

/** Row heights from the last measure, for the fills drawn behind the cells */
private class TableRows {
  var heights = IntArray(0)
}
