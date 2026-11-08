package cc.hotic.acpira.store

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.os.Build
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.core.content.FileProvider
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.logic.SessionExport
import cc.hotic.acpira.logic.SessionHits
import cc.hotic.acpira.logic.SessionLinks
import cc.hotic.acpira.logic.SessionSearch
import cc.hotic.acpira.logic.searchTerms
import cc.hotic.acpira.model.AgentTurn
import cc.hotic.acpira.model.CategoryOp
import cc.hotic.acpira.model.ConnectionView
import cc.hotic.acpira.model.ExportFormat
import cc.hotic.acpira.model.HostMsg
import cc.hotic.acpira.model.InitState
import cc.hotic.acpira.model.SessionCategories
import cc.hotic.acpira.model.SessionSection
import cc.hotic.acpira.model.SessionState
import cc.hotic.acpira.model.SessionStatus
import cc.hotic.acpira.model.SessionSummary
import cc.hotic.acpira.model.SessionView
import cc.hotic.acpira.model.WebviewMsg
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import java.io.File

/** A toast's button (apps/ios ToastAction): Undo after a delete, an archive or a project taken off the list */
class ToastAction(val label: String, val run: () -> Unit)

/** How the list groups its sessions (apps/ios `SessionListView`, the desktop view menu's Grouping) */
enum class SessionListView { time, project, agent, none }

/** Where pinned sessions sit by project (apps/ios `PinPlace`, the desktop view menu's ViewPin) */
enum class PinPlace { top, project }

/** What the session menu's Copy puts on the clipboard (the desktop's session.copy.*) */
enum class SessionCopy { link, markdown, cwd }

/**
 * The list's own feedback for a send (apps/ios Store/AppModel+PendingSummary.swift): a temporary title and the working
 * mark until the list itself acknowledges the turn. [updatedAt] is the remote clock before sending
 */
data class PendingSummary(val originalTitle: String, val title: String?, val working: Boolean = true, val turns: Int, val updatedAt: String)

/**
 * The session list's state and the session actions its rows, its menus and the chat's header menu post (apps/ios
 * AppModel's list, projects and session actions, AppModel+Links / +Files / +PendingSummary): grouping prefs, the desktop's
 * sections and removed folders, content search hits, open projects, the list's send feedback, and the toast's undo.
 * [AppModel] owns one and hooks it into its message dispatch
 */
class SessionLists(private val model: AppModel, private val context: Context, private val scope: CoroutineScope) {
  private val prefs = context.getSharedPreferences("list", Context.MODE_PRIVATE)

  /** The list's grouping and pin placement, kept on the phone (apps/ios Prefs.listView / pinPlace) */
  var view by mutableStateOf(prefs.getString("view", null)?.let { v -> SessionListView.entries.firstOrNull { it.name == v } } ?: SessionListView.time)
    private set
  var pinPlace by mutableStateOf(prefs.getString("pinPlace", null)?.let { v -> PinPlace.entries.firstOrNull { it.name == v } } ?: PinPlace.top)
    private set

  fun pickView(next: SessionListView) {
    view = next
    prefs.edit().putString("view", next.name).apply()
  }

  fun pickPinPlace(next: PinPlace) {
    pinPlace = next
    prefs.edit().putString("pinPlace", next.name).apply()
  }

