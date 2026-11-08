package cc.hotic.acpira.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.heightIn
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.IconButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTagsAsResourceId
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.logic.isUnread
import cc.hotic.acpira.model.ExportFormat
import cc.hotic.acpira.store.AppModel
import cc.hotic.acpira.store.Screen
import cc.hotic.acpira.store.SessionCopy

// The session's "…" menu (apps/ios Design/NativeChrome.swift NativeSessionMenu, its actions Chat/ChatScreen.swift
// HeaderActions): the account and the session's changes, pin, unread and rename, copy / move / export in submenus, the
// chat's own pages, archive and delete. A list row's long press shows it too, without the chat's own entries (the
// desktop's row menu) and without delete, which the row's swipe offers. Submenus replace the level they came from, with
// their head as the way back (ui/Menu.kt).

/** The chat's own entries of its "…" menu; null ones are left out (a list row passes none) */
class ChatEntries(
  val account: (() -> Unit)? = null,
  /** The files the session changed (panel.changes) */
  val changes: (() -> Unit)? = null,
  /** The session's subagent graph, [subagentCounts] its subtitle; null while the session has no subagents */
  val subagents: (() -> Unit)? = null,
  val subagentCounts: String = "",
  /** The project's terminal page */
  val terminal: (() -> Unit)? = null,
  val settings: (() -> Unit)? = null,
)

/**
 * Session [id]'s menu. [onRename] and [onAddFolder] raise the page's own dialogs (the title, a folder to move into);
 * [onArchived] runs after an archive or restore (the chat leaves for the list); [onDelete] null: no delete (a list row's
 * swipe deletes); [chat] the chat's own entries
 */
