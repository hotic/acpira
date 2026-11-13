package cc.hotic.acpira.ui

import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.keyframes
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.BoxWithConstraints
import cc.hotic.acpira.design.footFade
import cc.hotic.acpira.design.readablePadding
import cc.hotic.acpira.design.readableWidth
import cc.hotic.acpira.design.topFade
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material3.CenterAlignedTopAppBar
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material3.FloatingActionButtonDefaults
import androidx.compose.material3.IconButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.draw.scale
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.onLongClick
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.Dp
import cc.hotic.acpira.design.LocalMotion
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.design.VendorMark
import cc.hotic.acpira.i18n.L10n
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.logic.SessionListRows
import cc.hotic.acpira.logic.SessionSearch
import cc.hotic.acpira.logic.searchTerms
import cc.hotic.acpira.model.ConnectionState
import cc.hotic.acpira.model.ConnectionView
import cc.hotic.acpira.model.SessionSection
import cc.hotic.acpira.model.SessionState
import cc.hotic.acpira.model.SessionSummary
import cc.hotic.acpira.store.AppModel
import cc.hotic.acpira.store.Connection
import cc.hotic.acpira.store.PinPlace
import cc.hotic.acpira.store.Screen
import cc.hotic.acpira.store.SessionListView
import cc.hotic.acpira.store.projectName
import kotlinx.coroutines.delay
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.FormatStyle
import java.util.Locale

/** One captioned run of the list (置顶, 今天 …); dated runs leave the time off their rows, the caption says it */
private data class SessionGroup(val id: String, val caption: String?, val sessions: List<SessionSummary>, val dated: Boolean = false)

/**
 * The session list (apps/ios Sessions/SessionListScreen.swift): every session, grouped as picked in the bar's filter
 * menu. By time: the sessions pinned to the top, then day captions. By project: the same pinned run (or each project's
 * pins leading it), then the desktop app's sections, each project as a header folding open in place, the latest few,
 * the rest behind "N more". By channel: pinned, then a caption per agent. Ungrouped: one run. The archived view lists
 * the archived sessions alone. A search lists one run across every project, matching the conversations too (a content
 * hit shows the text around its match under the title). Material's centred top bar holds the filter and settings; the
 * search field and the new-session button ride the bottom, where the iOS app keeps them. A row's long press opens the
 * session menu (ui/SessionMenu.kt) and, by project, moving on drags it (ui/ListDrag.kt); a swipe bares Delete
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SessionListScreen(model: AppModel) {
  val lists = model.lists
  var query by rememberSaveable { mutableStateOf("") }
  // The view menu's "archived": the list shows the archived sessions, and only those
  var archivedOnly by rememberSaveable { mutableStateOf(false) }
  // The row standing swiped open, Delete bared (SwipeToDelete.kt)
  var swiped by remember { mutableStateOf<String?>(null) }
  // Per open project, how many sessions past its latest few are out (SessionList.tsx `more`)
  var moreShown by remember { mutableStateOf(mapOf<String, Int>()) }
  // The session the rename dialog and the move menu's "Add folder" are for
  var renaming by remember { mutableStateOf<String?>(null) }
  var movingTo by remember { mutableStateOf<String?>(null) }
  val drag = remember { ListDrag() }
  val listState = rememberLazyListState()
  val terms = searchTerms(query)
  // Paired remote access: the followed computer's state takes the page until it has sessions to list. The bar sits over
  // both, so the frame giving way to the list moves nothing in it
  val pairing = LocalPairing.current
  val covered = pairing != null && covers(pairing, model)
  // Listed folders alone keep the filter too: from another view its menu is the way back to them
  val filter = !covered && terms.isEmpty() && !(model.summaries.isEmpty() && lists.listedProjects.isEmpty())
  // Standing in a wide window's sidebar (ui/Root.kt Split), and how to put it away there
  val inSidebar = model.listInSidebar
  val hide = LocalSidebarHide.current

  // The conversation search goes out once typing pauses (SessionList.tsx SEARCH_DEBOUNCE)
  LaunchedEffect(query) {
    if (query.isNotEmpty()) delay(SEARCH_DEBOUNCE_MS)
    lists.search(query)
  }
  // A scroll puts a swiped row back, as in the system's lists
  LaunchedEffect(listState.isScrollInProgress) { if (listState.isScrollInProgress) swiped = null }

  val listMenu = @Composable {
    ListViewMenu(
      view = lists.view, onView = lists::pickView,
      pinPlace = lists.pinPlace, onPinPlace = lists::pickPinPlace,
      archived = archivedOnly, onArchived = { archivedOnly = it },
      unread = model.summaries.any { it.unread == true || it.state == SessionState.unread },
      onMarkAllRead = lists::markAllRead,
    )
  }
  Scaffold(
    containerColor = Palette.bg0,
    topBar = {
      Column {
      CenterAlignedTopAppBar(
        title = {
          // In the sidebar the computer moves to the foot (SidebarFoot) and the bar holds no title, as on iPadOS
          if (inSidebar) Unit
          else if (pairing != null) ListBarTitle(pairing)
          else Text(t("mobile.home"), fontWeight = FontWeight.SemiBold, fontSize = Type.barTitle)
        },
        // The filter leads the bar on the phone and in the sidebar alike
        navigationIcon = { if (filter) listMenu() },
        actions = {
          // In the sidebar settings sit in the foot, and the bar's end holds the hide button alone
          if (hide != null) {
            IconButton(onClick = hide, modifier = Modifier.testTag("split.hide")) {
              Lucide("panelLeft", Palette.fg1, size = Metrics.iconCtl, contentDescription = t("desktop.hideSidebar"))
            }
          } else if (!inSidebar) {
            IconButton(onClick = { model.push(Screen.Settings) }, modifier = Modifier.testTag("list.settings")) {
              Lucide("settings", Palette.fg1, size = Metrics.iconCtl, contentDescription = t("session.settings"))
            }
          }
        },
        colors = TopAppBarDefaults.centerAlignedTopAppBarColors(containerColor = Palette.bg0, scrolledContainerColor = Palette.bg0),
      )
      // In the sidebar the search stands under the bar, as the desktop's column has it (apps/ios SystemListBar), on
      // the page's colour as the bar is: rows leave under it through the top fade, not beside the field
      if (inSidebar && !covered) {
        Box(Modifier.fillMaxWidth().background(Palette.bg0).padding(horizontal = Metrics.pad).padding(bottom = Metrics.s2)) {
          SearchPill(query, { query = it })
        }
      }
      }
    },
    bottomBar = {
      if (!covered) {
        // In the sidebar the foot holds settings and the computer; new sessions come from the page beside it
        if (inSidebar) SidebarFoot(model, pairing)
        // A new session starts in the latest project, as the iOS list's button does
        else ListBottomBar(query, { query = it }) { model.open(Screen.New(cwd = lists.projects.firstOrNull())) }
      }
    },
  ) { padding ->
    if (covered) {
      PairHeroList(pairing!!, Modifier.padding(padding))
      return@Scaffold
    }
    val scene = ListScene(
      model = model, drag = drag, terms = terms, archivedOnly = archivedOnly, swiped = swiped, moreShown = moreShown,
      onSwiped = { swiped = it }, onMore = { cwd, n -> moreShown = if (n == null) moreShown - cwd else moreShown + (cwd to n) },
      onRename = { renaming = it }, onAddFolder = { movingTo = it },
    )
    // Rows leave under the bar through a fade, once the list has moved off its top
    val scrolled by remember(listState) { derivedStateOf { listState.canScrollBackward } }
    // A phone held sideways reads the list in the centred column, its bounds still full width so a drag anywhere scrolls
    BoxWithConstraints(Modifier.fillMaxSize()) {
      LazyColumn(
        state = listState,
        modifier = Modifier.fillMaxSize().topFade(scrolled, padding.calculateTopPadding()).testTag("list.sessions")
          // A drag held near the top or bottom scrolls the list under it
          .listDragAutoScroll(drag, listState, padding.calculateTopPadding(), padding.calculateBottomPadding()),
        contentPadding = readablePadding(maxWidth, top = padding.calculateTopPadding(), bottom = padding.calculateBottomPadding() + Metrics.msgGap),
      ) { scene.content(this) }
      ListDragLayer(drag)
    }
  }
  renaming?.let { id ->
    val title = model.summaries.firstOrNull { it.id == id }?.title.orEmpty()
    RenameDialog(title, onDismiss = { renaming = null }) { lists.rename(id, it) }
  }
  // The move menu's "Add folder", as in the new session's folder menu: the folder joins the list, the session moves in
  AddProject(model, movingTo != null, onClose = { movingTo = null }, onAdded = { folder -> movingTo?.let { lists.move(it, folder) } })
}

/**
 * One render of the list: what it draws from and how it is grouped, and the rows it lays into the lazy column. Made
 * afresh on every recomposition of the screen; its rows read the model again where they need more
 */
