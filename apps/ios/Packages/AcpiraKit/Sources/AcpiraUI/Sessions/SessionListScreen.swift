import AcpiraDesign
import AcpiraFixtures
import AcpiraModels
import SwiftUI

// The phone's first screen: every session, grouped as picked in the bar's filter menu (as Trae's phone app switches its list
// between a timeline and groups, rather than stacking projects over a timeline; the groupings are the desktop view
// menu's). By time: the sessions pinned to the top, then day captions. By project: the same pinned run (or, as the
// view menu says, each project's pins leading it), then the desktop app's sections, each project as a header folding
// open in place, the latest few, the rest behind "N more" (the desktop list's project grouping and its view menu's
// defaults; a page per project put back and its title on two rows). By channel: pinned, then a caption per agent.
// Ungrouped: one run. Projects follow the user's order, shared with the desktop. A search lists one run across every project,
// matching the conversations too (a content hit shows the text around its match under the title). Every row keeps one
// fact per block: the title with its time and state on one line, and the project under it. Inside a project's group
// the row is that one line, its mark already naming the agent, and sits a step in from the folder (`Metrics.nest`), as
// on the desktop app: flush, the sessions read as more folders.

/// How the list groups its sessions (`Prefs.listView`), the desktop view menu's Grouping: by date, by project (with the
/// desktop app's sections), by channel (the agent), or one ungrouped run
enum SessionListView: String, CaseIterable { case time, project, agent, none }

/// Where pinned sessions sit by project (`Prefs.pinPlace`, the desktop view menu's ViewPin): one run over every project,
/// or first inside their own project
enum PinPlace: String, CaseIterable { case top, project }

struct SessionListScreen: View {
  @Environment(AppModel.self) private var model
  @Environment(\.pairing) private var pairing
  @AppStorage(Prefs.listView) private var view = SessionListView.time
  @AppStorage(Prefs.pinPlace) private var pinPlace = PinPlace.top
  /// `-search TEXT` (the launch arguments' defaults domain) starts with a search typed, for screenshots of its hits
  @State private var query = UserDefaults.standard.string(forKey: "search") ?? ""
  @State private var renaming: SessionSummary?
  @State private var newTitle = ""
  /// The session whose move menu asked for a folder typed in
  @State private var movingTo: SessionSummary?
  /// The row standing swiped open, Delete bared (SwipeToDelete.swift)
  @State private var swiped: String?
  /// Per open project, how many sessions past its latest few are out (SessionList.tsx `more`): "N more" adds a page,
  /// and once all are out it folds them away again
  @State private var moreShown: [String: Int] = [:]
  /// The view menu's "archived": the list shows the archived sessions, and only those
  @State private var archivedOnly = false
  /// The row a drag lifted, and where it would land now (ListDrag.swift)
  @State private var held: ListDragItem?
  @State private var over: ListDropTarget?
  @Environment(\.now) private var now
  /// In a regular width's sidebar: a pick replaces the chat beside the list rather than pushing over it
  @Environment(\.listInSidebar) private var inSidebar

  /// Sessions an open project shows before "N more" (the desktop view menu's default)
  private static let perProject = 5
  /// What one tap on "N more" brings out (SessionList.tsx MORE_PAGE): a project of hundreds never lays them all out at once
  private static let morePage = 10

  var body: some View {
    // Paired remote access: the followed computer's state takes the page until it has sessions to list. The bar sits
    // over both, so the hero giving way to the list moves nothing in it
    let covered = pairing.map { PairHeroList.covers($0, model) } ?? false
    // Listed folders alone keep the filter too: from another view its menu is the way back to them
    let filter = !covered && searchTerms(query).isEmpty && !(model.summaries.isEmpty && model.listedProjects.isEmpty)
    PairListGate(pairing: pairing) { list }
      .modifier(ListTopBar(filter: filter ? ListViewMenu(view: $view, pinPlace: $pinPlace,
                                                         archived: $archivedOnly,
                                                         unread: model.summaries.contains { $0.unread == true || $0.state == .unread },
                                                         onMarkAllRead: model.markAllRead) : nil,
                           onSettings: { model.settingsOpen = true }))
  }

  /// Opens a page from the list: pushed over it on the phone, beside it in place of the open one in the sidebar
  private func show(_ route: Route) {
    if inSidebar { model.path = [route] } else { model.path.append(route) }
  }

  @ViewBuilder private var list: some View {
    let terms = searchTerms(query)
    let searching = !terms.isEmpty
    // Content hits count for the query they answered only
    let snippets = model.hits?.query == terms.joined(separator: " ") ? model.hits?.snippets : nil
    let known = Set(model.summaries.map(\.id))
    let extra = snippets == nil ? [] : model.hits?.summaries.filter { !known.contains($0.id) && ($0.archived == true) == archivedOnly } ?? []
    let pool = listed + extra
    let shown = searching ? filtered(pool, terms: terms, snippets: snippets) : pool
    // Pinned first, then the latest
    let ordered = shown.sorted { ($0.pinned == true ? 1 : 0, $0.updatedAt) > ($1.pinned == true ? 1 : 0, $1.updatedAt) }
    // The archived sessions read as one flat list, newest first
    let byProject = view == .project && !searching && !archivedOnly
    let pinsInProject = byProject && pinPlace == .project
    let groups = groups(ordered, searching: searching, byProject: byProject, pinsInProject: pinsInProject)
    // The folders the rows' menus move a session to, sorted once per render rather than in every row
    let projects = model.projects
    let onNew = { show(.new(agent: nil, cwd: projects.first ?? "")) }
    ScrollView(.vertical) {
      LazyVStack(alignment: .leading, spacing: 0) {
        // By project, the chats' group still stands (its "new chat" row) when nothing else does
        if ordered.isEmpty && !(byProject && (!model.listedProjects.isEmpty || model.chatRoot != nil)) {
          // Waiting on the conversation search, the empty list says so rather than "no match"
          // The whole list is empty, not one project's ("session.noneWorkspace" is the IDE's workspace-scoped list)
          Text(web: searching ? t(snippets == nil ? "session.searching" : "session.noMatch") : t("session.none"))
            .textStyle(TypeScale.text2).foregroundStyle(Palette.fg3)
            .frame(maxWidth: .infinity, minHeight: Metrics.sheetRow)
        }
        ForEach(groups) { group($0, terms: terms, snippets: snippets, projects: projects) }
        if byProject {
          projectGroups(pinsInProject: pinsInProject, projects: projects)
          chatGroup(pinsInProject: pinsInProject, projects: projects)
        }
      }
      .padding(.bottom, Metrics.msgGap)
      .animation(Motion.ease, value: ordered.map(\.id))
      .animation(Motion.ease, value: model.listedProjects)
      .animation(Motion.ease, value: model.sections)
      .animation(Motion.ease, value: view)
    }
    .scrollIndicators(.hidden)
    // A scroll puts a swiped row back, as in the system's lists
    .onScrollPhaseChange { _, phase in
      if phase == .interacting && swiped != nil { swiped = nil }
    }
    // Under the keyboard too, behind its rounded corners while searching (ChatScreen)
    .background(Palette.bg0.ignoresSafeArea())
    .modifier(SystemListBar(query: $query, onNew: onNew))
    // Rows fade into the page as they scroll up under the bar's glass buttons
    .topFade()
    .alert(t("common.rename"), isPresented: Binding(get: { renaming != nil }, set: { if !$0 { renaming = nil } })) {
      TextField(text: $newTitle) { Text(web: t("session.titleAria")) }
      Button(t("mobile.cancel"), role: .cancel) {}
      Button(t("mobile.save")) { if let s = renaming { model.rename(s.id, to: newTitle) } }
    }
    // The move menu's "Add folder", as in the new session's folder menu: the folder joins the list, the session moves in
    .modifier(AddProjectPicker(isPresented: Binding(get: { movingTo != nil }, set: { if !$0 { movingTo = nil } })) { folder in
      guard let s = movingTo else { return }
      model.addProject(folder)
      model.move(s.id, to: folder)
    })
    // The conversation search goes out once typing pauses (SessionList.tsx SEARCH_DEBOUNCE)
    .task(id: query) {
      if !query.isEmpty { try? await Task.sleep(for: .milliseconds(180)) }
      guard !Task.isCancelled else { return }
      model.search(query)
    }
  }

