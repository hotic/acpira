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
  /// Creating or renaming a custom project section
  @State private var sectionPrompt: SectionPrompt?
  @State private var sectionName = ""
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
  @Environment(SidebarPicks.self) private var picks: SidebarPicks?

  /// Sessions an open project shows before "N more" (the desktop view menu's default)
  private static let perProject = 5
  /// What one tap on "N more" brings out (SessionList.tsx MORE_PAGE): a project of hundreds never lays them all out at once
  private static let morePage = 10

  var body: some View {
    // Paired remote access: the followed computer's state takes the page until it has sessions to list. The bar sits
    // over both, so the hero giving way to the list moves nothing in it
    let covered = pairing.map { PairHeroList.covers($0, model) } ?? false
    // Listed folders alone keep the filter too: from another view its menu is the way back to them
    let filter = !covered && searchTerms(query).isEmpty && !(model.summaries.isEmpty && model.listedProjects.isEmpty && model.sections.isEmpty)
    PairListGate(pairing: pairing) { list }
      .modifier(ListTopBar(filter: filter ? ListViewMenu(view: $view, pinPlace: $pinPlace,
                                                         archived: $archivedOnly,
                                                         unread: model.summaries.contains { $0.unread == true || $0.state == .unread },
                                                         onMarkAllRead: model.markAllRead,
                                                         onNewSection: { sectionName = ""; sectionPrompt = .create(project: nil) }) : nil,
                           onSettings: { model.settingsOpen = true }))
  }

  /// Opens a page from the list: pushed over it on the phone, beside it in place of the open one in the sidebar
  private func show(_ route: Route) {
    guard inSidebar else { return model.push(route) }
    model.path = [route]
    picks?.count += 1
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
        if ordered.isEmpty && !(byProject && (!model.listedProjects.isEmpty || !model.sections.isEmpty || model.chatRoot != nil)) {
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
    .alert(sectionPrompt?.title ?? "", isPresented: Binding(get: { sectionPrompt != nil }, set: { if !$0 { sectionPrompt = nil } })) {
      TextField(text: $sectionName) { Text(web: t("session.section.untitled")) }
      Button(t("mobile.cancel"), role: .cancel) {}
      Button(t("mobile.save")) {
        guard let prompt = sectionPrompt else { return }
        withAnimation(Motion.ease) {
          switch prompt {
          case .create(let project): model.createSection(sectionName, project: project)
          case .rename(let section): model.renameSection(section.id, to: sectionName)
          }
        }
      }
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

  /// The by-project view's projects: the user's sections first, each under its name with its projects in its own order,
  /// then the projects in none under "Projects" (the desktop list's layout); without sections, the projects alone
  @ViewBuilder
  private func projectGroups(pinsInProject: Bool, projects: [String]) -> some View {
    let listed = model.listedProjects
    let runs = model.sections.map { SectionRun(section: $0, projects: $0.projects.filter(listed.contains)) }
    if runs.isEmpty {
      ForEach(listed, id: \.self) { projectGroup($0, section: nil, run: listed, pinsInProject: pinsInProject, projects: projects) }
    } else {
      let filed = Set(runs.flatMap(\.projects))
      let rest = listed.filter { !filed.contains($0) }
      ForEach(Array(runs.enumerated()), id: \.element.id) { index, run in
        let open = run.section.collapsed != true
        SectionCaptionHeader(
          section: run.section,
          open: open,
          canMoveUp: index > 0,
          canMoveDown: index < runs.count - 1,
          onToggle: { if !closeSwipe() { withAnimation(Motion.ease) { model.collapseSection(run.section.id, open) } } },
          onRename: { sectionName = run.section.name; sectionPrompt = .rename(run.section) },
          onMoveUp: { withAnimation(Motion.ease) { model.reorderSection(run.section.id, before: runs[index - 1].id) } },
          onMoveDown: { withAnimation(Motion.ease) { model.reorderSection(run.section.id, before: runs[safe: index + 2]?.id) } },
          onDelete: { withAnimation(Motion.ease) { model.deleteSection(run.section.id) } }
        )
        .modifier(captionZone(section: run.section.id))
        if open {
          if run.projects.isEmpty {
            Text(web: t("session.section.empty"), paragraph: false)
              .textStyle(TypeScale.text3)
              .foregroundStyle(Palette.fg3)
              .padding(.horizontal, Metrics.pad)
              .frame(maxWidth: .infinity, minHeight: Metrics.sheetRow, alignment: .leading)
              .modifier(captionZone(section: run.section.id))
          } else {
            ForEach(run.projects, id: \.self) {
              projectGroup($0, section: run.section.id, run: run.projects, pinsInProject: pinsInProject, projects: projects)
            }
          }
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
                    onNewSection: { sectionName = ""; sectionPrompt = .create(project: cwd) },
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
        exportJSON: model.holdsWholeView(s.id),
        copy: { kind in Task { await model.copy(s.id, kind) } },
        cwd: s.cwd, projects: projects, sections: model.sections, chat: model.isChat(s.cwd),
        move: { model.move(s.id, to: $0) },
        addFolder: { movingTo = s }))
    }
    // Deleted at once, as on the desktop: the toast's undo brings it back
    .modifier(SwipeToDelete(id: s.id, open: $swiped) { withAnimation(Motion.ease) { model.delete(s.id) } })
    // By project a session drags onto another project; pinned ones stay put, as on the desktop (sessionTree.ts draggable)
    .listDrag(filing && s.pinned != true ? .session(s.id) : nil, onStart: { held = $0 }) {
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
  var onNewSection: (() -> Void)?

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
        if !archived, let onNewSection {
          Section {
            Button(t("session.section.newEllipsis"), systemImage: "plus", action: onNewSection)
          }
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
    .barButtonBase()
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

/// `text` with a search's terms in the strong ink and weight (SessionRows.tsx Marked)
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