private class ListScene(
  val model: AppModel,
  val drag: ListDrag,
  val terms: List<String>,
  val archivedOnly: Boolean,
  val swiped: String?,
  val moreShown: Map<String, Int>,
  val onSwiped: (String?) -> Unit,
  val onMore: (String, Int?) -> Unit,
  val onRename: (String) -> Unit,
  val onAddFolder: (String) -> Unit,
) {
  val lists = model.lists
  val now: Instant = model.now()
  val searching = terms.isNotEmpty()
  // Content hits count for the query they answered only
  val snippets: Map<String, String>? = lists.hits?.takeIf { it.query == terms.joinToString(" ") }?.snippets
  /** The sessions the list draws from: archived ones only while the view menu asks for them */
  val listed: List<SessionSummary> = model.summaries.filter { (it.archived == true) == archivedOnly }
  val ordered: List<SessionSummary>
  val byProject = lists.view == SessionListView.project && !searching && !archivedOnly
  val pinsInProject = byProject && lists.pinPlace == PinPlace.project
  /** The chat open beside the sidebar: it stays outside paging even when it is older and idle */
  val activeId: String? = (model.path.firstOrNull() as? Screen.Chat)?.id?.takeIf { model.listInSidebar }
  val listedProjects = lists.listedProjects

  init {
    val known = model.summaries.map { it.id }.toSet()
    val extra = if (snippets == null) emptyList() else lists.hits?.summaries.orEmpty().filter { it.id !in known && (it.archived == true) == archivedOnly }
    val pool = listed + extra
    val shown = if (!searching) pool else pool.filter {
      snippets?.get(it.id) != null || SessionSearch.matches(listOf(it.title, model.agent(it.agent).name, model.placeName(it.cwd)), terms)
    }
    // Pinned first, then the latest
    ordered = shown.sortedWith(compareByDescending<SessionSummary> { it.pinned == true }.thenByDescending { it.updatedAt })
  }

  fun content(scope: LazyListScope) = with(scope) {
    // By project, the chats' group still stands (its "new chat" row) when nothing else does
    if (ordered.isEmpty() && !(byProject && (listedProjects.isNotEmpty() || model.chatRoot != null))) {
      item(key = "empty") { EmptyNote(model, searching, snippets != null) }
    }
    for (group in groups()) group(group)
    if (byProject) {
      projectGroups()
      chatGroup()
    }
  }

  /**
   * By time: pinned, then today / yesterday / this week / this month / older, captions over each. By project: the pinned
   * run alone (none when pins sit in their projects), the projects follow. By channel: pinned, then a run per agent
   * under its name, the agent of the latest session first (sessionView.ts groupSessions). Ungrouped, or a search: one
   * run, its pins first
   */
  private fun groups(): List<SessionGroup> {
    if (searching || lists.view == SessionListView.none) {
      return listOf(SessionGroup("all", null, ordered))
    }
    val top = if (pinsInProject) emptyList() else ordered.filter { it.pinned == true }
    val out = ArrayList<SessionGroup>()
    if (top.isNotEmpty()) out += SessionGroup("pinned", t("session.group.pinned"), top)
    if (byProject) return out
    val rest = ordered.filter { it.pinned != true }
    if (lists.view == SessionListView.agent) {
      for (agent in rest.map { it.agent }.distinct()) {
        out += SessionGroup("agent.$agent", model.agent(agent).name, rest.filter { it.agent == agent })
      }
      return out
    }
    val buckets = rest.groupBy { recency(it.updatedAt, now) }
    for (key in listOf("today", "yesterday", "week", "month", "older")) {
      buckets[key]?.let { out += SessionGroup(key, t("session.when.$key"), it, dated = true) }
    }
    return out
  }

  private fun LazyListScope.group(group: SessionGroup) {
    group.caption?.let { caption -> item(key = "caption:${group.id}") { Caption(caption) } }
    group.sessions.forEachIndexed { i, s ->
      // A caption already says "pinned"; one run without captions (a search) ends its leading pins at the pin rule
      val pinEnd = group.caption == null && s.pinned == true && group.sessions.getOrNull(i + 1)?.let { it.pinned != true } == true
      item(key = "${group.id}:${s.id}") {
        SessionItem(
          s, place = model.placeName(s.cwd),
          // A day caption already says when; pinned rows and search hits have none, so they keep the time
          time = if (group.dated) null else sessionTime(s.updatedAt, now),
          // Captions part the runs and rows sit on space alone; one run without captions parts by hairlines
          divider = i < group.sessions.size - 1 && group.caption == null && !pinEnd,
          snippet = snippets?.get(s.id),
        )
      }
      if (pinEnd) item(key = "pinEnd:${group.id}") { PinRule() }
    }
  }

  /**
   * The by-project view's projects: the desktop app's sections first, each under its name with its projects in its own
   * order, then the projects in none under "Projects"; without sections, the projects alone. Sections are made and
   * filled on the desktop; one with nothing listed here is left out
   */
  private fun LazyListScope.projectGroups() {
    val runs = lists.sections.map { it to it.projects.filter(listedProjects::contains) }.filter { it.second.isNotEmpty() }
    if (runs.isEmpty()) {
      for (cwd in listedProjects) projectGroup(cwd, null, listedProjects)
      return
    }
    val filed = runs.flatMap { it.second }.toSet()
    val rest = listedProjects.filter { it !in filed }
    for ((section, projects) in runs) {
      // A project dropped on a section's caption joins the end of that section
      item(key = "section:${section.id}") { SectionCaption(section.name.ifEmpty { t("session.section.untitled") }, section.id) }
      for (cwd in projects) projectGroup(cwd, section.id, projects)
    }
    if (rest.isNotEmpty()) {
      item(key = "section:") { SectionCaption(t("session.group.projects"), null) }
      for (cwd in rest) projectGroup(cwd, null, rest)
    }
  }

  @Composable
  private fun SectionCaption(text: String, section: String?) {
    Box(Modifier.dropZone(drag, "caption:$section") { item, _, _ -> if (item is ListDragItem.Project) ListDropTarget.Slot(section, null) else null }) {
      Caption(text)
    }
  }

  /**
   * A project in the by-project view: its header, then, open, its pins when they sit in their project (ending at the pin
   * rule) and its other sessions, the latest few only until "N more" is tapped. [run] is the projects listed with it in
   * [section] (null: those in none), where a dragged project lands. The header drags; every row of the group takes a
   * dragged session into the project and a dragged project after it
   */
  private fun LazyListScope.projectGroup(cwd: String, section: String?, run: List<String>) {
    val all = listed.filter { it.cwd == cwd }
    val pins = if (pinsInProject) all.filter { it.pinned == true }.sortedByDescending { it.updatedAt } else emptyList()
    val sessions = all.filter { it.pinned != true }.sortedByDescending { it.updatedAt }
    val open = cwd in lists.openProjects
    // The latest few (all of them when only one more would hide), plus the desktop's live / attention exceptions
    val cut = if (sessions.size <= PER_PROJECT + 1) sessions.size else PER_PROJECT
    val extra = moreShown[cwd] ?: 0
    val page = SessionListRows(sessions, cut, extra, activeId)
    val rows = if (open) page.rows else emptyList()
    // The "N more" row stays once paging began, reading "show less" with everything out
    val paged = open && page.canFold
    // A project landing after this one draws its line under the group's last row, not between the header and its rows
    val insert = insertEdge(cwd, section, run)
    val lastRow = if (open && !paged) (rows.lastOrNull() ?: pins.lastOrNull())?.id else null
    val tail = if (insert == DropEdge.Bottom) DropEdge.Bottom else null
    val zone = { header: Boolean, key: String ->
      Modifier.dropZone(drag, "zone:$cwd:$key") { item, y, height -> dropTarget(item, cwd, section, run, upper = header && y < height / 2) }
    }
    item(key = "project:$cwd") {
      ProjectHeader(
        model, cwd, first = listedProjects.firstOrNull() == cwd, open = open, sessions = all,
        modifier = zone(true, "header").dropMarks(
          hot = drag.over == ListDropTarget.Project(cwd),
          insert = if (insert == DropEdge.Top || (lastRow == null && !paged)) insert else null,
        ),
        onToggle = { if (!closeSwipe()) lists.toggleOpen(cwd) },
        drag = drag, onDrop = ::drop,
      )
    }
    if (open) {
      // Inside its project a row is one line, the mark naming the agent; no caption says when, so every row keeps its time
      for (s in pins) item(key = "p:$cwd:${s.id}") {
        SessionItem(s, null, sessionTime(s.updatedAt, now), divider = false, nested = true, modifier = zone(false, s.id).dropMarks(false, if (s.id == lastRow) tail else null))
      }
      if (pins.isNotEmpty() && (rows.isNotEmpty() || paged)) item(key = "pinEnd:$cwd") { PinRule() }
    }
    for (s in rows) item(key = "p:$cwd:${s.id}") {
      SessionItem(s, null, sessionTime(s.updatedAt, now), divider = false, nested = true, modifier = zone(false, s.id).dropMarks(false, if (s.id == lastRow) tail else null))
    }
    if (paged) item(key = "more:$cwd") {
      MoreRow(page.rest, model::agentName, tag = "more:$cwd", modifier = zone(false, "more").dropMarks(false, tail)) {
        if (!closeSwipe()) onMore(cwd, if (page.rest.isEmpty()) null else extra + MORE_PAGE)
      }
    }
  }

  /**
   * The chats (sessions in no project) under the projects, as the desktop list puts them: a caption, then the rows at the
   * top level's inset, the latest few until "N more"; with none yet, a row starting one. Shown where the engine keeps chats
   */
  private fun LazyListScope.chatGroup() {
    if (model.chatRoot == null) return
    val all = listed.filter { model.isChat(it.cwd) }
    val pins = if (pinsInProject) all.filter { it.pinned == true }.sortedByDescending { it.updatedAt } else emptyList()
    // Pinned to the top, a chat already sits in the pinned run
    val sessions = all.filter { it.pinned != true }.sortedByDescending { it.updatedAt }
    val cut = if (sessions.size <= PER_PROJECT + 1) sessions.size else PER_PROJECT
    val extra = moreShown[CHATS_KEY] ?: 0
    val page = SessionListRows(sessions, cut, extra, activeId)
    item(key = "caption:chats") { Caption(t("session.group.chats")) }
    for (s in pins) item(key = "c:${s.id}") { SessionItem(s, null, sessionTime(s.updatedAt, now), divider = false) }
    if (pins.isNotEmpty() && page.rows.isNotEmpty()) item(key = "pinEnd:chats") { PinRule() }
    for (s in page.rows) item(key = "c:${s.id}") { SessionItem(s, null, sessionTime(s.updatedAt, now), divider = false) }
    if (page.canFold) item(key = "more:chats") {
      MoreRow(page.rest, model::agentName, nested = false, tag = "more:chats") {
        if (!closeSwipe()) onMore(CHATS_KEY, if (page.rest.isEmpty()) null else extra + MORE_PAGE)
      }
    }
    if (all.isEmpty()) item(key = "newChat") { NewChatRow { if (!closeSwipe()) model.open(Screen.New(cwd = "")) } }
  }

  /**
   * One session with its menu: the chat's "…" menu without the chat's own pages, as the desktop's row menu is its
   * header's. Delete is the row's swipe, as in the system's lists; by project a row drags onto another project
   */
  @Composable
  private fun SessionItem(s: SessionSummary, place: String?, time: String?, divider: Boolean, nested: Boolean = false, snippet: String? = null, modifier: Modifier = Modifier) {
    var menu by remember { mutableStateOf(false) }
    // By project a session drags onto another project; pinned ones stay put, as on the desktop (sessionTree.ts draggable)
    val dragged = if (filing && s.pinned != true) ListDragItem.Session(s.id) else null
    // Deleted at once, as on the desktop: the toast's undo brings it back
    SwipeToDelete(s.id, swiped, onSwiped, onDelete = { lists.delete(s.id) }, modifier = modifier) {
      Box {
        SessionRow(
          s, agentName = model.agentName(s.agent), place = place, time = time, divider = divider, nested = nested,
          snippet = snippet, terms = terms,
          // The sidebar marks the chat open beside it
          active = model.listInSidebar && model.path.firstOrNull() == Screen.Chat(s.id),
          // A tap while a row stands swiped open only puts it back
          onSelect = { if (!closeSwipe()) model.open(Screen.Chat(s.id)) },
          press = Modifier
            .semantics { onLongClick(t("session.more")) { menu = true; true } }
            .listDrag(
              drag, dragged, onLongPress = { if (!closeSwipe()) menu = true }, onLift = { menu = false }, onDrop = ::drop,
            ) { ListDragPreview(s.title) { VendorMark(s.agent, model.agentName(s.agent), Palette.fg2, size = Metrics.rowMark, fit = true) } },
        )
        SessionMenu(
          model, s.id, expanded = menu, onDismiss = { menu = false },
          onRename = { onRename(s.id) }, onAddFolder = { onAddFolder(s.id) },
        )
      }
    }
  }

  /** The by-project view, where rows drag (the other views have no project to drop on) */
  private val filing get() = byProject

  /** Puts a swiped row back; true when one stood open, so the tap that did it does nothing else */
  private fun closeSwipe(): Boolean {
    if (swiped == null) return false
    onSwiped(null)
    return true
  }

  /**
   * Where the held item would land over project [cwd] of [run] in [section]: a session moves into it (unless it is there
   * already); a project goes before it over the header's upper half, after it anywhere lower in its group
   */
  private fun dropTarget(item: ListDragItem, cwd: String, section: String?, run: List<String>, upper: Boolean): ListDropTarget? = when (item) {
    is ListDragItem.Session -> model.summaries.firstOrNull { it.id == item.id }?.takeIf { it.cwd != cwd }?.let { ListDropTarget.Project(cwd) }
    is ListDragItem.Project -> when {
      item.cwd == cwd -> null
      upper -> ListDropTarget.Slot(section, cwd)
      else -> run.filter { it != item.cwd }.let { others -> ListDropTarget.Slot(section, others.getOrNull(others.indexOf(cwd) + 1)) }
    }
  }

  /**
   * The edge of project [cwd]'s group a held project would land against: above it when it would go before it, below it
   * when it would end the run
   */
  private fun insertEdge(cwd: String, section: String?, run: List<String>): DropEdge? {
    val slot = drag.over as? ListDropTarget.Slot ?: return null
    val moving = (drag.held as? ListDragItem.Project)?.cwd ?: return null
    if (slot.section != section) return null
    if (slot.before == cwd) return DropEdge.Top
    return if (slot.before == null && run.lastOrNull { it != moving } == cwd) DropEdge.Bottom else null
  }

  /** A drop that checked out: the session moves (its toast says where), the project takes its place */
  private fun drop(item: ListDragItem, at: ListDropTarget) {
    when {
      item is ListDragItem.Session && at is ListDropTarget.Project -> lists.move(item.id, at.cwd)
      item is ListDragItem.Project && at is ListDropTarget.Slot -> lists.placeProject(item.cwd, at.section, at.before)
    }
  }

  companion object {
    /** Sessions an open project shows before "N more" (the desktop view menu's default) */
    const val PER_PROJECT = 5
    /** What one tap on "N more" brings out (SessionList.tsx MORE_PAGE) */
    const val MORE_PAGE = 10
    /** The chats' key among the projects' `moreShown` (a project's is its folder, never blank-led) */
    const val CHATS_KEY = " chats"
  }
}