  /** The desktop app's sections, each holding whole projects in its own order; made and filled on the desktop */
  var sections by mutableStateOf<List<SessionSection>>(emptyList())
  /** Folders taken off the list, shared with every project shell on the machine */
  var removedProjects by mutableStateOf<List<String>>(emptyList())
  /** The SSH hosts set up on the computer (HostMsg `connections`); empty until the engine answers */
  var connections by mutableStateOf<List<ConnectionView>>(emptyList())
  /** The history search's content hits, for the query they answered */
  var hits by mutableStateOf<SessionHits?>(null)
  /** Projects folded open in the by-project view; null until one is toggled (the demo opens the latest one) */
  private var opened by mutableStateOf<Set<String>?>(null)
  /** The current toast's button, keyed by the toast's text so a later toast never shows it */
  var toastAction by mutableStateOf<Pair<String, ToastAction>?>(null)
  /** List feedback for sends, until the list acknowledges them (not observed: it only shapes `summaries`) */
  private val pending = HashMap<String, PendingSummary>()
  /** The demo's deleted sessions, for Undo */
  private val trash = HashMap<String, Pair<SessionSummary, SessionView?>>()
  private var searchSeq = 0
  private var searchQuery = ""
  private var askSeq = 0
  private val asks = HashMap<Int, CompletableDeferred<String?>>()

  val openProjects: Set<String>
    get() = opened ?: if (model.live) emptySet() else setOfNotNull(model.summaries.firstOrNull()?.cwd)

  fun toggleOpen(cwd: String) {
    val now = openProjects
    opened = if (cwd in now) now - cwd else now + cwd
  }

  // Toasts

  /** A toast, with a button that keeps it longer (Motion.toastUndo) */
  fun toast(text: String, action: ToastAction? = null) {
    if (action == null) {
      model.showToast(text)
      return
    }
    val entry = text to action
    toastAction = entry
    model.toast = text
    scope.launch {
      delay(UNDO_TOAST_MS)
      if (toastAction === entry) {
        toastAction = null
        if (model.toast == text) model.toast = null
      }
    }
  }

  /** The toast's button was tapped: its action runs and the toast goes */
  fun runToastAction() {
    val (_, action) = toastAction ?: return
    toastAction = null
    model.toast = null
    action.run()
  }

  // The engine's messages

  /** A wire's init: the shared categories, and the hosts' state for the host folders (pushed again on every change) */
  fun init(state: InitState) {
    state.categories?.let(::follow)
    model.post(WebviewMsg.Connections)
  }

  /** The project order's companions every project shell on the machine shares (AppModel keeps the order itself) */
  fun follow(categories: SessionCategories) {
    removedProjects = categories.removedProjects.orEmpty()
    sections = categories.sections.orEmpty()
  }

  /** The list's messages AppModel does not handle itself */
  fun receive(message: HostMsg) {
    when (message) {
      is HostMsg.SessionHits -> {
        // A reply to an older query is dropped rather than mixed in
        if (message.seq != searchSeq) return
        hits = SessionHits(searchQuery, message.hits.associate { it.id to it.snippet }, message.hits.mapNotNull { it.summary })
      }
      is HostMsg.SessionMarkdown -> asks.remove(message.seq)?.complete(message.markdown)
      is HostMsg.Connections -> connections = message.connections
      else -> {}
    }
  }

  /** Carries the list's state over from the model of the wire before (AppModel.resume) */
  fun resume(from: SessionLists) {
    pending.putAll(from.pending)
    sections = from.sections
    removedProjects = from.removedProjects
    connections = from.connections
    hits = from.hits
    opened = from.opened
  }

  // Send feedback (apps/ios AppModel+PendingSummary.swift)

  /** Gives the list the same immediate feedback as the sent bubble, without changing the engine's session view */
  fun sending(view: SessionView, text: String) {
    if (pending[view.id]?.working == true) return
    val existing = model.summaries.firstOrNull { it.id == view.id }
    val original = existing?.title ?: view.title
    val untitled = original.isEmpty() || original == t("session.untitled")
    pending[view.id] = PendingSummary(
      originalTitle = original,
      title = if (view.turns.isEmpty() && view.queued.isNullOrEmpty() && untitled && text.isNotEmpty()) titleFrom(text) else null,
      turns = view.turns.size,
      updatedAt = maxOf(existing?.updatedAt.orEmpty(), view.updatedAt),
    )
    val stamp = model.now().toString()
    // The session push can precede its first list push: it must already show when the chat is left
    val list = if (existing == null) {
      listOf(SessionSummary(id = view.id, title = original, agent = view.agent, cwd = view.cwd, updatedAt = stamp, accountId = view.accountId)) + model.summaries
    } else model.summaries
    model.summaries = list.map { if (it.id == view.id) shown(it.copy(updatedAt = stamp), acknowledging = false) else it }
  }

