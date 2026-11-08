package cc.hotic.acpira.logic

import java.net.URI
import java.net.URLDecoder
import java.net.URLEncoder

// Ported from apps/ios Logic/FileLinks.swift (chat/fileLinks.ts parseFileLink) and Logic/SessionLinks.swift
// (shared/sessionLinks.ts): the workspace paths agent Markdown names (a link's target, a code span), with an optional
// line (`src/a.ts:12`, `#L12`, `file:///…`), and the links that name another session. Every link tapped in prose goes
// through [openProseLink]: a file becomes the app's own `acpira-file:` link ([FileLinks.url]) and goes, like a session
// link, to the screen's link handler (ui/Links.kt), which opens the file page of the chat it was tapped in.

data class FileLink(val path: String, val line: Int? = null)

object FileLinks {
  const val SCHEME = "acpira-file"
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
    decode(raw)?.let { return it }
    if (raw.startsWith("#") || Patterns.scheme.containsMatchIn(raw)) return null
    if (raw.lowercase().startsWith("file:")) return parseFileURL(raw)
    return pathAndLine(raw)
  }

  /** The app's own link for a file, as the prose sets it on a path (`acpira-file://open?path=…&line=…`) */
  fun url(link: FileLink): String =
    "$SCHEME://open?path=" + URLEncoder.encode(link.path, "UTF-8").replace("+", "%20") + (link.line?.let { "&line=$it" } ?: "")

  /** The file an [url] link names; null for any other link */
  fun decode(url: String): FileLink? {
    if (!url.startsWith("$SCHEME:")) return null
    val query = url.substringAfter('?', "")
    val items = query.split('&').mapNotNull { part ->
      val eq = part.indexOf('=')
      if (eq <= 0) null else part.substring(0, eq) to runCatching { URLDecoder.decode(part.substring(eq + 1), "UTF-8") }.getOrNull()
    }.toMap()
    val path = items["path"]?.takeIf { it.isNotEmpty() } ?: return null
    return FileLink(path, items["line"]?.toIntOrNull())
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

/** The file a prose link or code span opens, unless it names a session (InlineText.swift fileTarget) */
fun fileTarget(text: String): FileLink? = if (SessionLinks.idOfLink(text) != null) null else FileLinks.parse(text)

/**
 * Opens a workspace file a reply named: as the app's own link through [openUrl], the screen's link handler
 * (ui/Links.kt), which pushes the file page of the chat on screen (store/Links.kt)
 */
fun openFileLink(link: FileLink, openUrl: (String) -> Unit) = openUrl(FileLinks.url(link))

/**
 * Every link tapped in prose: a workspace file, else the URL itself (chat/Link.tsx); [openUrl] is the screen's link
 * handler (ui/Links.kt), which opens a session or file link in the app
 */
fun openProseLink(url: String, openUrl: (String) -> Unit) {
  val file = fileTarget(url)
  if (file != null) openFileLink(file, openUrl) else openUrl(url)
}
