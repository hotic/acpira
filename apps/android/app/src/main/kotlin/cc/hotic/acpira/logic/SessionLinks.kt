package cc.hotic.acpira.logic

import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.model.SessionSummary
import cc.hotic.acpira.model.ToolCallBlock

// apps/ios Logic/SessionLinks.swift (shared/sessionLinks.ts): `acpira://session/<id>` and the shells' clickable forms
// (`vscode://hotic.acpira/session/<id>` …) name a session; the TypeScript, the Rust and Swift mirrors and this one agree
// on every case of test/fixtures/session-links.json.

object SessionLinks {
  const val PREFIX = "acpira://session/"
  private val schemed = Regex("([A-Za-z][A-Za-z0-9+.\\-]*)://(.*)", RegexOption.DOT_MATCHES_ALL)
  private val range = Regex("(\\d*)-(\\d*)")
  private val reserved = setOf("index", "prefs", "categories", "trash")

  /** A session id names files under the store root: ASCII letters, digits, `_` and `-` only */
  fun isSessionId(id: String): Boolean =
    id.isNotEmpty() && id !in reserved && id.all { it in 'a'..'z' || it in 'A'..'Z' || it in '0'..'9' || it == '_' || it == '-' }

  fun link(id: String): String = PREFIX + id

  /** Surrounding `<…>` or backticks are what models and chat clients wrap links in */
  private fun unwrap(input: String): String {
    val s = input.trim()
    if (s.length >= 2 && ((s.startsWith("<") && s.endsWith(">")) || (s.startsWith("`") && s.endsWith("`")))) {
      return s.substring(1, s.length - 1).trim()
    }
    return s
  }

  /** The session a link points at; query, fragment and one trailing slash are ignored, and bare ids are no links */
  fun idOfLink(href: String): String? {
    var s = unwrap(href)
    val cut = s.indexOfFirst { it == '?' || it == '#' }
    if (cut >= 0) s = s.substring(0, cut)
    if (s.endsWith("/")) s = s.dropLast(1)
    val m = schemed.matchEntire(s) ?: return null
    val scheme = m.groupValues[1].lowercase()
    val rest = m.groupValues[2]
    val slash = rest.indexOf('/')
    val authority = if (slash >= 0) rest.substring(0, slash) else rest
    val path = if (slash >= 0) rest.substring(slash) else ""
    if (scheme == "acpira" && authority.lowercase() == "session") {
      val id = path.drop(1)
      return id.takeIf(::isSessionId)
    }
    // `/session/` sits in the path, and "acpira" anywhere before it (scheme, authority or an earlier path segment)
    val at = path.lastIndexOf("/session/")
    if (at < 0) return null
    val id = path.substring(at + "/session/".length)
    val before = "$scheme://$authority${path.substring(0, at)}".lowercase()
    return id.takeIf { isSessionId(it) && before.contains("acpira") }
  }

  /** A session reference as a tool or the composer takes it: a link, else a bare id */
  fun ref(input: String): String? {
    val s = unwrap(input)
    return idOfLink(s) ?: s.takeIf(::isSessionId)
  }

  /** Session tools carry a canonical link followed by a turn range or search query */
  fun splitTarget(target: String?): Pair<String?, String?> {
    val text = target.orEmpty().trim()
    val space = text.indexOf(' ')
    val head = if (space >= 0) text.substring(0, space) else text
    val id = idOfLink(head)
    val rest = (if (id == null) text else if (space >= 0) text.substring(space + 1) else "").trim()
    return id to rest.ifEmpty { null }
  }

  fun isTool(block: ToolCallBlock): Boolean = block.verbKey == "verb.readSession" || block.verbKey == "verb.searchSessions"

  /** Local ids win over native aliases; an unknown session keeps its id rather than its URI */
  fun title(id: String, summaries: List<SessionSummary>): String {
    val session = summaries.firstOrNull { it.id == id } ?: summaries.firstOrNull { it.acpSessionId == id } ?: return id
    return session.title.ifEmpty { t("session.fallbackTitle") }
  }

  /** folding.ts sessionReadLabel without allocating a long list of turns */
  fun readLabel(turns: String?): String {
    val match = range.matchEntire(turns.orEmpty()) ?: return t("tool.sessionOutline")
    val (a, b) = match.destructured
    if (a.isEmpty() && b.isEmpty()) return t("tool.sessionOutline")
    val from = if (a.isEmpty()) 1 else a.toIntOrNull() ?: return t("tool.sessionOutline")
    if (b.isEmpty()) return t("tool.sessionTurnsFrom", "n" to from)
    val end = b.toIntOrNull() ?: return t("tool.sessionOutline")
    val lo = minOf(from, end)
    val hi = maxOf(from, end)
    val label = if (hi - lo < READ_LIST_MAX) (lo..hi).joinToString(t("turns.nameSep")) else "$lo–$hi"
    return t("tool.sessionTurns", "range" to label)
  }

  /** Up to this many turns read are named one by one; more read as a range */
  private const val READ_LIST_MAX = 5
}