  /** A list from the engine with the sends it has not acknowledged yet still showing (startup pushes may lag) */
  fun shown(list: List<SessionSummary>, fresh: Boolean = false): List<SessionSummary> =
    if (pending.isEmpty()) list else list.map { shown(it, fresh = fresh) }

  private fun shown(summary: SessionSummary, acknowledging: Boolean = true, fresh: Boolean = false): SessionSummary {
    var p = pending[summary.id] ?: return summary
    if (acknowledging) {
      if (summary.title.isNotEmpty() && summary.title != p.originalTitle && summary.title != t("session.untitled")) p = p.copy(title = null)
      // An old unread / error / background entry does not acknowledge this send; a fresh init may settle an idle one
      val advanced = summary.updatedAt > p.updatedAt
      if (advanced && (summary.state != null || fresh)) p = p.copy(working = false)
    }
    var out = summary
    p.title?.let { out = out.copy(title = it) }
    if (p.working && summary.state != SessionState.waiting) out = out.copy(state = SessionState.working, unread = null)
    if (p.title == null && !p.working) pending.remove(summary.id) else pending[summary.id] = p
    return out
  }

  /** A failure or a finished turn can come before any working list push (especially after reconnecting) */
  fun settle(view: SessionView) {
    val p = pending[view.id]?.takeIf { it.working } ?: return
    val failed = view.status != SessionStatus.starting && view.status != SessionStatus.ready
    val finished = view.turns.lastOrNull() is AgentTurn && view.turns.size > p.turns && !view.running &&
      view.queued.isNullOrEmpty() && model.outgoing[view.id].isNullOrEmpty()
    if (!failed && !finished) return
    if (p.title == null) pending.remove(view.id) else pending[view.id] = p.copy(working = false)
    model.summaries = model.summaries.map {
      if (it.id == view.id && it.state == SessionState.working) it.copy(state = if (failed) SessionState.error else null) else it
    }
  }

  /** The prompt's first line as a temporary title (AppModel.title(from:)) */
  private fun titleFrom(text: String): String {
    val line = text.lineSequence().firstOrNull()?.trim().orEmpty()
    if (line.isEmpty()) return t("session.untitled")
    return if (line.length > TITLE_MAX) line.take(TITLE_MAX) + "…" else line
  }

  // Projects

  /**
   * Every known project for the folder menus, the latest first: folders added by hand, the ones sessions work in, then
   * the registered ones no session used yet
   */
  val projects: List<String>
    get() {
      val removed = removedProjects.toSet()
      val used = model.summaries.sortedByDescending { it.updatedAt }.map { it.cwd }
      return (model.addedProjects + used + model.projectOrder).filter { it.isNotEmpty() && !model.isChat(it) && it !in removed }.distinct()
    }

  /**
   * The by-project view's projects (sessionTree.ts buildSessionTree): the user's order first, then every other folder a
   * session ran in, the latest first. Folders taken off the list stay off, and so does a host folder nobody added
   */
  val listedProjects: List<String>
    get() {
      val registered = model.projectOrder + model.addedProjects
      val removed = removedProjects.toSet()
      val used = model.summaries.sortedByDescending { it.updatedAt }.map { it.cwd }
        .filter { !it.startsWith(SSH) || it in registered }
      return (registered + used).filter { it.isNotEmpty() && !model.isChat(it) && it !in removed }.distinct()
    }