  @ViewBuilder
  private func group(_ group: SessionGroup, terms: [String], snippets: [String: String]?, projects: [String]) -> some View {
    if let caption = group.caption { Caption(text: caption) }
    ForEach(Array(group.sessions.enumerated()), id: \.element.id) { i, s in
      // A caption already says "pinned"; one run without captions (a search) ends its leading pins at the pin rule
      let pinEnd = group.caption == nil && s.pinned == true && group.sessions[safe: i + 1].map { $0.pinned != true } == true
      row(s, place: model.placeName(s.cwd),
          // A day caption already says when; pinned rows and search hits have none, so they keep the time
          time: group.dated ? nil : sessionTime(s.updatedAt, now: now),
          // Captions part the runs and rows sit on space alone; one run without captions parts by hairlines
          divider: i < group.sessions.count - 1 && group.caption == nil && !pinEnd,
          snippet: snippets?[s.id], terms: terms, projects: projects)
      if pinEnd { PinRule() }
    }
  }

  /// The by-project view's projects: the desktop app's sections first, each under its name with its projects in its own
  /// order, then the projects in none under "Projects" (the desktop list's layout); without sections, the projects
  /// alone. Sections are made and filled on the desktop; one with nothing listed here is left out
  @ViewBuilder
  private func projectGroups(pinsInProject: Bool, projects: [String]) -> some View {
    let listed = model.listedProjects
    let runs = model.sections.map { SectionRun(section: $0, projects: $0.projects.filter(listed.contains)) }.filter { !$0.projects.isEmpty }
    if runs.isEmpty {
      ForEach(listed, id: \.self) { projectGroup($0, section: nil, run: listed, pinsInProject: pinsInProject, projects: projects) }
    } else {
      let filed = Set(runs.flatMap(\.projects))
      let rest = listed.filter { !filed.contains($0) }
      ForEach(runs) { run in
        // A project dropped on a section's caption joins the end of that section
        Caption(text: run.section.name.isEmpty ? t("session.section.untitled") : run.section.name)
          .modifier(captionZone(section: run.section.id))
        ForEach(run.projects, id: \.self) {
          projectGroup($0, section: run.section.id, run: run.projects, pinsInProject: pinsInProject, projects: projects)
        }
      }
      if !rest.isEmpty {
        Caption(text: t("session.group.projects")).modifier(captionZone(section: nil))
        ForEach(rest, id: \.self) { projectGroup($0, section: nil, run: rest, pinsInProject: pinsInProject, projects: projects) }
      }
    }
  }

  /// A project in the by-project view: its header, then, open, its pins when they sit in their project (ending at the
  /// pin rule) and its other sessions, the latest few only until "N more" is tapped. `run` is the projects listed with
  /// it in `section` (nil: those in none), where a dragged project lands. The header drags; every row of the group takes
  /// a dragged session into the project and a dragged project after it
  @ViewBuilder
  private func projectGroup(_ cwd: String, section: String?, run: [String], pinsInProject: Bool, projects: [String]) -> some View {
    let all = listed.filter { $0.cwd == cwd }
    let pins = pinsInProject ? all.filter { $0.pinned == true }.sorted { $0.updatedAt > $1.updatedAt } : []
    let sessions = all.filter { $0.pinned != true }.sorted { $0.updatedAt > $1.updatedAt }
    let open = model.openProjects.contains(cwd)
    // The latest few (all of them when only one more would hide), plus the desktop's live / attention exceptions.
    let cut = sessions.count <= Self.perProject + 1 ? sessions.count : Self.perProject
    let extra = moreShown[cwd] ?? 0
    let page = SessionListRows(sessions, cut: cut, extra: extra, activeId: activeId)
    let rows = open ? page.rows : []
    let rest = page.rest
    // The "N more" row stays once paging began, reading "show less" with everything out.
    let paged = open && page.canFold
    // A project landing after this one draws its line under the group's last row, not between the header and its rows
    let insert = insertEdge(cwd, section: section, run: run)
    let lastRow = open ? paged ? nil : (rows.last ?? pins.last)?.id : nil
    let tail = insert == .bottom ? VerticalEdge.bottom : nil
    let zone: (Bool) -> ListDropZone = { header in ListDropZone(held: held, over: $over, perform: drop) { item, y, height in
      dropTarget(item, over: cwd, section: section, run: run, upper: header && y < height / 2)
    } }
    // A group rather than a stack: the lazy list then builds a project's rows as they scroll in, not all of them on
    // any change (a project of hundreds hitched on every tap)
    Group {
      ProjectHeader(cwd: cwd, first: model.listedProjects.first == cwd, open: open, sessions: all,
                    host: model.host(of: cwd), sections: model.sections,
                    onToggle: { if !closeSwipe() { withAnimation(Motion.ease) { model.toggleOpen(cwd) } } },
                    onMoveTop: { withAnimation(Motion.ease) { model.moveProjectToTop(cwd) } },
                    onRemove: { withAnimation(Motion.ease) { model.removeProject(cwd) } },
                    onNew: { show(.new(agent: nil, cwd: cwd)) },
                    onSection: { section in withAnimation(Motion.ease) { model.setProjectSection(cwd, section) } },
                    onCopyPath: { model.copyPath(cwd) })
      .listDrag(filing ? .project(cwd) : nil, onStart: { held = $0 }) {
        ListDragPreview(title: projectName(cwd)) {
          if model.host(of: cwd) != nil { RemoteFolderGlyph(open: false) } else { Icon(.folder) }
        }
      }
      .modifier(zone(true))
      .dropMarks(hot: over == .project(cwd), insert: insert == .top || lastRow == nil && !paged ? insert : nil)
      if open {
        // Inside its project a row is one line, the mark naming the agent; no caption says when, so every row keeps
        // its time
        ForEach(pins, id: \.id) { s in
          row(s, place: nil, time: sessionTime(s.updatedAt, now: now), divider: false, nested: true, projects: projects)
            .modifier(zone(false))
            .dropMarks(hot: false, insert: s.id == lastRow ? tail : nil)
        }
        if !pins.isEmpty && (!rows.isEmpty || paged) { PinRule() }
      }
      ForEach(rows, id: \.id) { s in
        row(s, place: nil, time: sessionTime(s.updatedAt, now: now), divider: false, nested: true, projects: projects)
          .modifier(zone(false))
          .dropMarks(hot: false, insert: s.id == lastRow ? tail : nil)
      }
      if paged {
        MoreRow(rest: rest, nameOf: { model.agent($0).name }) {
          if closeSwipe() { return }
          withAnimation(Motion.ease) { moreShown[cwd] = rest.isEmpty ? nil : extra + Self.morePage }
        }
        .modifier(zone(false))
        .dropMarks(hot: false, insert: tail)
      }
    }
  }

