package cc.hotic.acpira.ui

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.snap
import androidx.compose.animation.core.spring
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.Immutable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.TransformOrigin
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.design.VendorMark
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.logic.MentionPersona
import cc.hotic.acpira.model.FileHit
import cc.hotic.acpira.model.SlashCommand
import cc.hotic.acpira.store.FileSearch

// Ported from apps/ios Chat/Suggestions.swift (Completion.tsx, Slash.tsx and Mention.tsx at phone size): the `/`
// command list and the `@` list (personas, then files) as one card over the composer. The card is part of the
// composer's own column, above the card: a list hung outside it over the transcript would lose its taps to the thread.

/**
 * What the composer offers beyond its pickers: the session's `/` commands, the personas `@` summons and the engine's
 * file search for `@` (none on the new-session page, which has no session to search in)
 */
@Immutable
data class ComposerAssist(
  val commands: List<SlashCommand> = emptyList(),
  val personas: List<MentionPersona> = emptyList(),
  val files: FileSearch? = null,
)

/** The composer's column: the suggestions over the card, both clear of the gesture bar and the keyboard */
@Composable
fun ComposerColumn(modifier: Modifier, suggestions: @Composable ColumnScope.() -> Unit, content: @Composable () -> Unit) {
  Column(
    modifier.fillMaxWidth().navigationBarsPadding().imePadding().padding(horizontal = Metrics.pad, vertical = Metrics.s2),
    verticalArrangement = Arrangement.spacedBy(Metrics.s1_5),
  ) {
    suggestions()
    Box(Modifier.fillMaxWidth()) { content() }
  }
}

/** One card of suggestions: a row per item, scrolling past `gooPanelRows`; [empty] stands in for no rows */
@Composable
fun <T> SuggestionList(
  items: List<T>,
  key: (T) -> String,
  pick: (T) -> Unit,
  empty: String? = null,
  label: @Composable RowScope.(T) -> Unit,
) {
  val shape = RoundedCornerShape(Metrics.rGooPanel)
  val rows = maxOf(1, items.size).toFloat()
  Column(
    Modifier.fillMaxWidth()
      .height(Metrics.gooRow * minOf(rows, Metrics.gooPanelRows) + Metrics.s1_5 * 2)
      .background(Palette.menuBg, shape).border(Metrics.hairline, Palette.menuLine, shape).clip(shape)
      .verticalScroll(rememberScrollState())
      .padding(vertical = Metrics.s1_5)
      .testTag("composer.suggestions"),
  ) {
    if (items.isEmpty() && empty != null) {
      Box(Modifier.fillMaxWidth().heightIn(min = Metrics.gooRow).padding(horizontal = Metrics.gooPadX), contentAlignment = Alignment.CenterStart) {
        Text(empty, color = Palette.fg3, fontSize = Type.text3, maxLines = 1)
      }
    }
    for (item in items) {
      Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(Metrics.s2),
        modifier = Modifier.fillMaxWidth().heightIn(min = Metrics.gooRow).clickable { pick(item) }
          .padding(horizontal = Metrics.gooPadX).testTag("suggest.${key(item)}"),
      ) { label(item) }
    }
  }
}

/** The list's way in and out: grown from its bottom-left corner, as a bean's panel */
@Composable
fun ColumnScope.SuggestionReveal(shown: Boolean, motion: Boolean, content: @Composable () -> Unit) {
  val origin = TransformOrigin(0f, 1f)
  AnimatedVisibility(
    shown,
    enter = if (motion) fadeIn(spring()) + scaleIn(spring(), initialScale = SEED, transformOrigin = origin) else fadeIn(snap()),
    exit = if (motion) fadeOut(spring()) + scaleOut(spring(), targetScale = SEED, transformOrigin = origin) else fadeOut(snap()),
  ) { content() }
}