  /** The project leads the by-project view: registered if it was not, then moved before the first one */
  fun moveProjectToTop(cwd: String) {
    if (listedProjects.firstOrNull() == cwd) return
    if (model.live) {
      if (cwd !in model.projectOrder) model.post(WebviewMsg.CategoryOp(CategoryOp.AddProject(cwd)))
      model.post(WebviewMsg.CategoryOp(CategoryOp.ReorderProject(cwd, model.projectOrder.firstOrNull { it != cwd })))
      return
    }
    model.projectOrder = listOf(cwd) + model.projectOrder.filter { it != cwd }
  }

  /** Takes a folder off the list (its sessions stay and still show in a search); the toast puts it back */
  fun removeProject(cwd: String) {
    if (!model.post(WebviewMsg.CategoryOp(CategoryOp.RemoveProject(cwd)))) {
      model.projectOrder = model.projectOrder.filter { it != cwd }
      if (cwd !in removedProjects) removedProjects = removedProjects + cwd
    }
    toast(t("mobile.project.removed", "name" to projectName(cwd)), ToastAction(t("common.undo")) { addProject(cwd) })
  }

  /** A folder back on the list: AppModel's add, and the demo forgets it was removed */
  private fun addProject(cwd: String) {
    model.addProject(cwd)
    if (!model.live) removedProjects = removedProjects - cwd
  }

  /** Files a folder into one of the desktop's sections at its end, or takes it out ([section] null) */
  fun setProjectSection(cwd: String, section: String?) {
    if (model.post(WebviewMsg.CategoryOp(CategoryOp.SetProjectSection(cwd, section)))) return
    sections = sections.map { s ->
      val rest = s.projects.filter { it != cwd }
      s.copy(projects = if (s.id == section) rest + cwd else rest)
    }
  }

  /**
   * A dragged folder dropped in the list (SessionList.tsx `placeProject`): filed in [section] before [before] (at its
   * end without one), or out of any section and moved before [before] among the rest. The engine reorders registered
   * folders only, so a folder listed from its sessions alone is registered first
   */
  fun placeProject(cwd: String, section: String?, before: String?) {
    if (model.live) {
      if (section != null) {
        model.post(WebviewMsg.CategoryOp(CategoryOp.SetProjectSection(cwd, section, before)))
        return
      }
      if (cwd !in model.projectOrder) model.post(WebviewMsg.CategoryOp(CategoryOp.AddProject(cwd)))
      if (sections.any { cwd in it.projects }) model.post(WebviewMsg.CategoryOp(CategoryOp.SetProjectSection(cwd)))
      model.post(WebviewMsg.CategoryOp(CategoryOp.ReorderProject(cwd, before)))
      return
    }
    sections = sections.map { s ->
      val rest = s.projects.filter { it != cwd }
      if (s.id != section) s.copy(projects = rest)
      else s.copy(projects = rest.toMutableList().apply { add(before?.let { indexOf(it) }?.takeIf { it >= 0 } ?: size, cwd) })
    }
    if (section != null) return
    // The demo registers every listed folder in the order shown, so the dropped one lands where the line was
    val order = listedProjects.filter { it != cwd }.toMutableList()
    order.add(before?.let { order.indexOf(it) }?.takeIf { it >= 0 } ?: order.size, cwd)
    model.projectOrder = order
  }

  /** The desktop project menu's "Copy path": the folder as the engine names it (`ssh://alias/path` on a host) */
  fun copyPath(cwd: String) {
    if (clip(cwd)) toast(t("session.project.copied"))
  }

  // Session actions

  fun pin(id: String) {
    val pinned = model.summaries.firstOrNull { it.id == id }?.pinned != true
    model.togglePin(id)
    toast(t(if (pinned) "mobile.pinned" else "mobile.unpinned"))
  }

  /** Archived sessions leave the list (its view menu's "archived" shows them); the toast's undo brings one back */
  fun setArchived(id: String, archived: Boolean) {
    if (!model.post(WebviewMsg.ArchiveSession(id, archived))) {
      model.summaries = model.summaries.map { if (it.id == id) it.copy(archived = if (archived) true else null) else it }
    }
    if (!archived) return
    toast(t("session.archived", "title" to titleOf(id)), ToastAction(t("common.undo")) { setArchived(id, false) })
  }