  /// The chats (sessions in no project, AppModel.isChat) under the projects, as the desktop list puts them: a caption,
  /// then the rows at the top level's inset (one line, the caption already says where), the latest few until "N more";
  /// with none yet, a row starting one. Shown where the engine keeps chats
  @ViewBuilder
  private func chatGroup(pinsInProject: Bool, projects: [String]) -> some View {
    if model.chatRoot != nil {
      let all = listed.filter { model.isChat($0.cwd) }
      let pins = pinsInProject ? all.filter { $0.pinned == true }.sorted { $0.updatedAt > $1.updatedAt } : []
      // Pinned to the top, a chat already sits in the pinned run
      let sessions = all.filter { $0.pinned != true }.sorted { $0.updatedAt > $1.updatedAt }
      let cut = sessions.count <= Self.perProject + 1 ? sessions.count : Self.perProject
      let extra = moreShown[Self.chatsKey] ?? 0
      let page = SessionListRows(sessions, cut: cut, extra: extra, activeId: activeId)
      let rows = page.rows
      let rest = page.rest
      Group {
        Caption(text: t("session.group.chats"))
        ForEach(pins, id: \.id) { s in
          row(s, place: nil, time: sessionTime(s.updatedAt, now: now), divider: false, projects: projects)
        }
        if !pins.isEmpty && !rows.isEmpty { PinRule() }
        ForEach(rows, id: \.id) { s in
          row(s, place: nil, time: sessionTime(s.updatedAt, now: now), divider: false, projects: projects)
        }
        if page.canFold {
          MoreRow(rest: rest, nameOf: { model.agent($0).name }, nested: false) {
            if closeSwipe() { return }
            withAnimation(Motion.ease) { moreShown[Self.chatsKey] = rest.isEmpty ? nil : extra + Self.morePage }
          }
        }
        if all.isEmpty {
          NewChatRow { if !closeSwipe() { show(.new(agent: nil, cwd: "")) } }
        }
      }
    }
  }

  /// The selected chat stays outside paging even when it is older and idle (the regular-width sidebar).
  private var activeId: String? {
    guard case .chat(let id) = model.path.first else { return nil }
    return id
  }

  /// The chats' key among the projects' `moreShown` (a project's is its folder, never blank-led)
  private static let chatsKey = " chats"

  /// One session with its menu: the chat's "…" menu without the chat's own pages, as the desktop's row menu is its
  /// header's (pin, rename, copy, move, export). Delete is the row's swipe, as in the system's lists
  private func row(_ s: SessionSummary, place: String?, time: String?, divider: Bool, nested: Bool = false,
                   snippet: String? = nil, terms: [String] = [], projects: [String]) -> some View {
    SessionRow(session: s, agentName: model.agent(s.agent).name, place: place, time: time, divider: divider,
               nested: nested, snippet: snippet, terms: terms,
               // The sidebar marks the chat open beside it
               active: inSidebar && model.path.first == .chat(s.id),
               // A tap while a row stands swiped open only puts it back
               onSelect: { if !closeSwipe() { show(.chat(s.id)) } }) {
      NativeSessionMenu(pinned: s.pinned == true, unread: isUnread(s), archived: s.archived == true, actions: HeaderActions(
        pin: { model.togglePin(s.id) },
        unread: { model.setUnread(s.id, $0) },
        archive: { archived in withAnimation(Motion.ease) { model.setArchived(s.id, archived) } },
        rename: { newTitle = s.title; renaming = s },
        export: { model.export(s.id, format: $0) },
        exportJSON: model.holdsView(s.id),
        copy: { kind in Task { await model.copy(s.id, kind) } },
        cwd: s.cwd, projects: projects, chat: model.isChat(s.cwd),
        move: { model.move(s.id, to: $0) },
        addFolder: { movingTo = s }))
    }
    // Deleted at once, as on the desktop: the toast's undo brings it back
    .modifier(SwipeToDelete(id: s.id, open: $swiped) { withAnimation(Motion.ease) { model.delete(s.id) } })
    // By project a session drags onto another project; pinned ones stay put, as on the desktop (sessionTree.ts draggable)
    .listDrag(filing && s.pinned != true && s.external != true ? .session(s.id) : nil, onStart: { held = $0 }) {
      ListDragPreview(title: s.title) { VendorMark(brand: s.agent, name: model.agent(s.agent).name, size: Metrics.rowMark, fit: true) }
    }
  }

  /// The by-project view, where rows drag (the other views have no project to drop on)
  private var filing: Bool { view == .project && searchTerms(query).isEmpty && !archivedOnly }

  /// Where the held item would land over project `cwd` of `run` in `section`: a session moves into it (unless it is
  /// there already); a project goes before it over the header's upper half, after it anywhere lower in its group
  private func dropTarget(_ item: ListDragItem, over cwd: String, section: String?, run: [String], upper: Bool) -> ListDropTarget? {
    switch item {
    case .session(let id):
      guard let s = model.summaries.first(where: { $0.id == id }), s.cwd != cwd else { return nil }
      return .project(cwd)
    case .project(let moving):
      guard moving != cwd else { return nil }
      if upper { return .slot(section: section, before: cwd) }
      let others = run.filter { $0 != moving }
      return .slot(section: section, before: others.firstIndex(of: cwd).flatMap { others[safe: $0 + 1] })
    }
  }

  /// A section caption's zone: a project dropped there goes to the end of that section (nil: the projects in none)
  private func captionZone(section: String?) -> ListDropZone {
    ListDropZone(held: held, over: $over, perform: drop) { item, _, _ in
      guard case .project = item else { return nil }
      return .slot(section: section, before: nil)
    }
  }

  /// The edge of project `cwd`'s group a held project would land against: above it when it would go before it, below
  /// it when it would end the run
  private func insertEdge(_ cwd: String, section: String?, run: [String]) -> VerticalEdge? {
    guard case let .slot(at, before)? = over, at == section, case let .project(moving)? = held else { return nil }
    if before == cwd { return .top }
    return before == nil && run.last(where: { $0 != moving }) == cwd ? .bottom : nil
  }

