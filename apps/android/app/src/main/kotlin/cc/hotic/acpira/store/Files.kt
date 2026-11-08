package cc.hotic.acpira.store

import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.model.AgentTurn
import cc.hotic.acpira.model.DiffLine
import cc.hotic.acpira.model.HostMsg
import cc.hotic.acpira.model.ProjectBase
import cc.hotic.acpira.model.ProjectChange
import cc.hotic.acpira.model.ProjectError
import cc.hotic.acpira.model.ProjectHunk
import cc.hotic.acpira.model.SessionView
import cc.hotic.acpira.model.ToolCallBlock
import cc.hotic.acpira.model.ToolContent
import cc.hotic.acpira.model.WebviewMsg
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.withTimeoutOrNull

/** A file as the file page shows it: its lines with the session's changes put in place, or why it cannot be shown */
sealed interface FileContent {
  data class Lines(val lines: List<DiffLine>) : FileContent
  data class Failed(val error: ProjectError) : FileContent
}

/**
 * The engine's project reads the file pages ask for and await (apps/ios Store/AppModel+Files.swift, panel/api.ts): a
 * session's changed files and a project file with its changes. Each request carries a seq the answer echoes; an
 * answer that never comes ends as null after a while. The demo answers from the fixture's own diffs
 */
class FileReads(private val model: AppModel) {
  private val asking = HashMap<Int, CompletableDeferred<HostMsg?>>()
  private var seq = 0

  /** Posts the request [make] builds around a fresh seq and waits for the answer that echoes it */
  private suspend fun ask(make: (Int) -> WebviewMsg): HostMsg? {
    if (!model.live) return null
    val n = ++seq
    val answer = CompletableDeferred<HostMsg?>()
    asking[n] = answer
    if (!model.post(make(n))) { asking.remove(n); return null }
    return withTimeoutOrNull(ASK_TIMEOUT_MS) { answer.await() }.also { asking.remove(n) }
  }

  /** The answers this store asked for; false for anything else */
  fun receive(message: HostMsg): Boolean {
    val n = when (message) {
      is HostMsg.ProjectFile -> message.seq
      is HostMsg.SessionChanges -> message.seq
      else -> return false
    }
    asking.remove(n)?.complete(message)
    return true
  }

  /** The files the session's edits left changed, in the order it first edited them; null: no answer */
  suspend fun changes(sessionId: String): List<ProjectChange>? {
    if (!model.live) return demoChanges(model.views[sessionId])
    return (ask { WebviewMsg.SessionChanges(sessionId, it) } as? HostMsg.SessionChanges)?.files
  }

  /** A project file as it is now, with what the session changed in it marked */
  suspend fun file(sessionId: String, path: String): FileContent {
    if (!model.live) {
      val lines = demoLines(model.views[sessionId], path)
      return if (lines.isEmpty()) FileContent.Failed(ProjectError.Missing) else FileContent.Lines(lines)
    }
    val answer = ask { WebviewMsg.ProjectFile(sessionId, path, it, ProjectBase.session) } as? HostMsg.ProjectFile
    return when {
      answer?.text != null -> FileContent.Lines(fileLines(answer.text!!, answer.hunks.orEmpty()))
      answer?.error != null -> FileContent.Failed(answer.error!!)
      else -> FileContent.Failed(ProjectError.Message(t("mobile.engineNeeded")))
    }
  }

  companion object {
    private const val ASK_TIMEOUT_MS = 20_000L

    /** The fixture's edits by file, as the engine counts them */
    fun demoChanges(view: SessionView?): List<ProjectChange> {
      val counts = LinkedHashMap<String, Pair<Int, Int>>()
      for ((path, lines) in diffs(view)) {
        val (a, d) = counts[path] ?: (0 to 0)
        counts[path] = (a + lines.count { it.kind == DiffLine.Kind.add }) to (d + lines.count { it.kind == DiffLine.Kind.del })
      }
      return counts.map { (path, c) -> ProjectChange(path, c.first, c.second) }
    }

    /** The fixture has no project to read: the file page shows its diffs of that file, one after the other, no hunk heads */
    fun demoLines(view: SessionView?, path: String): List<DiffLine> =
      diffs(view).filter { it.first == path }.flatMap { it.second }.filter { it.kind != DiffLine.Kind.hunk }

    private fun diffs(view: SessionView?): List<Pair<String, List<DiffLine>>> =
      view?.turns.orEmpty().filterIsInstance<AgentTurn>().flatMap { turn ->
        turn.blocks.filterIsInstance<ToolCallBlock>().flatMap { call ->
          call.allContents.mapNotNull { content ->
            // A fixture's diff may carry no source; its edit row names the file
            val diff = content as? ToolContent.Diff ?: return@mapNotNull null
            val path = diff.source?.path ?: call.target ?: return@mapNotNull null
            path to diff.lines
          }
        }
      }
  }
}