  /** The menu's "mark as unread / read" */
  fun setUnread(id: String, unread: Boolean) {
    if (model.post(WebviewMsg.SetSessionUnread(id, unread))) return
    model.summaries = model.summaries.map { s ->
      if (s.id != id) s
      else if (s.state == SessionState.background) s.copy(unread = if (unread) true else null)
      else if (unread && s.state == null) s.copy(state = SessionState.unread)
      else if (!unread && s.state == SessionState.unread) s.copy(state = null)
      else s
    }
  }

  fun rename(id: String, title: String) {
    val trimmed = title.trim()
    if (trimmed.isEmpty()) return
    if (model.post(WebviewMsg.RenameSession(id, trimmed))) return
    model.views[id]?.let { model.views[id] = it.copy(title = trimmed) }
    model.summaries = model.summaries.map { if (it.id == id) it.copy(title = trimmed) else it }
  }

  /** Deletes at once, as the desktop does; the toast's undo restores the session */
  fun delete(id: String) {
    val summary = model.summaries.firstOrNull { it.id == id }
    val title = titleOf(id)
    if (!model.post(WebviewMsg.DeleteSession(id)) && summary != null) trash[id] = summary to model.views[id]
    model.summaries = model.summaries.filter { it.id != id }
    model.views.remove(id)
    model.path.removeAll { it == Screen.Chat(id) }
    toast(t("session.deleted", "title" to title), ToastAction(t("common.undo")) { restore(id) })
  }

  private fun restore(id: String) {
    if (model.post(WebviewMsg.RestoreSession(id))) return
    val (summary, view) = trash.remove(id) ?: return
    model.summaries = model.summaries + summary
    view?.let { model.views[id] = it }
  }

  /** Every unread mark goes, for every engine and window on the machine (they share them) */
  fun markAllRead() {
    if (model.post(WebviewMsg.MarkAllRead)) return
    model.summaries = model.summaries.map { s ->
      s.copy(unread = null, state = if (s.state == SessionState.unread) null else s.state)
    }
  }

  /** Moves a session to another project folder: the agent works there from its next open on */
  fun move(id: String, cwd: String) {
    if (!model.post(WebviewMsg.MoveSession(id, cwd))) {
      model.summaries = model.summaries.map { if (it.id == id) it.copy(cwd = cwd) else it }
      model.views[id]?.let { model.views[id] = it.copy(cwd = cwd) }
    }
    toast(t("session.movedTo", "title" to titleOf(id), "project" to projectName(cwd)))
  }

  /**
   * The history search over saved conversations; its content hits land in [hits] (the engine answers `sessionHits`,
   * the demo searches its own views)
   */
  fun search(query: String) {
    val terms = searchTerms(query)
    if (terms.isEmpty()) {
      searchSeq++
      searchQuery = ""
      hits = null
      return
    }
    val q = terms.joinToString(" ")
    if (model.live) {
      searchSeq++
      searchQuery = q
      model.post(WebviewMsg.SearchSessions(q, searchSeq))
      return
    }
    val found = model.views.mapNotNull { (id, view) -> SessionSearch.snippet(view, terms)?.let { id to it } }.toMap()
    hits = SessionHits(q, found)
  }

  /**
   * The session as Markdown, the engine's own export (the desktop's Copy → Markdown), or another of its exports
   * ([format] text); the demo exports its view
   */
  suspend fun sessionMarkdown(id: String, format: ExportFormat = ExportFormat.markdown): String? {
    if (!model.live) {
      val view = model.views[id] ?: return null
      val name = model.agent(view.agent).name
      return if (format == ExportFormat.text) SessionExport.text(view, name) else SessionExport.markdown(view, name, model.now())
    }
    val seq = ++askSeq
    val answer = CompletableDeferred<String?>()
    asks[seq] = answer
    // Markdown asks without the field, as before it existed
    model.post(WebviewMsg.SessionMarkdown(id, seq, format.takeIf { it != ExportFormat.markdown }))
    return withTimeoutOrNull(ASK_TIMEOUT_MS) { answer.await() }.also { asks.remove(seq) }
  }