private fun AppModel.agentName(id: String): String = agent(id).name

/** What the list says with nothing to show: still searching, no match, or none yet (and where the engine stands) */
@Composable
private fun EmptyNote(model: AppModel, searching: Boolean, answered: Boolean) {
  val note = when {
    // Waiting on the conversation search, the empty list says so rather than "no match"
    searching -> t(if (answered) "session.noMatch" else "session.searching")
    model.connection == Connection.Connecting -> t("connections.state.connecting")
    model.connection is Connection.Closed -> t("mobile.engine.disconnected")
    else -> t("session.none")
  }
  Box(Modifier.fillMaxWidth().heightIn(min = Metrics.sheetRow), contentAlignment = Alignment.Center) {
    Text(note, color = Palette.fg3, fontSize = Type.text2, modifier = Modifier.testTag("list.empty"))
  }
}

private fun parse(iso: String): Instant? = runCatching { Instant.parse(iso) }.getOrNull()

private fun recency(iso: String, now: Instant): String {
  val date = parse(iso) ?: return "older"
  val zone = ZoneId.systemDefault()
  val day = date.atZone(zone).toLocalDate()
  val today = now.atZone(zone).toLocalDate()
  if (day == today) return "today"
  if (day == today.minusDays(1)) return "yesterday"
  val days = (now.toEpochMilli() - date.toEpochMilli()) / 86_400_000.0
  return if (days < 7) "week" else if (days < 30) "month" else "older"
}