@Composable
fun SessionMenu(
  model: AppModel,
  id: String,
  expanded: Boolean,
  onDismiss: () -> Unit,
  onRename: () -> Unit,
  onAddFolder: () -> Unit,
  onArchived: (Boolean) -> Unit = {},
  onDelete: (() -> Unit)? = null,
  chat: ChatEntries? = null,
) {
  // null: the top level; else a submenu (COPY, EXPORT, MOVE, or FOLDERS + a host alias, "" this computer's folders)
  var level by remember { mutableStateOf<String?>(null) }
  val close = { onDismiss(); level = null }
  AppMenu(expanded = expanded, onDismiss = close, modifier = Modifier.heightIn(max = Metrics.menuRow * MENU_ROWS)) {
    // Read once the menu is out, so a list's rows do not follow every session's changes for a menu that is shut
    val lists = model.lists
    val summary = model.summaries.firstOrNull { it.id == id }
    val cwd = model.views[id]?.cwd ?: summary?.cwd.orEmpty()
    val pinned = summary?.pinned == true
    val unread = summary?.let(::isUnread) == true
    val archived = summary?.archived == true
    val inChat = model.isChat(cwd)
    val projects = lists.projects
    val pick = { action: () -> Unit -> close(); action() }
    when (val at = level) {
      null -> {
        if (chat?.account != null || chat?.changes != null) {
          chat.account?.let { AppMenuItem(t("common.account"), onClick = { pick(it) }, icon = { MenuGlyph("userRound") }) }
          chat.changes?.let { AppMenuItem(t("panel.changes"), onClick = { pick(it) }, icon = { MenuGlyph("filePen") }) }
          AppMenuGap()
        }
        AppMenuItem(
          t(if (pinned) "common.unpin" else "common.pin"), onClick = { pick { lists.pin(id) } },
          icon = { MenuGlyph(if (pinned) "pinOff" else "pin") }, modifier = Modifier.testTag("menu.pin"),
        )
        AppMenuItem(
          t(if (unread) "session.markRead" else "session.markUnread"), onClick = { pick { lists.setUnread(id, !unread) } },
          icon = { MenuGlyph(if (unread) "mailCheck" else "mail") }, modifier = Modifier.testTag("menu.unread"),
        )
        AppMenuItem(t("common.rename"), onClick = { pick(onRename) }, icon = { MenuGlyph("pencil") }, modifier = Modifier.testTag("menu.rename"))
        AppMenuItem(t("session.copy"), onClick = { level = COPY }, icon = { MenuGlyph("chevronRight") }, modifier = Modifier.testTag("menu.copy"))
        AppMenuItem(t("session.moveToProject"), onClick = { level = MOVE }, icon = { MenuGlyph("chevronRight") }, modifier = Modifier.testTag("menu.move"))
        AppMenuItem(t("session.export"), onClick = { level = EXPORT }, icon = { MenuGlyph("chevronRight") }, modifier = Modifier.testTag("menu.export"))
        if (chat?.subagents != null || chat?.terminal != null || chat?.settings != null) {
          AppMenuGap()
          chat.subagents?.let {
            AppMenuItem(t("subagents.graph"), subtitle = chat.subagentCounts.ifEmpty { null }, onClick = { pick(it) }, icon = { MenuGlyph("workflow") })
          }
          chat.terminal?.let { AppMenuItem(t("panel.terminal"), onClick = { pick(it) }, icon = { MenuGlyph("squareTerminal") }) }
          chat.settings?.let { AppMenuItem(t("session.settings"), onClick = { pick(it) }, icon = { MenuGlyph("settings") }) }
        }
        // Archive keeps the session, delete is the one with no way back; a list row leaves delete to its swipe
        AppMenuGap()
        AppMenuItem(
          t(if (archived) "session.unarchive" else "session.archive"),
          onClick = { pick { lists.setArchived(id, !archived); onArchived(!archived) } },
          icon = { MenuGlyph(if (archived) "archiveRestore" else "archive") }, modifier = Modifier.testTag("menu.archive"),
        )
        onDelete?.let {
          AppMenuItem(
            t("session.deleteForever"), onClick = { pick(it) }, danger = true,
            icon = { MenuGlyph("trash2", danger = true) }, modifier = Modifier.testTag("menu.delete"),
          )
        }
      }
      COPY -> {
        AppMenuBack(t("session.copy"), onClick = { level = null })
        AppMenuGap()
        AppMenuItem(t("session.copy.link"), onClick = { pick { lists.copy(id, SessionCopy.link) } }, icon = { MenuGlyph("link") }, modifier = Modifier.testTag("menu.copy.link"))
        AppMenuItem(t("session.copy.markdown"), onClick = { pick { lists.copy(id, SessionCopy.markdown) } }, icon = { MenuGlyph("fileText") }, modifier = Modifier.testTag("menu.copy.markdown"))
        if (cwd.isNotEmpty()) {
          AppMenuItem(t("session.copy.cwd"), onClick = { pick { lists.copy(id, SessionCopy.cwd) } }, icon = { MenuGlyph("folder") }, modifier = Modifier.testTag("menu.copy.cwd"))
        }
      }
      EXPORT -> {
        AppMenuBack(t("session.export"), onClick = { level = null })
        AppMenuGap()
        AppMenuItem(t("session.export.markdown"), onClick = { pick { lists.export(id, ExportFormat.markdown) } }, icon = { MenuGlyph("fileText") }, modifier = Modifier.testTag("menu.export.markdown"))
        // Built from the view the phone holds, which a list row's session may not have
        if (lists.holdsView(id)) {
          AppMenuItem(t("session.export.json"), onClick = { pick { lists.export(id, ExportFormat.json) } }, icon = { MenuGlyph("braces") }, modifier = Modifier.testTag("menu.export.json"))
        }
        // Only what was said, prompts and replies without tools or thinking
        AppMenuItem(t("session.export.text"), onClick = { pick { lists.export(id, ExportFormat.text) } }, icon = { MenuGlyph("textAlignStart") }, modifier = Modifier.testTag("menu.export.text"))
      }
      MOVE -> {
        // The new session's folder menu: this computer's folders, each SSH host's, then a folder typed in
        val (local, hosts) = places(cwd, projects, inChat)
        AppMenuBack(t("session.moveToProject"), onClick = { level = null })
        AppMenuGap()
        if (local.isNotEmpty()) {
          AppMenuItem(t("mobile.projects.local"), onClick = { level = FOLDERS + LOCAL }, icon = { MenuGlyph("chevronRight") }, modifier = Modifier.testTag("menu.move.local"))
        }
        for (h in hosts.keys) {
          AppMenuItem(h, subtitle = t("mobile.projects.remote"), onClick = { level = FOLDERS + h }, icon = { MenuGlyph("chevronRight") })
        }
        if (local.isNotEmpty() || hosts.isNotEmpty()) AppMenuGap()
        AppMenuItem(t("mobile.projects.add"), onClick = { pick(onAddFolder) }, icon = { MenuGlyph("folderPlus") }, modifier = Modifier.testTag("menu.move.add"))
      }
      else -> {
        // One machine's folders; the session's own is ticked, so picking it moves nothing
        val host = at.removePrefix(FOLDERS)
        val (local, hosts) = places(cwd, projects, inChat)
        val folders = if (host == LOCAL) local else hosts[host].orEmpty()
        AppMenuBack(if (host == LOCAL) t("mobile.projects.local") else host, onClick = { level = MOVE })
        AppMenuGap()
        for (p in folders) {
          AppMenuItem(
            folderLabel(p), subtitle = folderSubtitle(p), checked = !inChat && p == cwd,
            onClick = { pick { if (p != cwd) lists.move(id, p) } },
            icon = { MenuGlyph(if (host == LOCAL) "folder" else "globe") }, modifier = Modifier.testTag("menu.move.to:$p"),
          )
        }
      }
    }
  }
}