/**
 * FileViewer.tsx viewRows: the file as it is now, the removed lines put back where they stood, as diff lines (each text
 * starting with its sign, as the wire's do)
 */
fun fileLines(text: String, hunks: List<ProjectHunk>): List<DiffLine> {
  var lines = text.split("\n")
  if (lines.lastOrNull() == "") lines = lines.dropLast(1)
  lines = lines.map { it.removeSuffix("\r") }
  val starts = HashMap<Int, ProjectHunk>()
  for (h in hunks) starts.putIfAbsent(h.at, h)
  val rows = ArrayList<DiffLine>()
  var inside = 0
  var old = 0
  fun putBack(hunk: ProjectHunk) {
    for (d in hunk.del.orEmpty()) {
      old += 1
      rows += DiffLine(DiffLine.Kind.del, "-$d", oldLine = old)
    }
  }
  lines.forEachIndexed { i, line ->
    val n = i + 1
    starts[n]?.let { putBack(it); inside = it.add }
    if (inside > 0) {
      inside -= 1
      rows += DiffLine(DiffLine.Kind.add, "+$line", newLine = n)
    } else {
      old += 1
      rows += DiffLine(DiffLine.Kind.ctx, " $line", oldLine = old, newLine = n)
    }
  }
  // Lines removed at the end stood one past the last line
  starts[lines.size + 1]?.let(::putBack)
  return rows
}

/** Where each run of changed lines starts */
fun changeStarts(lines: List<DiffLine>): List<Int> =
  lines.indices.filter { i -> lines[i].kind != DiffLine.Kind.ctx && (i == 0 || lines[i - 1].kind == DiffLine.Kind.ctx) }

/** Files the page can show rendered as well as as source (FileViewer.tsx isMarkdown) */
fun isMarkdownPath(path: String): Boolean = fileName(path).substringAfterLast('.', "").lowercase() in setOf("md", "markdown", "mdx")

private val frontMatter = Regex("""^---\r?\n([\s\S]*?)\r?\n---[ \t]*(?:\r?\n|$)""")

/**
 * FileViewer.tsx previewText: YAML front matter (a plan's or a skill's header) would render as a rule and a setext
 * heading, so it shows as a YAML block
 */
fun markdownPreviewText(text: String): String {
  val m = frontMatter.find(text) ?: return text
  return "```yaml\n${m.groupValues[1]}\n```\n\n" + text.substring(m.range.last + 1)
}

private val drivePath = Regex("""[A-Za-z]:[\\/].*""")

/** [target] from the folder [dir], `.` and `..` resolved; null for an absolute target or one that climbs out of the root */
fun joinPath(dir: String, target: String): String? {
  if (target.startsWith("/") || target.startsWith("\\") || drivePath.matches(target)) return null
  val parts = dir.split('/', '\\').filter { it.isNotEmpty() }.toMutableList()
  for (part in target.split('/', '\\').filter { it.isNotEmpty() }) {
    when (part) {
      "." -> {}
      ".." -> { if (parts.isEmpty()) return null; parts.removeAt(parts.size - 1) }
      else -> parts += part
    }
  }
  val joined = parts.joinToString("/")
  return if (dir.startsWith("/")) "/$joined" else joined
}

fun fileName(path: String): String = path.split('/', '\\').lastOrNull { it.isNotEmpty() } ?: path

fun folderOf(path: String): String {
  val cut = path.indexOfLast { it == '/' || it == '\\' }
  return if (cut < 0) "" else path.substring(0, cut)
}
