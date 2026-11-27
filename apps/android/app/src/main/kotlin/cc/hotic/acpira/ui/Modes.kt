package cc.hotic.acpira.ui

import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.model.SessionOption

/** modeIcons.ts `modeIcon`: a mode's glyph from its kind, id and name */
fun modeIcon(mode: SessionOption): String {
  if (mode.kind == "full_access") return "shieldOff"
  if (mode.kind == "auto_review") return "sparkles"
  if (mode.id == "auto_edit" || mode.id == "acceptEdits") return "code"
  val key = "${mode.id} ${mode.name}".lowercase()
  val rules = listOf(
    "bypass|yolo|auto[\\s-]?accept|full[\\s-]?access|unsafe" to "shieldOff",
    "debug" to "bug",
    "plan" to "listTodo",
    "\\bask\\b|chat|question" to "messageCircleQuestionMark",
    "smart|\\bauto\\b" to "sparkles",
    "code|edit|build" to "code",
    "\\bteam\\b|multitask|orchestrat" to "users",
    "agent|default|normal" to "infinity",
  )
  return rules.firstOrNull { Regex(it.first).containsMatchIn(key) }?.second ?: "circleDashed"
}

/**
 * modeIcons.ts `modeLabel`: the short name for a mode the built-in integrations are known to send, matched on the exact
 * id, in the app's language; anything unknown shows the agent's own name
 */
fun modeLabel(mode: SessionOption): String {
  if (mode.kind == "full_access") return t("composer.modeShort.full")
  if (mode.kind == "auto_review") return t("composer.modeShort.auto")
  // Every known agent's `default` asks before acting (modeIcons.ts)
  if (mode.id == "default") return t("composer.access.ask")
  val short = mapOf(
    "agent" to "agent", "build" to "build", "plan" to "plan", "raw" to "raw", "team" to "team",
    "acceptEdits" to "autoEdit", "auto_edit" to "autoEdit", "auto" to "auto", "smart" to "smart", "bypassPermissions" to "full",
    "bypass" to "full", "yolo" to "full", "agent-full-access" to "full", "read-only" to "readOnly",
    "workspace-write" to "workspace", "ask" to "ask", "code" to "code", "accept-edits" to "code",
  )
  return short[mode.id]?.let { t("composer.modeShort.$it") } ?: mode.name
}

/**
 * modeIcons.ts `modeName`: a working mode where it is picked by name (the `/` list, the bar's pill), the bare name without
 * the bean labels' padding to one length; unknown modes keep the label
 */
fun modeName(mode: SessionOption): String {
  val names = mapOf("plan" to "composer.modeName.plan", "team" to "composer.modeName.team", "raw" to "composer.modeName.raw")
  return names[mode.id]?.let { t(it) } ?: modeLabel(mode)
}

/** modeIcons.ts `modeHint`: one line on what the mode does, else the agent's own description */
fun modeHint(mode: SessionOption): String? {
  val hints = mapOf("plan" to "composer.modeHint.plan", "team" to "composer.modeHint.team", "raw" to "composer.modeHint.raw")
  return hints[mode.id]?.let { t(it) } ?: mode.description
}

/**
 * modeIcons.ts `accessIcon`: the approval levels (the built-in agent's, and `auto` from the engine's split of Claude's
 * modes) as one shield family, so the bean reads as a scale; other levels keep their mode glyph
 */
internal fun accessIcon(level: SessionOption): String {
  val icons = mapOf("ask" to "shieldEllipsis", "auto-edit" to "shieldCheck", "auto" to "shieldHalf", "full" to "shieldOff")
  return icons[level.id] ?: modeIcon(level)
}

/**
 * The working mode in force, after the bar's chips (Composer.tsx ModePill): outlined where the chips are plain, so it
 * reads as a token and not a third picker. The whole pill leaves the mode (a phone's × alone is too small to hit), and
 * the bar's height takes the tap
 */
@Composable
internal fun ModePill(mode: SessionOption, modifier: Modifier, onExit: () -> Unit) {
  val name = modeName(mode)
  val exit = t("composer.modeExit", "mode" to name)
  Box(
    modifier.height(Metrics.barCtl).clickable(interactionSource = remember { MutableInteractionSource() }, indication = null, onClick = onExit)
      .semantics { contentDescription = exit; role = Role.Button }.testTag("composer.modePill"),
    contentAlignment = Alignment.Center,
  ) {
    Row(
      Modifier.height(Metrics.rowDense).border(Metrics.hairline, Palette.conversationLine, CircleShape).padding(start = Metrics.s2, end = Metrics.s1_5),
      verticalAlignment = Alignment.CenterVertically,
      horizontalArrangement = Arrangement.spacedBy(Metrics.s1),
    ) {
      Lucide(modeIcon(mode), Palette.fg1, size = Metrics.icon)
      Text(name, color = Palette.fg1, fontSize = Type.text2, maxLines = 1, softWrap = false)
      Lucide("x", Palette.fg3, size = Metrics.chevron)
    }
  }
}

/** Codex's collaboration mode: its `default` is the Build mode, not the permission default beside it */
internal fun workLabel(mode: SessionOption): String = if (mode.id == "default") t("composer.modeShort.build") else modeLabel(mode)

/** The built-in agent's approval levels by id, any other level its short mode label */
internal fun accessLabel(level: SessionOption): String {
  val keys = mapOf("ask" to "composer.access.ask", "auto-edit" to "composer.access.autoEdit", "full" to "composer.access.full")
  return keys[level.id]?.let { t(it) } ?: modeLabel(level)
}