/** SlashList's row: the glyph, `/name` bright, the input hint faint beside it, the description trailing */
@Composable
fun RowScope.CommandRow(command: SlashCommand) {
  Lucide("squareSlash", Palette.fg3, size = Metrics.icon)
  Text("/${command.name}", color = Palette.fgStrong, fontSize = Type.mono, fontFamily = FontFamily.Monospace, maxLines = 1, overflow = TextOverflow.Ellipsis)
  val hint = command.input?.hint
  if (!hint.isNullOrEmpty()) {
    Text(hint, color = Palette.fg3, fontSize = Type.mono, fontFamily = FontFamily.Monospace, maxLines = 1, overflow = TextOverflow.Ellipsis)
  }
  if (command.description.isNotEmpty()) {
    Text(command.description, color = Palette.fg3, fontSize = Type.text3, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
  }
}

/** MentionList's row: a persona (its CLI's mark, the name, the CLI and model faint after it) or a file (name, folder) */
@Composable
fun RowScope.MentionRow(hit: FileHit, persona: MentionPersona?) {
  if (persona != null) {
    SummonMark(persona)
    Text(persona.name, color = Palette.fgStrong, fontSize = Type.text2, maxLines = 1, overflow = TextOverflow.Ellipsis)
    persona.cli?.let { Text(it, color = Palette.fg2, fontSize = Type.text3, maxLines = 1, overflow = TextOverflow.Ellipsis) }
    persona.model?.let { Text(it, color = Palette.fg3, fontSize = Type.text3, maxLines = 1, overflow = TextOverflow.Ellipsis) }
    persona.effort?.let { Text(it, color = Palette.fg3, fontSize = Type.text3, maxLines = 1, overflow = TextOverflow.Ellipsis) }
    return
  }
  val cut = hit.path.lastIndexOf('/')
  Lucide(if (imageFile(hit.path)) "image" else "fileText", Palette.fg3, size = Metrics.icon)
  Text(
    if (cut >= 0) hit.path.substring(cut + 1) else hit.path, color = Palette.fgStrong, fontSize = Type.mono,
    fontFamily = FontFamily.Monospace, maxLines = 1, overflow = TextOverflow.Ellipsis,
  )
  if (cut >= 0) {
    Text(
      hit.path.substring(0, cut), color = Palette.fg3, fontSize = Type.text3, maxLines = 1, overflow = TextOverflow.StartEllipsis,
      modifier = Modifier.weight(1f, fill = false),
    )
  }
}

/** What leads a summonable entry (Mention.tsx SummonMark): its CLI's mark, or the people glyph for a group */
@Composable
fun SummonMark(persona: MentionPersona, size: androidx.compose.ui.unit.Dp = Metrics.icon) {
  if (persona.group) Lucide("users", Palette.fg2, size = size)
  else VendorMark(persona.agent, persona.cli ?: persona.name, Palette.fg2, size = size)
}

/** attachments.ts imageMimeOf, by extension */
private fun imageFile(path: String): Boolean =
  path.substringAfterLast('.', "").lowercase() in setOf("png", "jpg", "jpeg", "gif", "webp", "heic", "bmp", "svg")

/** `+`'s summon bean (Goo.swift LiquidAttach): the personas in a panel, a pick writes `@name ` at the caret */
@Composable
fun summonBeans(personas: List<MentionPersona>, goo: Goo, onSummon: (MentionPersona) -> Unit): List<Bean> {
  if (personas.isEmpty()) return emptyList()
  val label = t("summon.entry")
  return listOf(
    Bean(
      "summon", label, label,
      glyph = { Lucide("atSign", Palette.fg1, size = Metrics.barIcon) },
      rows = personas.size,
      panel = {
        GooList(-1) {
          for (p in personas) {
            GooRow(p.name, false, glyph = { SummonMark(p, Metrics.iconCtl) }, trailing = {
              p.cli?.let { Text(it, color = Palette.fg3, fontSize = Type.text3, fontWeight = FontWeight.Normal, maxLines = 1) }
            }) {
              goo.close()
              onSummon(p)
            }
          }
        }
      },
    ),
  )
}

/** The list grows from this share of its size */
private const val SEED = 0.85f
