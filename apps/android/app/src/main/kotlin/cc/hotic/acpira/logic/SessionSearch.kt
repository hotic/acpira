package cc.hotic.acpira.logic

import cc.hotic.acpira.model.AgentTurn
import cc.hotic.acpira.model.SessionSummary
import cc.hotic.acpira.model.SessionView
import cc.hotic.acpira.model.TextBlock
import cc.hotic.acpira.model.UserTurn

// The list's history search (apps/ios Logic/SessionSearch.swift, packages/ui chat/sessionSearch.ts): a session matches
// when it carries every term, in its title, agent or project here, or in its conversation, which the engine searches
// (`searchSessions`, session_search.rs) and answers with the text around the first match. The demo searches its own
// views the same way.

/** Content hits for one query: session id → the text around its first match, and summaries the list may not hold */
data class SessionHits(val query: String, val snippets: Map<String, String>, val summaries: List<SessionSummary> = emptyList())

/** The distinct lower-cased terms of a query */
fun searchTerms(query: String): List<String> =
  query.lowercase().split(Regex("\\s+")).filter { it.isNotEmpty() }.distinct()

object SessionSearch {
  /** Every term occurs in one of the row's own words: the title, the agent's name or the project's */
  fun matches(fields: List<String>, terms: List<String>): Boolean {
    val hay = fields.joinToString("\n").lowercase()
    return terms.all { hay.contains(it) }
  }

  /**
   * [text] split around every occurrence of the terms, longest first and case-insensitively; matches sit at the odd
   * indices (markParts)
   */
  fun parts(text: String, terms: List<String>): List<String> {
    val sorted = terms.filter { it.isNotEmpty() }.sortedByDescending { it.length }
    if (sorted.isEmpty()) return listOf(text)
    val out = ArrayList<String>()
    val plain = StringBuilder()
    var i = 0
    outer@ while (i < text.length) {
      for (term in sorted) {
        if (text.regionMatches(i, term, 0, term.length, ignoreCase = true)) {
          out += plain.toString()
          out += text.substring(i, i + term.length)
          plain.clear()
          i += term.length
          continue@outer
        }
      }
      plain.append(text[i])
      i++
    }
    out += plain.toString()
    return out
  }

  /**
   * The demo's conversation search: every term somewhere in the session's prompts and replies, the snippet from the
   * first message holding any of them; null when the title alone matches or a term is missing
   */
  fun snippet(view: SessionView, terms: List<String>): String? {
    val messages = ArrayList<String>()
    for (turn in view.turns) {
      when (turn) {
        is UserTurn -> messages += turn.text
        is AgentTurn -> turn.blocks.filterIsInstance<TextBlock>().forEach { messages += it.markdown }
      }
    }
    val all = messages.joinToString("\n").lowercase()
    if (!terms.all { all.contains(it) } || matches(listOf(view.title), terms)) return null
    for (message in messages) {
      val flat = message.split('\n', '\r').filter { it.isNotEmpty() }.joinToString(" ")
      val first = terms.mapNotNull { term -> flat.indexOf(term, ignoreCase = true).takeIf { it >= 0 }?.let { it to term.length } }
        .minByOrNull { it.first } ?: continue
      return excerpt(flat, first.first, first.first + first.second)
    }
    return null
  }

  /** A short lead before the match and the rest of the line after it, as the engine cuts it (the row truncates the tail) */
  private fun excerpt(text: String, from: Int, to: Int): String {
    val start = maxOf(0, from - EXCERPT_LEAD)
    val end = minOf(text.length, start + EXCERPT_LENGTH)
    return (if (start > 0) "…" else "") + text.substring(start, maxOf(end, to)) + (if (end < text.length) "…" else "")
  }

  private const val EXCERPT_LEAD = 8
  private const val EXCERPT_LENGTH = 160
}