/** Today: the time of day; earlier: month/day, both in the UI locale */
fun sessionTime(iso: String, now: Instant): String {
  val date = parse(iso) ?: return ""
  val zone = ZoneId.systemDefault()
  val locale = Locale.forLanguageTag(L10n.locale)
  val local = date.atZone(zone)
  return if (local.toLocalDate() == now.atZone(zone).toLocalDate()) {
    local.format(DateTimeFormatter.ofLocalizedTime(FormatStyle.SHORT).withLocale(locale))
  } else {
    local.format(DateTimeFormatter.ofPattern("M/d", locale))
  }
}

/** [text] with a search's terms in the strong ink and weight (SessionList.tsx Marked) */
@Composable
private fun marked(text: String, terms: List<String>): AnnotatedString {
  if (terms.isEmpty()) return AnnotatedString(text)
  val strong = SpanStyle(color = Palette.fgStrong, fontWeight = FontWeight.Bold)
  return buildAnnotatedString {
    SessionSearch.parts(text, terms).forEachIndexed { i, part ->
      if (i % 2 == 1) withStyle(strong) { append(part) } else append(part)
    }
  }
}

/** The list's grouping filter at the bar's start: a menu ticking the grouping, where pins sit by project, archived, mark read */
@Composable
private fun ListViewMenu(
  view: SessionListView,
  onView: (SessionListView) -> Unit,
  pinPlace: PinPlace,
  onPinPlace: (PinPlace) -> Unit,
  archived: Boolean,
  onArchived: (Boolean) -> Unit,
  unread: Boolean,
  onMarkAllRead: () -> Unit,
) {
  var open by remember { mutableStateOf(false) }
  Box {
    IconButton(onClick = { open = true }, modifier = Modifier.testTag("list.view")) {
      Lucide("listFilter", Palette.fg1, size = Metrics.iconCtl, contentDescription = t("session.view.group"))
    }
    AppMenu(expanded = open, onDismiss = { open = false }) {
      // Titled sections of ticked choices: 列表顶部 / 所在项目内 keep the 置顶 that says what they place
      AppMenuCaption(t("session.view.group"))
      for ((value, icon) in VIEW_CHOICES) {
        AppMenuItem(
          t("mobile.view.${value.name}"), onClick = { onView(value); open = false }, checked = view == value,
          icon = { MenuGlyph(icon) }, modifier = Modifier.testTag("view.${value.name}"),
        )
      }
      if (view == SessionListView.project) {
        AppMenuGap()
        AppMenuCaption(t("session.view.pinned"))
        AppMenuItem(t("session.view.pinned.top"), onClick = { onPinPlace(PinPlace.top); open = false }, checked = pinPlace == PinPlace.top, icon = { MenuGlyph("pin") }, modifier = Modifier.testTag("view.pin.top"))
        AppMenuItem(t("session.view.pinned.project"), onClick = { onPinPlace(PinPlace.project); open = false }, checked = pinPlace == PinPlace.project, icon = { MenuGlyph("folder") }, modifier = Modifier.testTag("view.pin.project"))
      }
      AppMenuGap()
      AppMenuItem(
        t("session.view.status.archived"), onClick = { onArchived(!archived); open = false }, checked = archived,
        icon = { MenuGlyph("archive") }, modifier = Modifier.testTag("view.archived"),
      )
      if (unread) {
        AppMenuItem(
          t("session.view.markAllRead"), onClick = { onMarkAllRead(); open = false }, checked = false,
          icon = { MenuGlyph("mailCheck") }, modifier = Modifier.testTag("view.markAllRead"),
        )
      }
    }
  }
}

