package cc.hotic.acpira.ui

import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
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
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.graphics.SolidColor
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
import cc.hotic.acpira.model.SessionSection
import cc.hotic.acpira.model.SessionState
import cc.hotic.acpira.model.SessionSummary
import cc.hotic.acpira.store.AppModel
import cc.hotic.acpira.store.Connection
import cc.hotic.acpira.store.PinPlace
import cc.hotic.acpira.store.Screen
import cc.hotic.acpira.store.SessionListView
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
  val query = lists.query
  val archivedOnly = lists.archivedOnly
  // The row standing swiped open, Delete bared (SwipeToDelete.kt)
  var swiped by remember { mutableStateOf<String?>(null) }
  // The session the rename dialog and the move menu's "Add folder" are for
  var renaming by remember { mutableStateOf<String?>(null) }
  var movingTo by remember { mutableStateOf<String?>(null) }
  var sectionPrompt by remember { mutableStateOf<SectionPrompt?>(null) }
  val drag = remember { ListDrag() }
  // Restore the list's scroll offset after leaving for a chat or folding the sidebar; switching computers starts at the top
  val listState = key(model.computer) { rememberLazyListState(lists.scrollIndex, lists.scrollOffset) }
  val terms = searchTerms(query)
  // Paired remote access: the followed computer's state takes the page until it has sessions to list. The bar sits over
  // both, so the frame giving way to the list moves nothing in it
  val pairing = LocalPairing.current
  val covered = pairing != null && covers(pairing, model)
  // Listed folders alone keep the filter too: from another view its menu is the way back to them
  val filter = !covered && terms.isEmpty() && !(model.summaries.isEmpty() && lists.listedProjects.isEmpty() && lists.sections.isEmpty())
  // Standing in a wide window's sidebar (ui/Root.kt Split), and how to put it away there
  val inSidebar = model.listInSidebar
  val hide = LocalSidebarHide.current

  // The conversation search goes out once typing pauses (SessionList.tsx SEARCH_DEBOUNCE)
  LaunchedEffect(query) {
    if (lists.isSearched(query)) return@LaunchedEffect
    if (query.isNotEmpty()) delay(SEARCH_DEBOUNCE_MS)
    lists.search(query)
  }
  // Keep the latest scroll offset in SessionLists so returning from a chat lands back where the reader was
  LaunchedEffect(listState, lists, covered) {
    if (covered) return@LaunchedEffect
    snapshotFlow { listState.firstVisibleItemIndex to listState.firstVisibleItemScrollOffset }
      .collect { (index, offset) ->
        lists.scrollIndex = index
        lists.scrollOffset = offset
      }
  }
  DisposableEffect(listState, lists, covered) {
    onDispose {
      if (!covered) {
        lists.scrollIndex = listState.firstVisibleItemIndex
        lists.scrollOffset = listState.firstVisibleItemScrollOffset
      }
    }
  }
  // A scroll puts a swiped row back, as in the system's lists
  LaunchedEffect(listState.isScrollInProgress) { if (listState.isScrollInProgress) swiped = null }

  val listMenu = @Composable {
    ListViewMenu(
      view = lists.view, onView = lists::pickView,
      pinPlace = lists.pinPlace, onPinPlace = lists::pickPinPlace,
      archived = archivedOnly, onArchived = { lists.archivedOnly = it },
      unread = model.summaries.any { it.unread == true || it.state == SessionState.unread },
      onMarkAllRead = lists::markAllRead,
      onNewSection = { sectionPrompt = SectionPrompt.Create() },
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
          SearchPill(query, { lists.query = it })
        }
      }
      }
    },
    bottomBar = {
      if (!covered) {
        // In the sidebar the foot holds settings and the computer; new sessions come from the page beside it
        if (inSidebar) SidebarFoot(model, pairing)
        // A new session starts in the latest project, as the iOS list's button does
        else ListBottomBar(query, { lists.query = it }) { model.open(Screen.New(cwd = lists.projects.firstOrNull())) }
      }
    },
  ) { padding ->
    if (covered) {
      PairHeroList(pairing!!, Modifier.padding(padding))
      return@Scaffold
    }
    val scene = ListScene(
      model = model, drag = drag, terms = terms, archivedOnly = archivedOnly, swiped = swiped, moreShown = lists.moreShown,
      onSwiped = { swiped = it }, onMore = lists::setMore,
      onRename = { renaming = it }, onAddFolder = { movingTo = it }, onSectionPrompt = { sectionPrompt = it },
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
  sectionPrompt?.let { prompt ->
    when (prompt) {
      is SectionPrompt.Create -> SectionPromptDialog(
        dialogTitle = t("session.section.new"),
        initial = "",
        allowBlank = true,
        onDismiss = { sectionPrompt = null },
        onSave = { lists.createSection(it, prompt.project) },
      )
      is SectionPrompt.Rename -> SectionPromptDialog(
        dialogTitle = t("common.rename"),
        initial = prompt.section.name,
        allowBlank = false,
        onDismiss = { sectionPrompt = null },
        onSave = { lists.renameSection(prompt.section.id, it) },
      )
    }
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
  val onSectionPrompt: (SectionPrompt) -> Unit,
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
    if (ordered.isEmpty() && !(byProject && (listedProjects.isNotEmpty() || lists.sections.isNotEmpty() || model.chatRoot != null))) {
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
   * The by-project view's projects: the user's sections first, each under its name with its projects in its own
   * order, then the projects in none under "Projects"; without sections, the projects alone
   */
  private fun LazyListScope.projectGroups() {
    val runs = lists.sections.map { it to it.projects.filter(listedProjects::contains) }
    if (runs.isEmpty()) {
      for (cwd in listedProjects) projectGroup(cwd, null, listedProjects)
      return
    }
    val filed = runs.flatMap { it.second }.toSet()
    val rest = listedProjects.filter { it !in filed }
    runs.forEachIndexed { index, (section, projects) ->
      val open = section.collapsed != true
      // A project dropped on a section's caption joins the end of that section
      item(key = "section:${section.id}") {
        SectionHeader(
          section = section,
          open = open,
          canMoveUp = index > 0,
          canMoveDown = index < runs.lastIndex,
          onToggle = { if (!closeSwipe()) lists.collapseSection(section.id, open) },
          onRename = { onSectionPrompt(SectionPrompt.Rename(section)) },
          onMoveUp = { lists.reorderSection(section.id, runs[index - 1].first.id) },
          onMoveDown = { lists.reorderSection(section.id, runs.getOrNull(index + 2)?.first?.id) },
          onDelete = { lists.deleteSection(section.id) },
        )
      }
      if (open) {
        if (projects.isEmpty()) {
          item(key = "section-empty:${section.id}") { SectionEmptyRow(section.id) }
        } else {
          for (cwd in projects) projectGroup(cwd, section.id, projects)
        }
      }
    }
    if (rest.isNotEmpty()) {
      item(key = "section:") { SectionCaption(t("session.group.projects"), null) }
      for (cwd in rest) projectGroup(cwd, null, rest)
    }
  }

  @Composable
  private fun SectionEmptyRow(section: String) {
    Box(
      Modifier
        .fillMaxWidth()
        .heightIn(min = Metrics.sheetRow)
        .dropZone(drag, "section-empty:$section") { item, _, _ -> if (item is ListDragItem.Project) ListDropTarget.Slot(section, null) else null }
        .padding(horizontal = Metrics.pad),
      contentAlignment = Alignment.CenterStart,
    ) {
      Text(t("session.section.empty"), color = Palette.fg3, fontSize = Type.text3, lineHeight = Type.text3Line)
    }
  }

  @Composable
  private fun SectionHeader(
    section: SessionSection,
    open: Boolean,
    canMoveUp: Boolean,
    canMoveDown: Boolean,
    onToggle: () -> Unit,
    onRename: () -> Unit,
    onMoveUp: () -> Unit,
    onMoveDown: () -> Unit,
    onDelete: () -> Unit,
  ) {
    var menu by remember { mutableStateOf(false) }
    val turn by animateFloatAsState(if (open) CHEVRON_OPEN else 0f, tween(if (LocalMotion.current) MOTION_EASE_MS else 0), label = "secChevron")
    Box(Modifier.dropZone(drag, "caption:${section.id}") { item, _, _ -> if (item is ListDragItem.Project) ListDropTarget.Slot(section.id, null) else null }) {
      Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(Metrics.s1_5),
        modifier = Modifier
          .fillMaxWidth()
          .heightIn(min = Metrics.groupCaption)
          .combinedClickable(onClick = onToggle, onLongClick = { menu = true })
          .semantics { heading(); onLongClick(t("session.section.more")) { menu = true; true } }
          .padding(horizontal = Metrics.pad)
          .padding(bottom = Metrics.s1)
          .testTag("section:${section.id}"),
      ) {
        Text(
          section.name.ifEmpty { t("session.section.untitled") },
          color = Palette.fg3, fontSize = Type.text3, lineHeight = Type.text3Line, fontWeight = FontWeight.Medium,
          maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f, fill = false),
        )
        Lucide("chevronRight", Palette.fg3, Modifier.rotate(turn), size = Metrics.chevron)
      }
      AppMenu(expanded = menu, onDismiss = { menu = false }) {
        AppMenuItem(t("common.rename"), onClick = { menu = false; onRename() }, icon = { MenuGlyph("pencil") }, modifier = Modifier.testTag("section.rename"))
        if (canMoveUp) {
          AppMenuItem(t("session.section.moveUp"), onClick = { menu = false; onMoveUp() }, icon = { MenuGlyph("arrowUp") }, modifier = Modifier.testTag("section.up"))
        }
        if (canMoveDown) {
          AppMenuItem(t("session.section.moveDown"), onClick = { menu = false; onMoveDown() }, icon = { MenuGlyph("arrowDown") }, modifier = Modifier.testTag("section.down"))
        }
        AppMenuGap()
        AppMenuItem(
          t("session.section.delete"), onClick = { menu = false; onDelete() }, danger = true,
          icon = { MenuGlyph("trash2", danger = true) }, modifier = Modifier.testTag("section.delete"),
        )
      }
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
        onNewSection = { onSectionPrompt(SectionPrompt.Create(cwd)) },
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

internal fun AppModel.agentName(id: String): String = agent(id).name

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
internal fun marked(text: String, terms: List<String>): AnnotatedString {
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
  onNewSection: (() -> Unit)? = null,
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
        if (!archived && onNewSection != null) {
          AppMenuGap()
          AppMenuItem(t("session.section.newEllipsis"), onClick = { open = false; onNewSection() }, icon = { MenuGlyph("plus") }, modifier = Modifier.testTag("view.newSection"))
        }
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
internal const val CHEVRON_OPEN = 90f
internal const val MOTION_EASE_MS = 250