  /// A drop that checked out: the session moves (its toast says where), the project takes its place
  private func drop(_ item: ListDragItem, at: ListDropTarget) {
    held = nil
    switch (item, at) {
    case let (.session(id), .project(cwd)):
      model.move(id, to: cwd)
    case let (.project(cwd), .slot(section, before)):
      withAnimation(Motion.ease) { model.placeProject(cwd, section: section, before: before) }
    default:
      break
    }
  }

  /// The sessions the list draws from: archived ones only while the view menu asks for them
  private var listed: [SessionSummary] { model.summaries.filter { ($0.archived == true) == archivedOnly } }

  /// Puts a swiped row back; true when one stood open, so the tap that did it does nothing else
  private func closeSwipe() -> Bool {
    guard swiped != nil else { return false }
    swiped = nil
    return true
  }

  /// By time: pinned, then today / yesterday / this week / this month / older, captions over each. By project: the
  /// pinned run alone (none when pins sit in their projects), the projects follow. By channel: pinned, then a run per
  /// agent under its name, the agent of the latest session first (sessionView.ts groupSessions). Ungrouped, or a
  /// search: one run, its pins first
  private func groups(_ sessions: [SessionSummary], searching: Bool, byProject: Bool, pinsInProject: Bool) -> [SessionGroup] {
    if searching || view == .none {
      return [SessionGroup(id: "all", caption: nil, sessions: sessions)]
    }
    let top = pinsInProject ? [] : sessions.filter { $0.pinned == true }
    var out: [SessionGroup] = top.isEmpty ? [] : [SessionGroup(id: "pinned", caption: t("session.group.pinned"), sessions: top)]
    guard !byProject else { return out }
    let rest = sessions.filter { $0.pinned != true }
    if view == .agent {
      var agents: [AgentId] = []
      for s in rest where !agents.contains(s.agent) { agents.append(s.agent) }
      return out + agents.map { a in
        SessionGroup(id: "agent.\(a)", caption: model.agent(a).name, sessions: rest.filter { $0.agent == a })
      }
    }
    // Each session's bucket is worked out once, not once per bucket
    var buckets: [String: [SessionSummary]] = [:]
    for s in rest { buckets[recency(s.updatedAt), default: []].append(s) }
    for key in ["today", "yesterday", "week", "month", "older"] {
      if let members = buckets[key] {
        out.append(SessionGroup(id: key, caption: t("session.when.\(key)"), sessions: members, dated: true))
      }
    }
    return out
  }

  private func recency(_ iso: String) -> String {
    guard let date = SessionDate.parse(iso) else { return "older" }
    let calendar = Calendar.current
    if calendar.isDate(date, inSameDayAs: now) { return "today" }
    if let y = calendar.date(byAdding: .day, value: -1, to: now), calendar.isDate(date, inSameDayAs: y) { return "yesterday" }
    let days = now.timeIntervalSince(date) / 86_400
    return days < 7 ? "week" : days < 30 ? "month" : "older"
  }

  /// A search's rows: every term in the title, the agent or the project, or a content hit for the query
  private func filtered(_ sessions: [SessionSummary], terms: [String], snippets: [String: String]?) -> [SessionSummary] {
    sessions.filter {
      snippets?[$0.id] != nil || SessionSearch.matches([$0.title, model.agent($0.agent).name, model.placeName($0.cwd)], terms: terms)
    }
  }
}

/// The list's grouping (date, project, channel or none): a filter button at the bar's start opening a system menu that
/// ticks the current one. By project it also says where pins sit (the desktop view menu's Pinned), and while anything
/// is unread it marks all as read
struct ListViewMenu: View {
  @Binding var view: SessionListView
  @Binding var pinPlace: PinPlace
  @Binding var archived: Bool
  var unread = false
  var onMarkAllRead: () -> Void = {}

  private func name(_ view: SessionListView) -> String { t("mobile.view.\(view.rawValue)") }

  /// One option of a pick: checked while chosen; tapping the chosen one keeps it
  private func choice<Value: Hashable>(_ value: Value, of pick: Binding<Value>, _ title: String, _ icon: String) -> some View {
    Toggle(isOn: Binding(get: { pick.wrappedValue == value }, set: { if $0 { pick.wrappedValue = value } })) {
      Label(title, systemImage: icon)
    }
  }

  var body: some View {
    Menu {
      // Titled sections of checked toggles rather than inline pickers: a menu drops an inline picker's label and the
      // title of a section around one, which left 列表顶部 / 所在项目内 without the 置顶 that says what they place
      Section(t("session.view.group")) {
        choice(.time, of: $view, name(.time), "clock")
        choice(.project, of: $view, name(.project), "folder")
        choice(.agent, of: $view, name(.agent), "antenna.radiowaves.left.and.right")
        choice(.none, of: $view, name(.none), "list.bullet")
      }
      if view == .project {
        Section(t("session.view.pinned")) {
          choice(.top, of: $pinPlace, t("session.view.pinned.top"), "pin")
          choice(.project, of: $pinPlace, t("session.view.pinned.project"), "folder")
        }
      }
      Section {
        Toggle(isOn: $archived) { Label(t("session.view.status.archived"), systemImage: "archivebox") }
        if unread {
          Button(t("session.view.markAllRead"), systemImage: "envelope.open", action: onMarkAllRead)
        }
      }
    } label: {
      Image(systemName: "line.3.horizontal.decrease")
    }
    .accessibilityLabel(Text(web: t("session.view.group")))
    .accessibilityValue(Text(web: name(view)))
  }
}

/// A group's caption (置顶, 今天 …)
struct Caption: View {
  let text: String
  var body: some View {
    Text(web: text, paragraph: false)
      .textStyle(TypeScale.text3, weight: .medium)
      .foregroundStyle(Palette.fg3)
      .padding(.horizontal, Metrics.pad)
      .frame(maxWidth: .infinity, minHeight: Metrics.groupCaption, alignment: .bottomLeading)
      .padding(.bottom, Metrics.s1)
  }
}

func projectName(_ cwd: String) -> String {
  cwd.split(whereSeparator: { $0 == "/" || $0 == "\\" }).last.map(String.init) ?? cwd
}

/// The engine's ISO timestamps, with fractional seconds or without. A formatter is costly to make and parses safely
/// from any thread, so one pair serves every row: a new pair per call, five calls per session per list render, showed
/// up in the profile of pushing a chat
enum SessionDate {
  nonisolated(unsafe) private static let fractional: ISO8601DateFormatter = {
    let parser = ISO8601DateFormatter()
    parser.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    return parser
  }()
  nonisolated(unsafe) private static let plain = ISO8601DateFormatter()

  static func parse(_ iso: String) -> Date? { fractional.date(from: iso) ?? plain.date(from: iso) }
}

/// Today: the time of day; earlier: month/day — both in the UI locale
func sessionTime(_ iso: String, now: Date = Date()) -> String {
  guard let date = SessionDate.parse(iso) else { return "" }
  let locale = Locale(identifier: L10n.locale)
  if Calendar.current.isDate(date, inSameDayAs: now) {
    return date.formatted(Date.FormatStyle(locale: locale).hour(.defaultDigits(amPM: .abbreviated)).minute(.twoDigits))
  }
  return date.formatted(Date.FormatStyle(locale: locale).month(.defaultDigits).day(.defaultDigits))
}