/** The groupings and their glyphs, in the menu's order */
private val VIEW_CHOICES = listOf(
  SessionListView.time to "clock",
  SessionListView.project to "folder",
  SessionListView.agent to "bot",
  SessionListView.none to "list",
)

/** A group's caption (置顶, 今天 …) */
@Composable
private fun Caption(text: String) {
  Box(Modifier.fillMaxWidth().heightIn(min = Metrics.groupCaption).padding(horizontal = Metrics.pad).padding(bottom = Metrics.s1), contentAlignment = Alignment.BottomStart) {
    Text(text, color = Palette.fg3, fontSize = Type.text3, lineHeight = Type.text3Line, fontWeight = FontWeight.Medium)
  }
}

/**
 * One session, full width, one fact per block: the agent's bare mark; the title, with the time at its right end where no
 * day caption says it, and the state's mark in a trailing slot on that line; under it the place (the project), and a
 * search's content hit. Inside a project's group the row is one line, a step in from the folder. [press] carries the
 * row's long press (its menu and drag)
 */
@Composable
private fun SessionRow(
  session: SessionSummary,
  agentName: String,
  place: String?,
  time: String?,
  divider: Boolean,
  nested: Boolean,
  snippet: String?,
  terms: List<String>,
  active: Boolean,
  onSelect: () -> Unit,
  press: Modifier,
) {
  val line = Palette.line
  val fill = Palette.active
  val density = LocalDensity.current
  val ruleStart = with(density) { (Metrics.pad + (if (nested) Metrics.nest else Metrics.s0) + Metrics.lead + Metrics.gap).toPx() }
  val hair = with(density) { Metrics.hairline.toPx() }
  Row(
    verticalAlignment = Alignment.CenterVertically,
    horizontalArrangement = Arrangement.spacedBy(Metrics.gap),
    modifier = Modifier
      .fillMaxWidth()
      // One line stands as tall as a project's header
      .heightIn(min = if (place == null && snippet == null) Metrics.sheetRow else Metrics.sessionRow)
      .background(Palette.bg0)
      // The sidebar's open chat: the desktop column's fill, rounded and held in from both edges (a square one ran into
      // the column's edge)
      .drawBehind {
        if (active) {
          val inset = Metrics.s2.toPx()
          val r = Metrics.rLg.toPx()
          drawRoundRect(fill, Offset(inset, 0f), androidx.compose.ui.geometry.Size(size.width - inset * 2, size.height), androidx.compose.ui.geometry.CornerRadius(r, r))
        }
      }
      .drawBehind { if (divider) drawRect(line, Offset(ruleStart, size.height - hair), androidx.compose.ui.geometry.Size(size.width - ruleStart, hair)) }
      .clickable(onClick = onSelect)
      .then(press)
      .padding(start = if (nested) Metrics.nest else Metrics.s0)
      .padding(horizontal = Metrics.pad, vertical = if (snippet == null) Metrics.s0 else Metrics.s2)
      .testTag("session:${session.id}"),
  ) {
    // The bare mark, its ink fitted; its column starts the slot, so the ink stands on the title's and captions' edge
    Box(Modifier.width(Metrics.lead), contentAlignment = Alignment.CenterStart) {
      VendorMark(session.agent, agentName, Palette.fg2, size = Metrics.rowMark, fit = true)
    }
    Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(Metrics.s0_5)) {
      Row(verticalAlignment = Alignment.CenterVertically) {
        Text(
          marked(session.title, terms), color = Palette.fgStrong, fontSize = Type.field, lineHeight = Type.fieldLine,
          maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f),
        )
        if (time != null) {
          Spacer(Modifier.width(Metrics.s2))
          Text(time, color = Palette.fg3, fontSize = Type.text3, style = TextStyle(fontFeatureSettings = "tnum"), maxLines = 1)
        }
        // The state is its mark alone, as on the desktop; every row keeps the slot, so the times above line up
        Box(Modifier.padding(start = Metrics.gap - Metrics.s2).size(Metrics.stateSlot), contentAlignment = Alignment.Center) {
          session.state?.takeIf { it != SessionState.unknown }?.let { StateMark(it, session.unread == true) }
        }
      }
      if (place != null) {
        Text(place, color = Palette.fg3, fontSize = Type.text3, lineHeight = Type.text3Line, maxLines = 1, overflow = TextOverflow.Ellipsis)
      }
      if (snippet != null) {
        Text(
          marked(snippet, terms), color = Palette.fg2, fontSize = Type.text3, lineHeight = Type.text3Line, maxLines = 2,
          overflow = TextOverflow.Ellipsis, modifier = Modifier.testTag("session.snippet:${session.id}"),
        )
      }
    }
  }
}

/**
 * A project's header in the by-project view (apps/ios ProjectHeader): the folder in the marks' column, the name and a
 * chevron turning down while open; folded, the project's most pressing state sits in the rows' state column. A folder on
 * an SSH host wears the globe and ends in its host's alias and connection dot. A tap folds it open or shut; its long
 * press opens its menu (a session in it, move to top, a section, copy path, off the list) and, moving on, drags it
 */
@Composable
private fun ProjectHeader(
  model: AppModel,
  cwd: String,
  first: Boolean,
  open: Boolean,
  sessions: List<SessionSummary>,
  modifier: Modifier,
  onToggle: () -> Unit,
  drag: ListDrag,
  onDrop: (ListDragItem, ListDropTarget) -> Unit,
) {
  val lists = model.lists
  var menu by remember { mutableStateOf(false) }
  val host = remoteAlias(cwd)
  val connection = host?.let { alias -> lists.connections.firstOrNull { it.alias == alias } }
  val turn by animateFloatAsState(if (open) CHEVRON_OPEN else 0f, tween(if (LocalMotion.current) MOTION_EASE_MS else 0), label = "chevron")
  Box(modifier) {
    Row(
      verticalAlignment = Alignment.CenterVertically,
      horizontalArrangement = Arrangement.spacedBy(Metrics.gap),
      modifier = Modifier
        .fillMaxWidth()
        .heightIn(min = Metrics.sheetRow)
        .clickable(onClick = onToggle)
        .semantics { heading(); onLongClick(t("session.more")) { menu = true; true } }
        .listDrag(drag, ListDragItem.Project(cwd), onLongPress = { menu = true }, onLift = { menu = false }, onDrop = onDrop) {
          ListDragPreview(projectName(cwd)) { if (host != null) RemoteFolderGlyph(false) else Lucide("folder", Palette.fg2) }
        }
        .padding(horizontal = Metrics.pad)
        .testTag("project:$cwd"),
    ) {
      // The folder in the top-level session marks' column (its own sessions sit a step in), shut or open with the fold
      Box(Modifier.width(Metrics.lead), contentAlignment = Alignment.CenterStart) {
        Box(Modifier.width(Metrics.rowMark), contentAlignment = Alignment.Center) {
          if (host != null) Breathing(connection?.state == ConnectionState.connecting) { RemoteFolderGlyph(open) }
          else Lucide(if (open) "folderOpen" else "folder", Palette.fg3)
        }
      }
      Row(Modifier.weight(1f), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(Metrics.s1_5)) {
        Text(
          projectName(cwd), color = Palette.fgStrong, fontSize = Type.field, lineHeight = Type.fieldLine, fontWeight = FontWeight.SemiBold,
          maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f, fill = false),
        )
        Lucide("chevronRight", Palette.fg3, Modifier.rotate(turn), size = Metrics.chevron)
      }
      // The alias keeps its room; the project's name truncates first, as on the desktop
      if (host != null) HostTag(host, connection)
      // Open, each row shows its own state; the slot stays either way, as in the rows
      Box(Modifier.size(Metrics.stateSlot), contentAlignment = Alignment.Center) {
        if (!open) pressing(sessions)?.let { (state, unread) -> StateMark(state, unread) }
      }
    }
    ProjectMenu(model, cwd, first, menu, onDismiss = { menu = false })
  }
}