/**
 * The folders a session can move to (NativeProjectMenu.places): its own first unless it is a chat, then the known
 * projects; this computer's, and each SSH host's by alias in the order they first turn up
 */
private fun places(cwd: String, projects: List<String>, chat: Boolean): Pair<List<String>, Map<String, List<String>>> {
  val folders = ((if (chat) emptyList() else listOf(cwd)) + projects).filter { it.isNotEmpty() }.distinct()
  val local = folders.filter { remoteAlias(it) == null }
  val hosts = LinkedHashMap<String, MutableList<String>>()
  for (f in folders) remoteAlias(f)?.let { hosts.getOrPut(it) { mutableListOf() } += f }
  return local to hosts
}

/** `ssh://<alias>/<path>`'s alias (apps/ios RemoteFolder.parse); null for a local folder */
internal fun remoteAlias(folder: String): String? {
  if (!folder.startsWith(SSH)) return null
  val rest = folder.removePrefix(SSH)
  val slash = rest.indexOf('/')
  return if (slash > 0) rest.substring(0, slash) else null
}

/** A folder's own name, on a host its path's */
private fun folderLabel(folder: String): String {
  val path = remoteAlias(folder)?.let { folder.removePrefix(SSH + it) } ?: folder
  return path.trimEnd('/', '\\').split('/', '\\').lastOrNull { it.isNotEmpty() } ?: path
}

/** The folder's parent as a short subtitle (NativeProjectMenu.subtitle): its last two parts, `…/` before more */
private fun folderSubtitle(folder: String): String {
  val path = remoteAlias(folder)?.let { folder.removePrefix(SSH + it) } ?: folder
  val parts = path.trimEnd('/', '\\').split('/', '\\').filter { it.isNotEmpty() }.dropLast(1)
  if (parts.isEmpty()) return if (path.startsWith("/")) "/" else "."
  val tail = parts.takeLast(2).joinToString("/")
  return if (parts.size > 2) "…/$tail" else (if (path.startsWith("/")) "/" else "") + tail
}

