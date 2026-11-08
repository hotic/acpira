package cc.hotic.acpira.store

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import cc.hotic.acpira.model.FileHit

/**
 * The `@` list's file search (apps/ios Store/AppModel+Composer.swift FileSearch). Each query goes out with a new seq and
 * only the reply to the latest one lands, so a slow answer to an earlier query never replaces the newer list; closing
 * the list invalidates whatever is in flight. Without an engine the demo project's files answer at once
 */
@Stable
class FileSearch(private val post: (String, Int) -> Boolean) {
  /** The hits and the query they answer (null until the first reply, so an empty query is not taken as answered) */
  var hits by mutableStateOf<List<FileHit>>(emptyList())
    private set
  var answered by mutableStateOf<String?>(null)
    private set
  private var seq = 0
  private var asked: String? = null

  fun search(query: String) {
    seq++
    asked = query
    if (!post(query, seq)) receive(seq, DemoFiles.search(query))
  }

  /** The engine's `files` reply; one for an older seq is dropped */
  fun receive(seq: Int, files: List<FileHit>) {
    val q = asked
    if (seq != this.seq || q == null) return
    hits = files
    answered = q
  }

  /** The list closed: a late reply cannot refill it */
  fun close() {
    seq++
    asked = null
    if (answered != null || hits.isNotEmpty()) {
      hits = emptyList()
      answered = null
    }
  }
}

/** The demo's stand-in for the engine's walk of the project folder (apps/ios DemoFiles) */
object DemoFiles {
  private const val ROOT = "/Users/dev/projects/acpira"
  private val paths = listOf(
    "README.md", "package.json", "tsconfig.json", "src/args.ts", "src/foo.ts", "src/cli/args.ts", "src/cli/index.ts",
    "docs/dev/ios.md", "docs/dev/webview.md", "packages/ui/chat/Composer.tsx", "packages/shared/protocol.ts",
    "apps/ios/Packages/AcpiraKit/Sources/AcpiraUI/Chat/Composer.swift", "rust/crates/acpira-host/src/runtime.rs",
    "assets/screenshot.png",
  )

  /** Names holding the query first, then paths holding it, at most 20 (the engine's cap); a bare `@` lists them all */
  fun search(query: String): List<FileHit> {
    val q = query.lowercase()
    val named = paths.filter { q.isEmpty() || it.substringAfterLast('/').lowercase().contains(q) }
    val rest = paths.filter { it !in named && it.lowercase().contains(q) }
    return (named + rest).take(20).map { FileHit("file://$ROOT/$it", it) }
  }
}
