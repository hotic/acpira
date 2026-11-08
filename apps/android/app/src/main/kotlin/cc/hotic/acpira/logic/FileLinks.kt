package cc.hotic.acpira.logic

import android.content.Context
import android.widget.Toast
import java.net.URI
import java.net.URLDecoder

// Ported from apps/ios Logic/FileLinks.swift (chat/fileLinks.ts parseFileLink) and Logic/SessionLinks.swift
// (shared/sessionLinks.ts): the workspace paths agent Markdown names (a link's target, a code span), with an optional
// line (`src/a.ts:12`, `#L12`, `file:///…`), and the links that name another session. Every link tapped in prose goes
// through [openProseLink], which routes files to [openFileLink] and sessions to [openSessionLink].

data class FileLink(val path: String, val line: Int? = null)

object FileLinks {
  private val webTLD = setOf("com", "org", "net", "io", "edu", "gov", "cn", "co", "dev", "info", "xyz", "me", "ai")

  /** Compiled once: the prose asks about every link and code span each time a streamed paragraph is built */
  private object Patterns {
    val scheme = Regex("""^(?:https?|mailto|javascript|data|vbscript|blob|tel):""", RegexOption.IGNORE_CASE)
    val anchorLine = Regex("""(.*)#[Ll](\d+)(?:-\d+)?""")
    val colonLine = Regex("""(.+?):(\d+)(?:[–-]\d+)?(?::\d+)?""")
    val unc = Regex("""^\\\\[^\\]+\\[^\\]+""")
    val rooted = Regex("""^(?:\.{0,2}/|[A-Za-z]:[\\/]|/)""")
    val slashedExtension = Regex("""\.[A-Za-z][\w-]*$""")
    val bareFile = Regex("""[^\s/\\]+\.([A-Za-z][\w-]{1,15})""")
    val driveInPath = Regex("""/[A-Za-z]:.*""")
    val fragmentLine = Regex("""^[Ll](\d+)""")
  }

  fun parse(text: String): FileLink? {
    val raw = text.trim()
    if (raw.isEmpty() || raw.any(Char::isWhitespace)) return null
    if (raw.startsWith("#") || Patterns.scheme.containsMatchIn(raw)) return null
    if (raw.lowercase().startsWith("file:")) return parseFileURL(raw)
    return pathAndLine(raw)
  }

  private fun parseFileURL(href: String): FileLink? {
    val url = runCatching { URI(href) }.getOrNull() ?: return null
    if (url.scheme?.lowercase() != "file") return null
    val encoded = url.rawPath ?: return null
    // Line suffixes are read before percent decoding: a name ending in %23L12 is no line anchor
    val parsed = pathAndLine(encoded)
    var path = runCatching { URLDecoder.decode((parsed?.path ?: encoded).replace("+", "%2B"), "UTF-8") }.getOrDefault(encoded)
    val host = url.host
    if (!host.isNullOrEmpty() && host != "localhost") path = "//$host$path"
    else if (Patterns.driveInPath.matches(path)) path = path.drop(1)
    if (path.isEmpty()) return null
    val line = url.rawFragment?.let { Patterns.fragmentLine.find(it)?.groupValues?.get(1)?.toIntOrNull() } ?: parsed?.line
    return FileLink(path, line)
  }

  private fun pathAndLine(text: String): FileLink? {
    Patterns.anchorLine.matchEntire(text)?.let { m ->
      val path = m.groupValues[1]
      return if (isFilePath(path)) FileLink(path, m.groupValues[2].toIntOrNull()) else null
    }
    Patterns.colonLine.matchEntire(text)?.let { m ->
      if (isFilePath(m.groupValues[1])) return FileLink(m.groupValues[1], m.groupValues[2].toIntOrNull())
    }
    return if (isFilePath(text)) FileLink(text) else null
  }

  fun isFilePath(path: String): Boolean {
    if (path.isEmpty()) return false
    if (Patterns.unc.containsMatchIn(path)) return true
    if (Patterns.rooted.containsMatchIn(path)) return true
    val slashed = path.contains('/') || path.contains('\\')
    if (path.startsWith(".") && slashed) return true
    if (slashed && Patterns.slashedExtension.containsMatchIn(path)) return true
    val ext = Patterns.bareFile.matchEntire(path) ?: return false
    return ext.groupValues[1].lowercase() !in webTLD
  }
}

object SessionLinks {
  private val schemed = Regex("""([A-Za-z][A-Za-z0-9+.\-]*)://(.*)""")
  private val reserved = setOf("index", "prefs", "categories", "trash")

  const val PREFIX = "acpira://session/"

  /** A session id names files under the store root: ASCII letters, digits, `_` and `-` only */
  fun isSessionId(id: String) =
    id.isNotEmpty() && id !in reserved && id.all { it in 'a'..'z' || it in 'A'..'Z' || it in '0'..'9' || it == '_' || it == '-' }

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
      return if (isSessionId(id)) id else null
    }
    // `/session/` sits in the path, and "acpira" anywhere before it (scheme, authority or an earlier path segment)
    val at = path.lastIndexOf("/session/")
    if (at < 0) return null
    val id = path.substring(at + "/session/".length)
    val before = "$scheme://$authority${path.substring(0, at)}".lowercase()
    return if (isSessionId(id) && before.contains("acpira")) id else null
  }
}

/** The file a prose link or code span opens, unless it names a session (InlineText.swift fileTarget) */
fun fileTarget(text: String): FileLink? = if (SessionLinks.idOfLink(text) != null) null else FileLinks.parse(text)

/**
 * Opens a workspace file a reply named. The file screens are not built yet, so the path (and line) shows as a notice;
 * this is the one place they take over
 */
fun openFileLink(context: Context, link: FileLink) {
  Toast.makeText(context, link.line?.let { "${link.path}:$it" } ?: link.path, Toast.LENGTH_SHORT).show()
}

/**
 * Opens another session a reply links to, and answers whether it did. The session hook takes over here; until then
 * no link is handled, and the caller opens it as an ordinary URL
 */
@Suppress("UNUSED_PARAMETER")
fun openSessionLink(context: Context, id: String): Boolean = false

/** Every link tapped in prose: a session, a workspace file, else the URL itself (chat/Link.tsx) */
fun openProseLink(context: Context, url: String, openUrl: (String) -> Unit) {
  SessionLinks.idOfLink(url)?.let { if (openSessionLink(context, it)) return }
  val file = fileTarget(url)
  if (file != null) openFileLink(context, file) else openUrl(url)
}