/** The project header's menu (the desktop project menu, less revealing the folder on the computer) */
@Composable
private fun ProjectMenu(model: AppModel, cwd: String, first: Boolean, expanded: Boolean, onDismiss: () -> Unit) {
  val lists = model.lists
  // null: the top level; SECTIONS: the desktop's sections
  var level by remember { mutableStateOf<String?>(null) }
  val close = { onDismiss(); level = null }
  AppMenu(expanded = expanded, onDismiss = close) {
    val sections = lists.sections
    val current = sections.firstOrNull { cwd in it.projects }?.id
    when (level) {
      null -> {
        AppMenuItem(t("session.project.newSession"), onClick = { close(); model.open(Screen.New(cwd = cwd)) }, icon = { MenuGlyph("squarePen") }, modifier = Modifier.testTag("project.new"))
        AppMenuGap()
        if (!first) {
          AppMenuItem(t("mobile.project.moveTop"), onClick = { close(); lists.moveProjectToTop(cwd) }, icon = { MenuGlyph("arrowUp") }, modifier = Modifier.testTag("project.top"))
        }
        // Sections are made on the desktop; the entry shows once there is one
        if (sections.isNotEmpty()) {
          AppMenuItem(t("session.section.moveTo"), onClick = { level = SECTIONS }, icon = { MenuGlyph("chevronRight") }, modifier = Modifier.testTag("project.section"))
        }
        AppMenuItem(t("session.project.copyPath"), onClick = { close(); lists.copyPath(cwd) }, icon = { MenuGlyph("copy") }, modifier = Modifier.testTag("project.copyPath"))
        AppMenuGap()
        AppMenuItem(t("session.project.remove"), onClick = { close(); lists.removeProject(cwd) }, icon = { MenuGlyph("minus") }, modifier = Modifier.testTag("project.remove"))
      }
      else -> {
        // The sections as a ticked choice, the folder's own ticked, then "Remove from section" while it is in one
        AppMenuBack(t("session.section.moveTo"), onClick = { level = null })
        AppMenuGap()
        for (s in sections) {
          AppMenuItem(
            s.name.ifEmpty { t("session.section.untitled") }, checked = s.id == current,
            onClick = { close(); if (s.id != current) lists.setProjectSection(cwd, s.id) },
          )
        }
        if (current != null) {
          AppMenuGap()
          AppMenuItem(t("session.section.remove"), checked = false, onClick = { close(); lists.setProjectSection(cwd, null) }, icon = { MenuGlyph("minus") })
        }
      }
    }
  }
}

/**
 * The project's mark, ranked as the desktop's project rows rank it (SessionList.tsx STATE_RANK): running first, then
 * waiting, failed, unread, background work (lit when one of those sessions has a reply nobody has looked at)
 */
private fun pressing(sessions: List<SessionSummary>): Pair<SessionState, Boolean>? {
  val rank = listOf(SessionState.working, SessionState.waiting, SessionState.error, SessionState.unread, SessionState.background)
  val state = rank.firstOrNull { st -> sessions.any { it.state == st } } ?: return null
  return state to (state == SessionState.background && sessions.any { it.state == SessionState.background && it.unread == true })
}

/** RemoteFolderIcon.tsx: the folder with a small globe on its lower right corner, cut out of the folder under it */
@Composable
private fun RemoteFolderGlyph(open: Boolean) {
  Box(Modifier.size(Metrics.icon)) {
    Lucide(if (open) "folderOpen" else "folder", Palette.fg3)
    Box(
      Modifier
        .align(Alignment.BottomEnd)
        .offset(Metrics.remoteOverhang, Metrics.remoteOverhang)
        .size(Metrics.remoteBadge)
        .background(Palette.bg0, CircleShape),
      contentAlignment = Alignment.Center,
    ) { Lucide("globe", Palette.fg3, size = Metrics.remoteGlobe) }
  }
}

/** A host folder's header end (SessionList.tsx GroupHeader): the host's alias, then its connection as a dot */
@Composable
private fun HostTag(alias: String, view: ConnectionView?) {
  Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(Metrics.s1_5)) {
    Text(alias, color = Palette.fg3, fontSize = Type.text3, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.widthIn(max = Metrics.hostAlias))
    // No dot until the engine has said how the host is
    if (view != null) {
      val dot = when (view.state) {
        ConnectionState.connected -> Palette.ok
        ConnectionState.error -> Palette.warn
        ConnectionState.connecting -> Palette.fg3
        else -> Palette.fg3.copy(alpha = HOST_OFF_ALPHA)
      }
      Breathing(view.state == ConnectionState.connecting) { Box(Modifier.size(Metrics.hostDot).background(dot, CircleShape)) }
    }
  }
}

/** `remote-breathe`: [content] breathes while [on], unless motion is off */
@Composable
private fun Breathing(on: Boolean, content: @Composable () -> Unit) {
  if (!on || !LocalMotion.current) return content()
  val loop = rememberInfiniteTransition(label = "breathe")
  val level by loop.animateFloat(1f, BREATHE_LOW, infiniteRepeatable(tween(BREATHE_MS), RepeatMode.Reverse), label = "breatheLevel")
  Box(Modifier.alpha(level)) { content() }
}

/**
 * SessionList.tsx MoreRow, under an open project's latest few: how many the rest holds, the agents among them and the
 * most urgent state inside. A tap brings out the next page above it; with all of them out it reads "show less", the
 * chevron turned over, and a tap folds them away
 */