/** The session's title, typed anew (apps/ios the list's and the chat's rename alert) */
@Composable
fun RenameDialog(initial: String, onDismiss: () -> Unit, onSave: (String) -> Unit) {
  var title by remember { mutableStateOf(initial) }
  AlertDialog(
    onDismissRequest = onDismiss,
    title = { Text(t("common.rename")) },
    text = {
      // A dialog is a window of its own: its tags need the resource-id switch again
      Column(Modifier.semantics { testTagsAsResourceId = true }) {
        TextInput(title, { title = it }, t("session.titleAria"), tag = "rename.field", boxed = true, onDone = { onSave(title); onDismiss() })
      }
    },
    confirmButton = {
      TextButton(onClick = { onSave(title); onDismiss() }, enabled = title.isNotBlank(), modifier = Modifier.semantics { testTagsAsResourceId = true }.testTag("rename.save")) {
        Text(t("mobile.save"), color = if (title.isNotBlank()) Palette.fg1 else Palette.fg3)
      }
    },
    dismissButton = { TextButton(onClick = onDismiss) { Text(t("mobile.cancel"), color = Palette.fg1) } },
  )
}

/** The chat's delete asks first (apps/ios ChatScreen confirmationDialog); the list's swipe deletes at once */
@Composable
fun DeleteConfirm(title: String, onDismiss: () -> Unit, onDelete: () -> Unit) {
  AlertDialog(
    onDismissRequest = onDismiss,
    title = { Text(t("mobile.deleteConfirm", "title" to title)) },
    confirmButton = {
      TextButton(onClick = { onDismiss(); onDelete() }, modifier = Modifier.semantics { testTagsAsResourceId = true }.testTag("delete.confirm")) {
        Text(t("common.delete"), color = Palette.danger)
      }
    },
    dismissButton = { TextButton(onClick = onDismiss) { Text(t("mobile.cancel"), color = Palette.fg1) } },
  )
}

/**
 * The chat bar's "…" button and its session menu, with the dialogs it raises (rename, delete, a folder to move into).
 * [chat] the chat's own entries: the settings by default; screens that have the account, changes, subagent or terminal
 * pages pass theirs
 */
@Composable
fun ChatMenuButton(model: AppModel, id: String, chat: ChatEntries? = null) {
  var menu by remember { mutableStateOf(false) }
  var renaming by remember { mutableStateOf(false) }
  var deleting by remember { mutableStateOf(false) }
  var adding by remember { mutableStateOf(false) }
  Box {
    IconButton(onClick = { menu = true }, modifier = Modifier.testTag("chat.more")) {
      Lucide("ellipsis", Palette.fg1, size = Metrics.iconCtl, contentDescription = t("session.more"))
    }
    SessionMenu(
      model, id, expanded = menu, onDismiss = { menu = false },
      onRename = { renaming = true },
      onAddFolder = { adding = true },
      // An archived chat leaves the list; going back shows where it went
      onArchived = { archived -> if (archived && model.path.lastOrNull() == Screen.Chat(id)) model.pop() },
      onDelete = { deleting = true },
      chat = chat ?: ChatEntries(settings = { model.push(Screen.Settings) }),
    )
  }
  val title = model.views[id]?.title ?: model.summaries.firstOrNull { it.id == id }?.title.orEmpty()
  if (renaming) RenameDialog(title, onDismiss = { renaming = false }) { model.lists.rename(id, it) }
  if (deleting) DeleteConfirm(title, onDismiss = { deleting = false }) { model.lists.delete(id) }
  // As in the new session's folder menu, a typed folder joins the project list; the session moves into it
  AddProject(model, adding, onClose = { adding = false }, onAdded = { model.lists.move(id, it) })
}

private const val COPY = "copy"
private const val EXPORT = "export"
private const val MOVE = "move"
/** A machine's folders under the move menu: this prefix and the host's alias ("" this computer) */
private const val FOLDERS = "move:"
private const val LOCAL = ""
private const val SSH = "ssh://"
/** The menu's height in rows before it scrolls */
private const val MENU_ROWS = 10.5f
