package cc.hotic.acpira.ui

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.RowScope
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type

/**
 * The bar of a page pushed from a chat (the subagent pages, the terminal, the review pages): back, the title with an
 * optional faint subtitle under it (the system bar's subtitle on iOS 26), and the page's own buttons at the end
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ToolPageBar(
  title: String,
  tag: String,
  onBack: () -> Unit,
  subtitle: String? = null,
  ground: Color = Palette.bg0,
  actions: @Composable RowScope.() -> Unit = {},
) {
  TopAppBar(
    title = {
      Column {
        Text(
          title, fontSize = Type.barTitle, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis,
          modifier = Modifier.testTag("$tag.title"),
        )
        subtitle?.let { Text(it, fontSize = Type.text3, color = Palette.fg3, maxLines = 1, overflow = TextOverflow.Ellipsis) }
      }
    },
    navigationIcon = { LeadButton("$tag.back", onBack) },
    actions = actions,
    colors = TopAppBarDefaults.topAppBarColors(containerColor = ground, scrolledContainerColor = ground),
  )
}