@Composable
private fun MoreRow(rest: List<SessionSummary>, nameOf: (String) -> String, tag: String, modifier: Modifier = Modifier, nested: Boolean = true, onClick: () -> Unit) {
  val all = rest.isEmpty()
  val agents = rest.map { it.agent }.distinct().take(MORE_MARKS)
  Row(
    verticalAlignment = Alignment.CenterVertically,
    horizontalArrangement = Arrangement.spacedBy(Metrics.gap),
    modifier = modifier
      .fillMaxWidth()
      .heightIn(min = Metrics.sheetRow)
      .clickable(onClick = onClick)
      .padding(start = if (nested) Metrics.nest else Metrics.s0)
      .padding(horizontal = Metrics.pad)
      .testTag(tag),
  ) {
    // The chevron in the sessions' mark column
    Box(Modifier.width(Metrics.lead), contentAlignment = Alignment.CenterStart) {
      Box(Modifier.width(Metrics.rowMark), contentAlignment = Alignment.Center) {
        Lucide("chevronDown", Palette.fg3, Modifier.rotate(if (all) CHEVRON_TURNED else 0f), size = Metrics.chevron)
      }
    }
    Text(
      if (all) t("session.showLess") else t("session.showMore", "n" to rest.size), color = Palette.fg3, fontSize = Type.text2, maxLines = 1,
    )
    if (!all) {
      Row(horizontalArrangement = Arrangement.spacedBy(Metrics.s1)) {
        for (agent in agents) VendorMark(agent, nameOf(agent), Palette.fg2, size = Metrics.moreMark, fit = true)
      }
    }
    Spacer(Modifier.weight(1f))
    Box(Modifier.size(Metrics.stateSlot), contentAlignment = Alignment.Center) {
      pressing(rest)?.let { (state, unread) -> StateMark(state, unread) }
    }
  }
}

/** The empty chats group's only row (SessionList.tsx EmptyProjectRow, flush): a faint plus in the marks' column and "New chat" */
@Composable
private fun NewChatRow(onClick: () -> Unit) {
  Row(
    verticalAlignment = Alignment.CenterVertically,
    horizontalArrangement = Arrangement.spacedBy(Metrics.gap),
    modifier = Modifier.fillMaxWidth().heightIn(min = Metrics.sheetRow).clickable(onClick = onClick).padding(horizontal = Metrics.pad).testTag("list.newChat"),
  ) {
    Box(Modifier.width(Metrics.lead), contentAlignment = Alignment.CenterStart) {
      Box(Modifier.width(Metrics.rowMark), contentAlignment = Alignment.Center) { Lucide("plus", Palette.fg3) }
    }
    Text(t("session.chat.new"), color = Palette.fg3, fontSize = Type.text2, maxLines = 1, modifier = Modifier.weight(1f))
  }
}

/** The end of a pinned run: the desktop's short faded rule (SessionList.tsx `PinDivider`) instead of a pin on each row */
@Composable
private fun PinRule() {
  val line = Palette.line
  Box(
    Modifier
      .fillMaxWidth()
      .padding(horizontal = Metrics.pad, vertical = Metrics.s1)
      .height(Metrics.hairline)
      .background(Brush.horizontalGradient(listOf(Color.Transparent, line, Color.Transparent))),
  )
}

/**
 * The desktop list's state marks (packages/ui chat/SessionList.tsx StateMark, motion in styles/motion.css), one slot
 * each so they share a centre line. Working: the spinning ring, which keeps turning with motion off. Waiting: an amber
 * dot beating like a heart. Error: a red dot that double-flashes every few seconds. Unread: a dot with a halo rippling
 * out of it. Background: the ring with a dot orbiting it slowly, lit with a dot inside once a reply came in meanwhile.
 * Motion off stops all but the ring on their resting frame
 */
@Composable
fun StateMark(state: SessionState, unread: Boolean) {
  // While the connection is down (or a new wire has not synced yet) a live state may be stale: the marks that move stop
  // on their resting frame and breathe instead, as the remote dot does (apps/ios StateMark `held`)
  val held = LocalSyncing.current && (state == SessionState.working || state == SessionState.waiting || state == SessionState.background)
  val motion = LocalMotion.current && !held
  val loop = rememberInfiniteTransition(label = "state")
  Breathing(held) {
  Box(Modifier.size(Metrics.stateSlot), contentAlignment = Alignment.Center) {
    when (state) {
      SessionState.working -> {
        // animate-spin: one turn a second, linear; it turns with motion off too, not while held
        val turn by loop.animateFloat(0f, FULL_TURN, infiniteRepeatable(tween(SPIN_MS, easing = LinearEasing)), label = "spin")
        Lucide("loaderCircle", Palette.fg2, Modifier.rotate(if (held) 0f else turn), size = Metrics.stateRing)
      }
      SessionState.waiting -> {
        // acp-mark-beat: 0 / 100% 1, 20% 1.5, 40% 1.1, 55% 1.35, 75% 1, eased between keys
        val beat by loop.animateFloat(
          1f, 1f,
          infiniteRepeatable(keyframes {
            durationMillis = BEAT_MS
            1f at 0 using FastOutSlowInEasing
            1.5f at (BEAT_MS * 0.2f).toInt() using FastOutSlowInEasing
            1.1f at (BEAT_MS * 0.4f).toInt() using FastOutSlowInEasing
            1.35f at (BEAT_MS * 0.55f).toInt() using FastOutSlowInEasing
            1f at (BEAT_MS * 0.75f).toInt()
          }),
          label = "beat",
        )
        Box(Modifier.size(Metrics.stateDot).scale(if (motion) beat else 1f).background(Palette.warn, CircleShape))
      }
      SessionState.error -> {
        // acp-mark-flash: lit, then two dips to 0.2 at 70% and 82%, linear between keys
        val flash by loop.animateFloat(
          1f, 1f,
          infiniteRepeatable(keyframes {
            durationMillis = FLASH_MS
            1f at (FLASH_MS * 0.64f).toInt()
            FLASH_LOW at (FLASH_MS * 0.7f).toInt()
            1f at (FLASH_MS * 0.76f).toInt()
            FLASH_LOW at (FLASH_MS * 0.82f).toInt()
            1f at (FLASH_MS * 0.88f).toInt()
          }),
          label = "flash",
        )
        Box(Modifier.size(Metrics.stateDot).alpha(if (motion) flash else 1f).background(Palette.danger, CircleShape))
      }
      SessionState.unread -> {
        // The halo: a copy of the dot growing to 2.6 times while it fades, ease-out over 2 s; invisible at rest
        if (motion) {
          val p by loop.animateFloat(0f, 1f, infiniteRepeatable(tween(HALO_MS, easing = { 1 - (1 - it) * (1 - it) })), label = "halo")
          Box(Modifier.size(Metrics.stateUnread).scale(1 + HALO_GROWTH * p).alpha(HALO_ALPHA * (1 - p)).background(Palette.fgStrong, CircleShape))
        }
        Box(Modifier.size(Metrics.stateUnread).background(Palette.fgStrong, CircleShape))
      }
      SessionState.background -> {
        val turn by loop.animateFloat(0f, FULL_TURN, infiniteRepeatable(tween(ORBIT_MS, easing = LinearEasing)), label = "orbit")
        val ring = if (unread) Palette.fgStrong else Palette.fg3
        Lucide("circle", ring, size = Metrics.stateRing)
        // The orbit rides the ring's own line (Lucide's circle: r 10 in a 24-unit box)
        Box(Modifier.size(Metrics.stateRing).rotate(if (motion) turn else 0f), contentAlignment = Alignment.TopCenter) {
          Box(Modifier.offset(y = orbitTop(Metrics.stateRing, Metrics.stateOrbit)).size(Metrics.stateOrbit).background(if (unread) Palette.fgStrong else Palette.fg2, CircleShape))
        }
        if (unread) Box(Modifier.size(Metrics.stateInner).background(Palette.fgStrong, CircleShape))
      }
      SessionState.unknown -> {}
    }
  }
  }
}