/// `text` with a search's terms in the strong ink and weight (SessionList.tsx Marked)
func marked(_ text: String, terms: [String], size: CGFloat) -> AttributedString {
  var out = AttributedString()
  for (i, part) in SessionSearch.parts(text, terms: terms).enumerated() where !part.isEmpty {
    var run = CJKWidth.attributed(part, size: size, paragraph: false)
    if i % 2 == 1 {
      run.foregroundColor = Palette.fgStrong
      run.inlinePresentationIntent = .stronglyEmphasized
    }
    out += run
  }
  return out
}

// MARK: - Pieces

/// The system search field and new session ride the bottom bar (iOS 26 Mail / Notes); the top bar is `ListTopBar`.
/// In the sidebar the search stands under the top bar, as the desktop's column has it, and the foot holds settings
/// and the computer; new sessions come from the chat's own bar beside it
struct SystemListBar: ViewModifier {
  @Binding var query: String
  var onNew: () -> Void
  @Environment(\.listInSidebar) private var inSidebar

  @ViewBuilder func body(content: Content) -> some View {
    #if os(iOS)
    if inSidebar {
      content
        .navigationBarTitleDisplayModeInline()
        .searchable(text: $query, placement: .navigationBarDrawer(displayMode: .always), prompt: Text(web: t("session.searchContent")))
    } else {
      phone(content)
    }
    #else
    phone(content)
    #endif
  }

  @ViewBuilder private func phone(_ content: Content) -> some View {
    if #available(iOS 26, *) {
      content
        .navigationBarTitleDisplayModeInline()
        .toolbar {
          #if os(iOS)
          DefaultToolbarItem(kind: .search, placement: .bottomBar)
          ToolbarSpacer(.fixed, placement: .bottomBar)
          ToolbarItem(placement: .bottomBar) {
            Button(action: onNew) { Image(systemName: "square.and.pencil") }
              .accessibilityLabel(Text(web: t("session.new")))
          }
          #endif
        }
        // The engine searches the conversations too, so the field says so
        .searchable(text: $query, prompt: Text(web: t("session.searchContent")))
    } else {
      // Before iOS 26 the system search field cannot ride the bottom bar: the same field and new button are drawn
      // here, floating over the list as 26's bar does
      content
        .navigationBarTitleDisplayModeInline()
        .safeAreaInset(edge: .bottom) { LegacyListFoot(query: $query, onNew: onNew) }
    }
  }
}

/// iOS 18's stand-in for the bottom bar's search field and new button (`SystemListBar`): a glass capsule holding the
/// field, a glass circle beside it
private struct LegacyListFoot: View {
  @Binding var query: String
  var onNew: () -> Void
  @FocusState private var focused: Bool

  var body: some View {
    HStack(spacing: Metrics.gap) {
      HStack(spacing: Metrics.s2) {
        Image(systemName: "magnifyingglass").foregroundStyle(Palette.fg3)
        TextField(text: $query) { Text(web: t("session.searchContent")).foregroundStyle(Palette.fg3) }
          .textStyle(TypeScale.text2)
          .foregroundStyle(Palette.fg1)
          .submitLabel(.search)
          .autocorrectionDisabled()
          .focused($focused)
        if !query.isEmpty {
          Button { query = "" } label: { Image(systemName: "xmark.circle.fill").foregroundStyle(Palette.fg3) }
            .buttonStyle(.plain)
            .accessibilityLabel(Text(web: t("panel.clear")))
        }
      }
      .padding(.horizontal, Metrics.pad)
      .frame(height: Metrics.glassCtl)
      .glassSurface(GlassStyle.bar, in: Capsule())
      .onTapGesture { focused = true }
      Button(action: onNew) {
        Image(systemName: "square.and.pencil")
          .font(.system(size: Metrics.glassGlyph, weight: .medium))
          .foregroundStyle(Palette.fg1)
          .frame(width: Metrics.glassCtl, height: Metrics.glassCtl)
      }
      .buttonStyle(.plain)
      .glassSurface(GlassStyle.control.interactive(), in: Circle())
      .accessibilityLabel(Text(web: t("session.new")))
    }
    .padding(.horizontal, Metrics.pad)
    .padding(.bottom, Metrics.s2)
  }
}

/// One of the desktop app's sections with the projects of it the list shows
struct SectionRun: Identifiable {
  let section: SessionSection
  let projects: [String]
  var id: String { section.id }
}

struct SessionGroup: Identifiable {
  let id: String
  let caption: String?
  let sessions: [SessionSummary]
  /// A day caption: it says when, so its rows leave the time out
  var dated = false
}

/// The list's bar: the grouping filter at the start, what the list shows centred (the followed computer, whose menu
/// switches computers), settings at the end. A system bar rather than a title row, so it stays while the list scrolls
/// under it and the computer and grouping are always in reach
struct ListTopBar: ViewModifier {
  /// The grouping menu; none while the hero covers the list, during a search or with nothing to group
  var filter: ListViewMenu?
  var onSettings: () -> Void
  /// In the sidebar the filter stays and the computer and settings move to the foot (SidebarFoot)
  @Environment(\.listInSidebar) private var inSidebar

  func body(content: Content) -> some View {
    content
      // A pushed chat shows back without a word beside it
      .navigationTitle("")
      .navigationBarTitleDisplayModeInline()
      .backWithoutMenu()
      #if os(iOS)
      // The title is text, not a control: no glass capsule behind it
      .bareToolbarItem(placement: .principal, shown: !inSidebar) { ListBarTitle() }
      #endif
      .toolbar {
        #if os(iOS)
        if let filter { ToolbarItem(placement: .topBarLeading) { filter } }
        if !inSidebar {
          ToolbarItem(placement: .topBarTrailing) {
            Button(action: onSettings) { Image(systemName: "gearshape") }
              .accessibilityLabel(Text(web: t("session.settings")))
          }
        }
        #endif
      }
      .safeAreaBarIfAvailable(edge: .bottom) { if inSidebar { SidebarFoot() } }
  }
}

/// One session, full width, one fact per block: the agent's mark centred on both lines; the title, with the time at
/// its right end where no day caption says it; under it the place (the project, or inside a project's group the
/// agent); then the state's mark alone in a trailing column centred on both lines, the desktop's marks without a word
/// beside them
struct SessionRow<Items: View>: View {
  let session: SessionSummary
  let agentName: String
  /// The project under the title; none inside a project's group, where the row is one line (the mark already names
  /// the agent, which a second line only repeated)
  let place: String?
  var time: String?
  var divider = true
  /// Inside a project's group: a step in from the folder row
  var nested = false
  /// A search's content hit: the text around the match, a line of its own under the place
  var snippet: String?
  /// A search's terms, marked in the title and the snippet
  var terms: [String] = []
  /// The chat open beside the sidebar
  var active = false
  var onSelect: () -> Void
  /// The long press's menu (the list hands it the session menu)
  @ViewBuilder var menu: () -> Items

