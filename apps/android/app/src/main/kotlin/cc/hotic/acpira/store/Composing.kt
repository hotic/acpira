package cc.hotic.acpira.store

import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.logic.Markdown
import cc.hotic.acpira.logic.MentionPersona
import cc.hotic.acpira.model.AgentTurn
import cc.hotic.acpira.model.SlashCommand
import cc.hotic.acpira.model.TextBlock
import cc.hotic.acpira.ui.prose.QuoteText
import cc.hotic.acpira.ui.quoteStaged

// What the composer reads off the app model beyond the draft (apps/ios Store/AppModel+Composer.swift): the summonable
// personas, a new session's `/` commands, and quotes from the transcript.

/** The personas the composer offers for `@` summons (settings → Subagents) */
val AppModel.personas: List<MentionPersona>
  get() = MentionPersona.from(settings, agents, t("common.listSep"))

/**
 * The `/` commands a new session of [agent] in [cwd] offers before its CLI runs (AppModel.commands): the engine's
 * controls answer (its newest session's), else a kept view of the agent's
 */
fun AppModel.commands(agent: String, cwd: String?): List<SlashCommand> {
  hostControlsKey(agent, cwd)?.let { return agentCommands[it].orEmpty() }
  agentCommands[agent]?.takeIf { it.isNotEmpty() }?.let { return it }
  val latest = summaries.filter { it.agent == agent }.sortedByDescending { it.updatedAt }
  return latest.firstNotNullOfOrNull { s -> views[s.id]?.commands?.takeIf { it.isNotEmpty() } }
    ?: views.values.filter { it.agent == agent && it.commands.isNotEmpty() }.minByOrNull { it.id }?.commands.orEmpty()
}

/** Text quoted from the transcript waits in the chat's composer as an annotation (QuoteToolbar's "Add to chat") */
fun AppModel.quote(id: String, text: String) {
  val trimmed = text.trim()
  if (trimmed.isEmpty()) return
  val draft = chat.draft(id)
  draft.staged = draft.staged + quoteStaged(trimmed)
}

/** The `quotes` popup: the last reply's first two blocks staged as quotes (screenshots) */
fun AppModel.seedQuotes(id: String) {
  val view = views[id] ?: return
  if (chat.draft(id).staged.isNotEmpty()) return
  val reply = view.turns.asReversed().asSequence().filterIsInstance<AgentTurn>()
    .mapNotNull { turn -> turn.blocks.filterIsInstance<TextBlock>().firstOrNull()?.markdown }.firstOrNull()
  for (block in Markdown.parse(reply.orEmpty()).take(2)) quote(id, QuoteText.of(block))
}
