package cc.hotic.acpira.ui

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTagsAsResourceId
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.DpOffset
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type

/**
 * A menu as the iOS app's system menus read (UIMenu): one large-radius card a step lighter than the page, rows the
 * height of a menu row, the current choice ticked on the leading side, glyphs on the trailing side. Material's menu
 * keeps the placement and the dismissal; the look is the iOS one
 */
@Composable
fun AppMenu(
  expanded: Boolean,
  onDismiss: () -> Unit,
  modifier: Modifier = Modifier,
  offset: DpOffset = DpOffset(Metrics.s0, Metrics.s1),
  content: @Composable ColumnScope.() -> Unit,
) {
  DropdownMenu(
    expanded = expanded,
    onDismissRequest = onDismiss,
    // A menu is a window of its own: its rows' tags need the resource-id switch again (ui.py finds them by it)
    modifier = modifier.widthIn(min = Metrics.menuMinW).semantics { testTagsAsResourceId = true },
    offset = offset,
    shape = RoundedCornerShape(Metrics.rMenu),
    containerColor = Palette.menuBg,
    tonalElevation = Metrics.s0,
    shadowElevation = Metrics.menuShadow,
    border = BorderStroke(Metrics.hairline, Palette.menuLine),
    content = content,
  )
}

/**
 * One row of an [AppMenu]. checked: null for an action (no tick column), else whether this choice is the current one;
 * a menu of choices passes it on every row so the names line up. icon: the trailing glyph. danger: a destructive action
 */
@Composable
fun AppMenuItem(
  text: String,
  onClick: () -> Unit,
  modifier: Modifier = Modifier,
  subtitle: String? = null,
  checked: Boolean? = null,
  danger: Boolean = false,
  icon: (@Composable () -> Unit)? = null,
) {
  val ink = if (danger) Palette.danger else Palette.fgStrong
  Row(
    verticalAlignment = Alignment.CenterVertically,
    horizontalArrangement = Arrangement.spacedBy(Metrics.s2),
    modifier = modifier
      .fillMaxWidth()
      .heightIn(min = Metrics.menuRow)
      .clickable(onClick = onClick)
      .padding(horizontal = Metrics.pad, vertical = Metrics.s1_5),
  ) {
    if (checked != null) {
      Box(Modifier.size(Metrics.icon), contentAlignment = Alignment.Center) {
        if (checked) Lucide("check", ink, size = Metrics.icon)
      }
    }
    Column(Modifier.weight(1f)) {
      Text(text, color = ink, fontSize = Type.text1, lineHeight = Type.text2Line, maxLines = 1, overflow = TextOverflow.Ellipsis)
      if (subtitle != null) {
        Text(subtitle, color = Palette.fg3, fontSize = Type.text3, lineHeight = Type.text3Line, maxLines = 1, overflow = TextOverflow.Ellipsis)
      }
    }
    if (icon != null) Box(Modifier.size(Metrics.iconCtl), contentAlignment = Alignment.Center) { icon() }
  }
}

/** A submenu's head (UIMenu's): its title, and the way back to the level above */
@Composable
fun AppMenuBack(title: String, onClick: () -> Unit, modifier: Modifier = Modifier) {
  Row(
    verticalAlignment = Alignment.CenterVertically,
    horizontalArrangement = Arrangement.spacedBy(Metrics.s2),
    modifier = modifier.fillMaxWidth().heightIn(min = Metrics.menuRow).clickable(onClick = onClick).padding(horizontal = Metrics.pad),
  ) {
    Box(Modifier.size(Metrics.icon), contentAlignment = Alignment.Center) { Lucide("chevronLeft", Palette.fgStrong, size = Metrics.icon) }
    Text(title, color = Palette.fgStrong, fontSize = Type.text1, fontWeight = FontWeight.SemiBold, maxLines = 1, overflow = TextOverflow.Ellipsis)
  }
}

/** A titled section's caption (UIMenu's section title), over the rows it names */
@Composable
fun AppMenuCaption(title: String) {
  Text(
    title, color = Palette.fg3, fontSize = Type.text4, lineHeight = Type.text4Line, maxLines = 1, overflow = TextOverflow.Ellipsis,
    modifier = Modifier.fillMaxWidth().padding(horizontal = Metrics.pad).padding(top = Metrics.s2, bottom = Metrics.s1),
  )
}

/** The band between an [AppMenu]'s sections */
@Composable
fun AppMenuGap() {
  Box(Modifier.fillMaxWidth().padding(vertical = Metrics.s1).heightIn(min = Metrics.s2).background(Palette.menuGap))
}

/** A trailing glyph in the menu's ink (red for a destructive row) */
@Composable
fun MenuGlyph(name: String, danger: Boolean = false) {
  Lucide(name, if (danger) Palette.danger else Palette.fgStrong, size = Metrics.icon)
}