/** The screens show what may be stale: the wire is down, or a new one has not synced yet (AppModel `current`) */
val LocalSyncing = androidx.compose.runtime.compositionLocalOf { false }

/** Where the orbiting dot's top sits in the ring's box: centred on the circle's line, r 10 of a 24-unit box */
private fun orbitTop(ring: Dp, dot: Dp): Dp = ring / 2 - ring * ORBIT_R / LUCIDE_BOX - dot / 2

/** The search field and the new-session button the list keeps at the bottom */
@Composable
private fun ListBottomBar(query: String, onQuery: (String) -> Unit, onNew: () -> Unit) {
  // The page colour fades up under the bar, the gesture bar's strip included, so no row shows again below it; it stays
  // outside the handoff, which hides the bar's pieces only
  Box(Modifier.fillMaxWidth().background(footFade())) {
    Row(
      verticalAlignment = Alignment.CenterVertically,
      horizontalArrangement = Arrangement.spacedBy(Metrics.gap),
      // Handed over to a composer across a push or pop (ui/BarHandoff.kt)
      modifier = Modifier.handoffList(empty = query.isEmpty()).readableWidth().navigationBarsPadding().imePadding().padding(horizontal = Metrics.pad, vertical = Metrics.s2),
    ) {
      SearchPill(query, onQuery, Modifier.weight(1f).handoffPiece(HandoffPart.Search), face = Modifier.handoffFace(HandoffPart.Search))
      FloatingActionButton(
        onClick = onNew,
        containerColor = Palette.bg1, contentColor = Palette.fg1, shape = CircleShape,
        elevation = FloatingActionButtonDefaults.elevation(defaultElevation = Metrics.barShadow),
        modifier = Modifier.size(Metrics.barCtl).handoffPiece(HandoffPart.New).testTag("list.new"),
      ) {
        Lucide("squarePen", Palette.fg1, Modifier.handoffFace(HandoffPart.New), size = Metrics.iconCtl, contentDescription = t("session.new"))
      }
    }
  }
}

/**
 * The list's search field: a capsule with the glass bar's lift, the engine searching the conversations too. [face]
 * goes on its contents (the phone bar's handoff fades them apart from the capsule)
 */
@Composable
private fun SearchPill(query: String, onQuery: (String) -> Unit, modifier: Modifier = Modifier, face: Modifier = Modifier) {
  Surface(
    color = Palette.bg1, shape = RoundedCornerShape(percent = 50), shadowElevation = Metrics.barShadow,
    modifier = modifier.height(Metrics.barCtl),
  ) {
    Row(verticalAlignment = Alignment.CenterVertically, modifier = face.padding(horizontal = Metrics.pad)) {
      Lucide("search", Palette.fg1, size = Metrics.iconCtl)
      Spacer(Modifier.width(Metrics.s2 + Metrics.s1))
      Box(Modifier.weight(1f)) {
        if (query.isEmpty()) Text(t("session.searchContent"), color = Palette.fg3, fontSize = Type.text1, maxLines = 1, overflow = TextOverflow.Ellipsis)
        BasicTextField(
          value = query, onValueChange = onQuery, singleLine = true,
          textStyle = TextStyle(color = Palette.fg1, fontSize = Type.text1),
          cursorBrush = SolidColor(Palette.fg1),
          modifier = Modifier.fillMaxWidth().testTag("list.search"),
        )
      }
      if (query.isNotEmpty()) {
        Box(
          Modifier.size(Metrics.lead).clip(CircleShape).clickable { onQuery("") }.testTag("list.search.clear"),
          contentAlignment = Alignment.Center,
        ) { Lucide("x", Palette.fg3, size = Metrics.icon, contentDescription = t("panel.clear")) }
      }
    }
  }
}

/**
 * The sidebar's foot in a wide window, as the desktop's session column ends (apps/ios Sessions/SidebarFoot.swift):
 * settings, then the followed computer with its menu (switch computers, add one). On the phone both sit in the list's
 * top bar instead. The page colour fades up under it into the list
 */
@Composable
private fun SidebarFoot(model: AppModel, pairing: cc.hotic.acpira.store.PairingModel?) {
  Row(
    verticalAlignment = Alignment.CenterVertically,
    horizontalArrangement = Arrangement.spacedBy(Metrics.gap),
    modifier = Modifier.fillMaxWidth().background(footFade()).navigationBarsPadding()
      .padding(horizontal = Metrics.pad).padding(top = Metrics.s2, bottom = Metrics.s2),
  ) {
    FloatingActionButton(
      onClick = { model.push(Screen.Settings) },
      containerColor = Palette.bg1, contentColor = Palette.fg1, shape = CircleShape,
      elevation = FloatingActionButtonDefaults.elevation(defaultElevation = Metrics.barShadow),
      modifier = Modifier.size(Metrics.barCtl).testTag("list.settings"),
    ) { Lucide("settings", Palette.fg1, size = Metrics.iconCtl, contentDescription = t("session.settings")) }
    val current = pairing?.selected
    if (pairing != null && current != null) {
      var open by remember { mutableStateOf(false) }
      val online = pairing.link is cc.hotic.acpira.store.PairLink.Online
      Box(Modifier.weight(1f)) {
        Surface(
          color = Palette.bg1, shape = RoundedCornerShape(percent = 50), shadowElevation = Metrics.barShadow,
          modifier = Modifier.fillMaxWidth().height(Metrics.barCtl).clip(RoundedCornerShape(percent = 50))
            .clickable { open = true }.testTag("list.computer"),
        ) {
          Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(Metrics.s2),
            modifier = Modifier.padding(horizontal = Metrics.pad),
          ) {
            Lucide(computerIcon(current.model), Palette.fg2, size = Metrics.iconCtl)
            Text(
              current.name, color = Palette.fg1, fontSize = Type.text2, maxLines = 1, overflow = TextOverflow.Ellipsis,
              modifier = Modifier.weight(1f),
            )
            // Lit while its wire is up
            Box(Modifier.size(Metrics.hostDot).background(if (online) Palette.ok else Palette.fg3, CircleShape))
          }
        }
        ComputerMenu(pairing, open, onDismiss = { open = false })
      }
    }
  }
}

private const val SEARCH_DEBOUNCE_MS = 180L
/** The project menu's sections level */
private const val SECTIONS = "sections"
private const val MORE_MARKS = 3
private const val CHEVRON_OPEN = 90f
private const val CHEVRON_TURNED = 180f
private const val FULL_TURN = 360f
private const val MOTION_EASE_MS = 250
private const val HOST_OFF_ALPHA = 0.4f
private const val BREATHE_LOW = 0.35f
private const val BREATHE_MS = 1200
private const val SPIN_MS = 1000
private const val BEAT_MS = 1400
private const val FLASH_MS = 3000
private const val FLASH_LOW = 0.2f
private const val HALO_MS = 2000
private const val HALO_GROWTH = 1.6f
private const val HALO_ALPHA = 0.6f
private const val ORBIT_MS = 3600
private const val ORBIT_R = 10f
private const val LUCIDE_BOX = 24f
