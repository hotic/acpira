@file:Suppress("EnumEntryName")

package cc.hotic.acpira.model

import kotlinx.serialization.Serializable

// SSH remote workspaces (packages/shared/remote.ts, acpira_shared::remote, apps/ios Remote.swift). A folder on an SSH host
// is written `ssh://<alias>/<absolute path>` in every field that holds a folder (a session's cwd, the project list); the
// alias is the host's name in ~/.ssh/config (or a typed `user@host:port`). Every side runs test/fixtures/remote-folders.json.

/** A folder on an SSH host: the host's alias and the absolute path there */
data class RemoteFolder(val alias: String, val path: String) {
  companion object {
    const val SCHEME = "ssh://"

    /** `ssh://<alias>/<path>` split and normalized; null for anything else, a local path included */
    fun parse(folder: String): RemoteFolder? {
      if (!folder.startsWith(SCHEME)) return null
      val rest = folder.substring(SCHEME.length)
      val slash = rest.indexOf('/')
      if (slash < 0) return null
      val alias = rest.substring(0, slash)
      if (!isAlias(alias)) return null
      val path = normalize(rest.substring(slash)) ?: return null
      return RemoteFolder(alias, path)
    }

    private val ENDS_AUTHORITY = "/?#\\".map { it.code }.toSet()

    /** Rust's `char::is_control`: C0, DEL and C1 */
    private fun isControl(cp: Int) = cp < 0x20 || cp in 0x7f..0x9f

    /** Rust's `char::is_whitespace` (Unicode White_Space) */
    private fun isWhitespace(cp: Int) = Character.isWhitespace(cp) || Character.isSpaceChar(cp) || cp == 0x85

    /** A host alias as ssh takes it: no whitespace or control characters, nothing that ends the authority, no leading `-` */
    internal fun isAlias(alias: String): Boolean =
      alias.isNotEmpty() && !alias.startsWith("-") &&
        alias.codePoints().noneMatch { isControl(it) || isWhitespace(it) || it in ENDS_AUTHORITY }

    /**
     * An absolute POSIX path normalized: empty and `.` segments dropped, no trailing slash; `..` and control characters
     * refused
     */
    internal fun normalize(path: String): String? {
      if (!path.startsWith("/") || path.codePoints().anyMatch(::isControl)) return null
      val parts = mutableListOf<String>()
      for (segment in path.split('/')) {
        if (segment.isEmpty() || segment == ".") continue
        if (segment == "..") return null
        parts += segment
      }
      return "/" + parts.joinToString("/")
    }
  }
}

@Serializable(with = ConnectionState.S::class)
enum class ConnectionState { off, connecting, connected, error, unknown;
  object S : TolerantEnumSerializer<ConnectionState>("ConnectionState", entries, unknown)
}

/** One SSH host as the pages see it (HostMsg `connections`), the part the list and the new session read */
@Serializable
data class ConnectionView(
  val alias: String,
  val disabled: Boolean? = null,
  val state: ConnectionState = ConnectionState.unknown,
  val error: Failure? = null,
) {
  /** Why the last attempt failed; the message is the engine's, already worded for people */
  @Serializable
  data class Failure(val message: String = "")
}