  /** Copies the session's link, its Markdown or its folder; the Markdown is the engine's export, so it is awaited */
  fun copy(id: String, kind: SessionCopy) {
    scope.launch {
      val text = when (kind) {
        SessionCopy.link -> SessionLinks.link(id)
        SessionCopy.markdown -> sessionMarkdown(id)
        SessionCopy.cwd -> model.views[id]?.cwd ?: model.summaries.firstOrNull { it.id == id }?.cwd
      }
      if (text.isNullOrEmpty()) {
        toast(t("session.copyFailed", "detail" to t("mobile.engineNeeded")))
        return@launch
      }
      if (!clip(text)) return@launch
      toast(t(when (kind) { SessionCopy.link -> "session.copied.link"; SessionCopy.markdown -> "session.copied.markdown"; SessionCopy.cwd -> "session.copied.cwd" }))
    }
  }

  /** The phone holds the session's view (a JSON export needs it) */
  fun holdsView(id: String): Boolean = model.views.containsKey(id)

  /**
   * Writes the export to a file of the app's cache and hands it to the system's share sheet. The Markdown and the plain
   * text are [sessionMarkdown]'s (with an engine, the desktop's own exports, so a session the phone never opened exports
   * too); the JSON is the view the phone holds
   */
  fun export(id: String, format: ExportFormat) {
    scope.launch {
      val title = titleOf(id)
      val body = if (format == ExportFormat.json) model.views[id]?.let(SessionExport::json) else sessionMarkdown(id, format)
      if (body.isNullOrEmpty()) {
        toast(t("mobile.engineNeeded"))
        return@launch
      }
      val (ext, mime) = when (format) {
        ExportFormat.json -> "json" to "application/json"
        ExportFormat.text -> "txt" to "text/plain"
        else -> "md" to "text/markdown"
      }
      val shared = runCatching {
        withContext(Dispatchers.IO) {
          val dir = File(context.cacheDir, EXPORT_DIR).apply { mkdirs() }
          File(dir, "${SessionExport.fileName(title)}.$ext").apply { writeText(body) }
        }
      }.getOrElse { toast(it.localizedMessage ?: it.toString()); return@launch }
      val uri = FileProvider.getUriForFile(context, "${context.packageName}.files", shared)
      val send = Intent(Intent.ACTION_SEND).apply {
        type = mime
        putExtra(Intent.EXTRA_STREAM, uri)
        putExtra(Intent.EXTRA_TITLE, shared.name)
        clipData = ClipData.newRawUri(shared.name, uri)
        addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
      }
      context.startActivity(Intent.createChooser(send, null).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    }
  }

  private fun titleOf(id: String): String = model.views[id]?.title ?: model.summaries.firstOrNull { it.id == id }?.title.orEmpty()

  /** Puts [text] on the clipboard; true when the app should confirm it, as Android 13+ shows its own confirmation */
  private fun clip(text: String): Boolean {
    context.getSystemService(ClipboardManager::class.java)?.setPrimaryClip(ClipData.newPlainText(t("common.copy"), text))
    return Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU
  }

  companion object {
    private const val SSH = "ssh://"
    /** A toast with Undo stays this long (apps/ios Motion.toastUndo) */
    private const val UNDO_TOAST_MS = 6_000L
    /** How long an export waits for the engine's answer */
    private const val ASK_TIMEOUT_MS = 15_000L
    private const val TITLE_MAX = 40
    private const val EXPORT_DIR = "exports"
  }
}

/** A folder's last path part, the name the list shows for its project */
fun projectName(cwd: String): String = cwd.split('/', '\\').lastOrNull { it.isNotEmpty() } ?: cwd
