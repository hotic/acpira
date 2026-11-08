package cc.hotic.acpira.logic

import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.model.AgentBlock
import cc.hotic.acpira.model.AgentTurn
import cc.hotic.acpira.model.Attachment
import cc.hotic.acpira.model.CompactionBlock
import cc.hotic.acpira.model.PlanBlock
import cc.hotic.acpira.model.PlanStatus
import cc.hotic.acpira.model.SessionView
import cc.hotic.acpira.model.SteerBlock
import cc.hotic.acpira.model.TextBlock
import cc.hotic.acpira.model.ThoughtBlock
import cc.hotic.acpira.model.ToolCallBlock
import cc.hotic.acpira.model.TurnStop
import cc.hotic.acpira.model.UserTurn
import cc.hotic.acpira.model.WireJson
import kotlinx.serialization.json.Json
import java.time.Instant

// apps/ios Logic/SessionExport.swift (shared/exportTranscript.ts on the phone): the session as Markdown for reading, as
// plain text, or its view as JSON for tooling. The Markdown follows the webview's layout (title, agent / project / time,
// then one section per turn) with the export.label.* strings.

object SessionExport {
  fun markdown(view: SessionView, agentName: String, exportedAt: Instant): String {
    val parts = mutableListOf(
      "# ${view.title}",
      "- **${t("export.label.agent")}**: $agentName\n- **${t("export.label.project")}**: ${view.cwd}\n- **${t("export.label.exported")}**: $exportedAt",
      "---",
    )
    for (turn in view.turns) {
      when (turn) {
        is UserTurn -> {
          if (turn.auto == true) { parts += "_${t("export.label.autoCompact")}_"; continue }
          parts += "### ${t("export.label.user")}\n\n${turn.text}"
        }
        is AgentTurn -> {
          val out = mutableListOf("### $agentName")
          for (block in turn.blocks) block(block)?.let { out += ""; out += it }
          val error = turn.error
          val stop = turn.stop
          if (stop == TurnStop.error && error != null) { out += ""; out += "> ${t("export.label.error")}: ${error.message}" }
          else if (stop != null && stop != TurnStop.end_turn) { out += ""; out += "> ${stop.name}" }
          parts += out.joinToString("\n")
        }
      }
    }
    return parts.joinToString("\n\n") + "\n"
  }

  private fun block(b: AgentBlock): String? = when (b) {
    is ThoughtBlock -> "<details><summary>${t("export.label.thinking")}</summary>\n\n${b.text}\n\n</details>"
    is TextBlock -> b.markdown
    is PlanBlock -> b.entries.joinToString("\n") { if (it.status == PlanStatus.completed) "- [x] ${it.title}" else "- [ ] ${it.title}" }
    is ToolCallBlock -> "- **${b.verb}**" + (b.target?.let { " `$it`" } ?: "") + (b.meta?.let { " ($it)" } ?: "")
    is CompactionBlock -> "_${t("export.label.compacted")}_"
    is SteerBlock -> "### ${t("export.label.user")}\n\n${b.text}"
    else -> null
  }

  /**
   * The engine's plain-text export (`export_transcript::export_text`) for the demo: the title, then each run of one
   * speaker under its label, prompts with their quotes as `>` lines, replies as their text blocks
   */
  fun text(view: SessionView, agentName: String): String {
    val said = ArrayList<Pair<Boolean, String>>()
    fun say(user: Boolean, raw: String) {
      val body = raw.trim()
      if (body.isEmpty()) return
      val last = said.lastOrNull()
      if (last != null && last.first == user) said[said.size - 1] = user to last.second + "\n\n" + body else said += user to body
    }
    for (turn in view.turns) {
      when (turn) {
        is UserTurn -> if (turn.auto != true) say(true, promptText(turn.text, turn.attachments))
        is AgentTurn -> for (block in turn.blocks) {
          when (block) {
            is SteerBlock -> say(true, promptText(block.text, block.attachments))
            is TextBlock -> say(false, block.markdown)
            else -> {}
          }
        }
      }
    }
    val label = t("export.label.user")
    val parts = listOf(view.title) + said.map { (user, body) -> "[${if (user) label else agentName}]\n$body" }
    return parts.joinToString("\n\n") + "\n"
  }

  /** A prompt as sent: its quotes as `>` lines (each with its comment) ahead of the typed text */
  private fun promptText(text: String, attachments: List<Attachment>?): String {
    val out = attachments.orEmpty().filterIsInstance<Attachment.Quote>().map { q ->
      // Line by line as Rust's `lines()`: no empty line after a closing newline
      val block = q.text.removeSuffix("\n").split("\n").joinToString("\n") { "> " + it.removeSuffix("\r") }
      if (!q.comment.isNullOrEmpty()) block + "\n\n" + q.comment else block
    }.toMutableList()
    out += text
    return out.joinToString("\n\n")
  }

  private val pretty = Json(WireJson) { prettyPrint = true }

  fun json(view: SessionView): String = pretty.encodeToString(SessionView.serializer(), view)

  /** The title as a file name: path separators and control characters out, at most 60 characters */
  fun fileName(title: String): String {
    val cleaned = title.map { if (it in "/\\:*?\"<>|" || it.isISOControl()) '-' else it }.joinToString("").trim()
    return if (cleaned.isEmpty()) "session" else cleaned.take(FILE_NAME_MAX)
  }

  private const val FILE_NAME_MAX = 60
}