  var body: some View {
    Button(action: onSelect) {
      // Marks sit centred on both lines: a mark on the title line read as hanging off the block
      HStack(alignment: .center, spacing: Metrics.gap) {
        // The bare mark, its ink fitted and centred (logos sit off-centre in their own boxes). Its column starts the
        // slot, so the ink stands on the title's and captions' edge rather than centred a few points in
        VendorMark(brand: session.agent, name: agentName, size: Metrics.rowMark, fit: true)
          .foregroundStyle(Palette.fg2)
          .frame(width: Metrics.rowMark, height: Metrics.lead)
          .frame(width: Metrics.lead, alignment: .leading)
        VStack(alignment: .leading, spacing: Metrics.s0_5) {
          HStack(alignment: .firstTextBaseline, spacing: Metrics.s2) {
            Text(marked(session.title, terms: terms, size: TypeScale.field.size)).lineLimit(1).truncationMode(.tail)
              .textStyle(TypeScale.field)
              .foregroundStyle(Palette.fgStrong)
              .frame(maxWidth: .infinity, alignment: .leading)
            if let time {
              Text(web: time, paragraph: false)
                .textStyle(TypeScale.text3).monospacedDigit().foregroundStyle(Palette.fg3).fixedSize()
            }
            // The state is its mark alone, as on the desktop (its meaning is the mark's accessibility label), on the
            // time's line and centred on its digits: centred on both lines it sat below the time. Every row keeps the
            // slot, so the times above line up: an empty `Group` with a frame lays out as nothing
            Color.clear.frame(width: Metrics.stateSlot, height: Metrics.stateSlot).overlay {
              if let state = session.state, state != .unknown { StateMark(state: state, unread: session.unread == true) }
            }
            .padding(.leading, Metrics.gap - Metrics.s2)
            .alignmentGuide(.firstTextBaseline) { $0[VerticalAlignment.center] + TypeScale.text3.size * 0.35 }
          }
          if let place {
            Text(web: place, paragraph: false).lineLimit(1).truncationMode(.middle)
              .frame(maxWidth: .infinity, alignment: .leading)
              .textStyle(TypeScale.text3)
              .foregroundStyle(Palette.fg3)
          }
          if let snippet {
            Text(marked(snippet, terms: terms, size: TypeScale.text3.size)).lineLimit(2)
              .frame(maxWidth: .infinity, alignment: .leading)
              .textStyle(TypeScale.text3)
              .foregroundStyle(Palette.fg2)
          }
        }
      }
      .padding(.leading, nested ? Metrics.nest : 0)
      .padding(.horizontal, Metrics.pad)
      .padding(.vertical, snippet == nil ? 0 : Metrics.s2)
      // One line stands as tall as a project's header
      .frame(minHeight: place == nil && snippet == nil ? Metrics.sheetRow : Metrics.sessionRow)
      .overlay(alignment: .bottom) {
        if divider {
          // From the text column, under the lead
          Rectangle().fill(Palette.line).frame(height: Metrics.hairline)
            .padding(.leading, Metrics.pad + (nested ? Metrics.nest : 0) + Metrics.lead + Metrics.gap)
        }
      }
      .contentShape(Rectangle())
    }
    .buttonStyle(ListRowStyle(active: active))
    .contextMenu { menu() }
  }
}

/// A project's header in the by-project view: the folder in the marks' column, the name and a chevron turning down
/// while open; folded, the project's most pressing state sits in the rows' state column. A folder on an SSH host wears
/// the globe and ends in its host's alias and connection dot, as on the desktop. A tap folds it open or shut; its menu
/// starts a session in it, moves it to the top of the user's order or into one of the desktop's sections, copies its
/// path, or takes it off the list (the desktop project menu, less revealing the folder on the computer)
struct ProjectHeader: View {
  let cwd: String
  /// It leads the list already, so "move to top" is not offered
  let first: Bool
  let open: Bool
  let sessions: [SessionSummary]
  /// The SSH host the folder lives on (`AppModel.host(of:)`); nil for a local folder
  var host: (folder: RemoteFolder, view: ConnectionView?)?
  /// The desktop's sections; "Move to section" shows once there is one (they are made on the desktop)
  var sections: [SessionSection] = []
  var onToggle: () -> Void
  var onMoveTop: () -> Void
  var onRemove: () -> Void
  var onNew: () -> Void
  /// Files the folder into a section, or takes it out (nil)
  var onSection: (String?) -> Void = { _ in }
  var onCopyPath: () -> Void = {}

  var body: some View {
    Button(action: onToggle) {
      HStack(spacing: Metrics.gap) {
        // The folder in the top-level session marks' column (its own sessions sit a step in): the desktop list's
        // glyph, shut or open with the fold (SessionList.tsx), so both lists draw one folder. A pin never takes its
        // place (it says "pinned", not "project")
        Group {
          if host != nil {
            RemoteFolderGlyph(open: open).breathing(host?.view?.state == .connecting)
          } else {
            ZStack {
              // Icon's defaults are the desktop's: the icon size, a 1.5 stroke
              Icon(.folder).opacity(open ? 0 : 1)
              Icon(.folderOpen).opacity(open ? 1 : 0)
            }
          }
        }
        .foregroundStyle(Palette.fg3)
        .frame(width: Metrics.rowMark)
        .frame(width: Metrics.lead, alignment: .leading)
        HStack(spacing: Metrics.s1_5) {
          Text(web: projectName(cwd), paragraph: false).lineLimit(1).truncationMode(.tail)
            .textStyle(TypeScale.field, weight: .semibold)
            .foregroundStyle(Palette.fgStrong)
          Icon(.chevronRight, size: Metrics.chevron, strokeWidth: 2).foregroundStyle(Palette.fg3)
            .rotationEffect(.degrees(open ? 90 : 0))
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        // The alias keeps its room (up to `hostAlias`); the project's name truncates first, as on the desktop
        if let host { HostTag(alias: host.folder.alias, view: host.view).layoutPriority(1) }
        // Open, each row shows its own state; the slot stays either way, as in the rows
        Color.clear.frame(width: Metrics.stateSlot, height: Metrics.stateSlot).overlay {
          if !open, let mark = Self.pressing(sessions) { StateMark(state: mark.state, unread: mark.unread) }
        }
      }
      .padding(.horizontal, Metrics.pad)
      .frame(minHeight: Metrics.sheetRow)
      .contentShape(Rectangle())
    }
    .buttonStyle(ListRowStyle(active: false))
    .accessibilityAddTraits(.isHeader)
    .contextMenu {
      Section { Button(t("session.project.newSession"), systemImage: "square.and.pencil", action: onNew) }
      Section {
        if !first { Button(t("mobile.project.moveTop"), systemImage: "arrow.up.to.line", action: onMoveTop) }
        if !sections.isEmpty { sectionMenu }
        Button(t("session.project.copyPath"), systemImage: "doc.on.doc", action: onCopyPath)
      }
      Section { Button(t("session.project.remove"), systemImage: "minus.circle", action: onRemove) }
    }
  }

  /// The sections as a checked picker, the folder's own ticked, then "Remove from section" while it is in one
  private var sectionMenu: some View {
    let current = sections.first { $0.projects.contains(cwd) }?.id
    return Menu {
      Picker(selection: Binding(get: { current }, set: { if let id = $0, id != current { onSection(id) } })) {
        ForEach(sections) { section in
          Text(verbatim: section.name.isEmpty ? t("session.section.untitled") : section.name).tag(Optional(section.id))
        }
      } label: {
        EmptyView()
      }
      .pickerStyle(.inline)
      if current != nil { Button(t("session.section.remove"), systemImage: "minus.circle") { onSection(nil) } }
    } label: {
      Label(t("session.section.moveTo"), systemImage: "square.stack")
    }
  }

  /// The project's mark, ranked as the desktop's project rows rank it (SessionList.tsx STATE_RANK): running first, then
  /// waiting, failed, unread, background work (lit when one of those sessions has a reply nobody has looked at)
  static func pressing(_ sessions: [SessionSummary]) -> (state: SessionState, unread: Bool)? {
    let rank: [SessionState] = [.working, .waiting, .error, .unread, .background]
    guard let state = rank.first(where: { state in sessions.contains { $0.state == state } }) else { return nil }
    return (state, state == .background && sessions.contains { $0.state == .background && $0.unread == true })
  }
}

/// RemoteFolderIcon.tsx: the folder with a small globe on its lower right corner. The folder is cut away under the
/// globe, so the badge reads on any fill behind it (a pressed row's included); the globe hangs past the folder's box
struct RemoteFolderGlyph: View {
  var open: Bool

  var body: some View {
    let badge = Metrics.remoteBadge, out = Metrics.remoteOverhang
    ZStack(alignment: .bottomTrailing) {
      ZStack {
        Icon(.folder).opacity(open ? 0 : 1)
        Icon(.folderOpen).opacity(open ? 1 : 0)
      }
      .mask {
        Rectangle()
          .overlay(alignment: .bottomTrailing) {
            Circle().frame(width: badge, height: badge).offset(x: out, y: out).blendMode(.destinationOut)
          }
          .compositingGroup()
      }
      Icon(.globe, size: Metrics.remoteGlobe, strokeWidth: 2)
        .frame(width: badge, height: badge)
        .offset(x: out, y: out)
    }
  }
}

/// A host folder's header end (SessionList.tsx GroupHeader): the host's alias, then its connection as a dot: green
/// connected, amber failed, breathing while connecting, faint when off. No dot until the engine has said how the host
/// is (an older gateway keeps the hosts from devices)
struct HostTag: View {
  let alias: String
  let view: ConnectionView?

  var body: some View {
    HStack(spacing: Metrics.s1_5) {
      Text(verbatim: alias).lineLimit(1).truncationMode(.middle)
        .textStyle(TypeScale.text3)
        .foregroundStyle(Palette.fg3)
        .frame(maxWidth: Metrics.hostAlias, alignment: .trailing)
        .fixedSize(horizontal: false, vertical: true)
      if let view {
        Circle().fill(Self.dot(view.state)).frame(width: Metrics.hostDot, height: Metrics.hostDot)
          .breathing(view.state == .connecting)
      }
    }
    .accessibilityElement(children: .combine)
    .accessibilityLabel(Text(web: spoken))
  }

  /// The alias and, once known, the connection's state in words
  private var spoken: String {
    guard let view, view.state != .unknown else { return alias }
    let state = t("connections.state." + view.state.rawValue)
    return "\(alias), \(state)"
  }

  static func dot(_ state: ConnectionState) -> Color {
    switch state {
    case .connected: Palette.ok
    case .error: Palette.warn
    case .connecting: Palette.fg3
    case .off, .unknown: Palette.fg3.opacity(0.4)
    }
  }
}

extension View {
  /// `remote-breathe`: the view breathes while `on`, unless motion is off
  func breathing(_ on: Bool) -> some View { modifier(Breathing(on: on)) }
}

private struct Breathing: ViewModifier {
  let on: Bool
  @Environment(\.motionEnabled) private var motion

  func body(content: Content) -> some View {
    if on && motion {
      content.phaseAnimator([1, Motion.breatheLow]) { view, level in view.opacity(level) } animation: { _ in Motion.breathe }
    } else {
      content
    }
  }
}

/// SessionList.tsx MoreRow, under an open project's latest few. It tells what the rest holds: how many, the agents among
/// them (their marks, the first few) and the most urgent state inside, on the rows' state column. A tap brings out the
/// next page above it; with all of them out it reads "show less", the chevron turned over, and a tap folds them away
struct MoreRow: View {
  /// The sessions still hidden, in list order
  let rest: [SessionSummary]
  var nameOf: (AgentId) -> String
  /// Under a project's rows, a step in; under the chats, at the top level's inset
  var nested = true
  var action: () -> Void

  /// SessionList.tsx MORE_MARKS
  static let marks = 3

  var body: some View {
    let all = rest.isEmpty
    var seen = Set<AgentId>()
    let agents = Array(rest.map(\.agent).filter { seen.insert($0).inserted }.prefix(Self.marks))
    Button(action: action) {
      HStack(spacing: Metrics.gap) {
        // The chevron in the sessions' mark column
        Icon(.chevronDown, size: Metrics.chevron, strokeWidth: 2)
          .foregroundStyle(Palette.fg3)
          .rotationEffect(.degrees(all ? 180 : 0))
          .frame(width: Metrics.rowMark)
          .frame(width: Metrics.lead, alignment: .leading)
        ZStack(alignment: .leading) {
          Text(web: t("session.showMore", ["n": String(rest.count)]), paragraph: false).opacity(all ? 0 : 1)
          Text(web: t("session.showLess"), paragraph: false).opacity(all ? 1 : 0)
        }
        .textStyle(TypeScale.text2)
        .foregroundStyle(Palette.fg3)
        .lineLimit(1)
        HStack(spacing: Metrics.s1) {
          ForEach(agents, id: \.self) { agent in
            VendorMark(brand: agent, name: nameOf(agent), size: Metrics.moreMark, fit: true)
          }
        }
        .foregroundStyle(Palette.fg2)
        .opacity(all ? 0 : 1)
        Spacer(minLength: 0)
        Color.clear.frame(width: Metrics.stateSlot, height: Metrics.stateSlot).overlay {
          if let mark = ProjectHeader.pressing(rest) { StateMark(state: mark.state, unread: mark.unread) }
        }
      }
      // Its chevron on its sessions' marks
      .padding(.leading, nested ? Metrics.nest : 0)
      .padding(.horizontal, Metrics.pad)
      .frame(minHeight: Metrics.sheetRow)
      .contentShape(Rectangle())
    }
    .buttonStyle(ListRowStyle(active: false))
    .accessibilityLabel(Text(web: all ? t("session.showLess") : t("session.showMore", ["n": String(rest.count)])))
  }
}

/// The empty chats group's only row (SessionList.tsx EmptyProjectRow, flush): a faint plus in the marks' column and
/// "New chat"
struct NewChatRow: View {
  var action: () -> Void

  var body: some View {
    Button(action: action) {
      HStack(spacing: Metrics.gap) {
        Icon(.plus)
          .frame(width: Metrics.rowMark)
          .frame(width: Metrics.lead, alignment: .leading)
        Text(web: t("session.chat.new"), paragraph: false).lineLimit(1)
          .textStyle(TypeScale.text2)
          .frame(maxWidth: .infinity, alignment: .leading)
      }
      .foregroundStyle(Palette.fg3)
      .padding(.horizontal, Metrics.pad)
      .frame(minHeight: Metrics.sheetRow)
      .contentShape(Rectangle())
    }
    .buttonStyle(ListRowStyle(active: false))
  }
}

/// The end of a pinned run: the desktop's short faded rule (SessionList.tsx `PinDivider`) instead of a pin on each row,
/// which pushed the row's title off the text line
struct PinRule: View {
  var body: some View {
    LinearGradient(colors: [.clear, Palette.line, .clear], startPoint: .leading, endPoint: .trailing)
      .frame(height: Metrics.hairline)
      .padding(.horizontal, Metrics.pad)
      .padding(.vertical, Metrics.s1)
  }
}

/// The pressed fill only; rows run edge to edge, so no radius
struct ListRowStyle: ButtonStyle {
  var active: Bool
  func makeBody(configuration: Configuration) -> some View {
    configuration.label
      .background(Rectangle().fill(active ? Palette.active : configuration.isPressed ? Palette.hover : .clear))
  }
}

/// The desktop list's state marks (packages/ui chat/SessionList.tsx StateMark, motion in styles/motion.css), one slot
/// each so they share a centre line. Working: the spinning ring, which keeps turning with motion off (a list row has no
/// verb to shimmer). Waiting: an amber dot beating like a heart. Error: a red dot that double-flashes every few seconds.
/// Unread (a turn finished unwatched): a white dot with a halo rippling out of it. Background (the turn ended, a task it
/// started still runs): the ring at rest with a dot orbiting it slowly, lit white with a dot inside once a reply came in
/// meanwhile. Motion off stops all but the ring on their resting frame (full dot, lit dot, orbit dot at the top).
/// Syncing, the live states (working, waiting, background) rest and breathe like a connecting glyph: what the last wire
/// said may have ended since
struct StateMark: View {
  let state: SessionState
  var unread = false
  @Environment(\.motionEnabled) private var motion
  @Environment(\.syncing) private var syncing

  var body: some View {
    let held = syncing && (state == .working || state == .waiting || state == .background)
    let moving = !held && (motion || state == .working)
    TimelineView(.animation(paused: !moving || state == .unknown)) { context in
      glyph(moving ? context.date.timeIntervalSinceReferenceDate : nil)
    }
    .breathing(held)
    .frame(width: Metrics.stateSlot, height: Metrics.stateSlot)
    .accessibilityElement()
    .accessibilityLabel(Text(web: t(state == .background && unread ? "session.state.backgroundUnread" : "session.state.\(state.rawValue)")))
  }

  /// The mark at time `t` (seconds), or on its resting frame when nil
  @ViewBuilder
  private func glyph(_ t: Double?) -> some View {
    switch state {
    case .working:
      // animate-spin: one turn a second, linear
      Icon(.loaderCircle, size: Metrics.stateRing, strokeWidth: 2.5)
        .foregroundStyle(Palette.fg2)
        .rotationEffect(.degrees(Self.phase(t, 1) * 360))
    case .waiting:
      Circle().fill(Palette.warn)
        .frame(width: Metrics.stateDot, height: Metrics.stateDot)
        .scaleEffect(t == nil ? 1 : Self.beat(Self.phase(t, 1.4)))
    case .error:
      Circle().fill(Palette.danger)
        .frame(width: Metrics.stateDot, height: Metrics.stateDot)
        .opacity(t == nil ? 1 : Self.flash(Self.phase(t, 3)))
    case .unread:
      ZStack {
        // The halo: a copy of the dot growing to 2.6 times while it fades, ease-out over 2 s; invisible at rest
        if t != nil {
          let p = 1 - pow(1 - Self.phase(t, 2), 2)
          Circle().fill(Palette.fgStrong)
            .frame(width: Metrics.stateUnread, height: Metrics.stateUnread)
            .scaleEffect(1 + 1.6 * p)
            .opacity(0.6 * (1 - p))
        }
        Circle().fill(Palette.fgStrong).frame(width: Metrics.stateUnread, height: Metrics.stateUnread)
      }
    case .background:
      ZStack {
        Icon(.circle, size: Metrics.stateRing, strokeWidth: 2.5).foregroundStyle(unread ? Palette.fgStrong : Palette.fg3)
        // The orbit rides the ring's own line (Lucide's circle: r 10 in a 24-unit box), one turn in 3.6 s
        Circle().fill(unread ? Palette.fgStrong : Palette.fg2)
          .frame(width: Metrics.stateOrbit, height: Metrics.stateOrbit)
          .offset(y: -Metrics.stateRing * 10 / 24)
          .rotationEffect(.degrees(Self.phase(t, 3.6) * 360))
        if unread { Circle().fill(Palette.fgStrong).frame(width: Metrics.stateInner, height: Metrics.stateInner) }
      }
    case .unknown:
      EmptyView()
    }
  }

  /// Where in a `period`-second loop `t` falls, 0..<1 (0 at rest)
  private static func phase(_ t: Double?, _ period: Double) -> Double {
    guard let t else { return 0 }
    return t.truncatingRemainder(dividingBy: period) / period
  }

  /// acp-mark-beat: 0 / 100% 1, 20% 1.5, 40% 1.1, 55% 1.35, 75% 1, eased in and out between keys
  static func beat(_ p: Double) -> Double {
    keyframes(p, [(0, 1), (0.2, 1.5), (0.4, 1.1), (0.55, 1.35), (0.75, 1), (1, 1)], eased: true)
  }

  /// acp-mark-flash: lit, then two dips to 0.2 at 70% and 82%, linear between keys
  static func flash(_ p: Double) -> Double {
    keyframes(p, [(0, 1), (0.64, 1), (0.7, 0.2), (0.76, 1), (0.82, 0.2), (0.88, 1), (1, 1)], eased: false)
  }

  private static func keyframes(_ p: Double, _ keys: [(Double, Double)], eased: Bool) -> Double {
    guard let upper = keys.firstIndex(where: { $0.0 >= p }), upper > 0 else { return keys.first?.1 ?? 1 }
    let (p0, v0) = keys[upper - 1], (p1, v1) = keys[upper]
    var x = p1 > p0 ? (p - p0) / (p1 - p0) : 1
    // CSS ease-in-out, close enough as a smoothstep for a few pixels of scale
    if eased { x = x * x * (3 - 2 * x) }
    return v0 + (v1 - v0) * x
  }
}
