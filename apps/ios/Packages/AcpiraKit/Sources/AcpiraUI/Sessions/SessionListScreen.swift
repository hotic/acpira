import AcpiraDesign
import AcpiraFixtures
import AcpiraModels
import SwiftUI

// The phone's first screen: every session, in one of two views picked under the title (as Trae's phone app switches
// its list between a timeline and groups, rather than stacking projects over a timeline). By time: the sessions
// pinned to the top, then day captions. By project: the same pinned run, then each project as a header folding open
// in place, its own pins first and the latest few, the rest behind "N more" (the desktop list's project grouping and
// its view menu's defaults; a page per project put back and its title on two rows). A search lists one run across
// every project. Every row keeps one fact per block: the title, the place (the project, or inside a project's group
// the agent) and the state each in their own spot, never joined on one line.

/// The list's two views (`Prefs.listView`)
enum SessionListView: String, CaseIterable { case time, project }

struct SessionListScreen: View {
  @Environment(AppModel.self) private var model
  @AppStorage(Prefs.listView) private var view = SessionListView.time
  @State private var query = ""
  @State private var renaming: SessionSummary?
  @State private var newTitle = ""
  @State private var deleting: SessionSummary?
  /// Open projects showing every session, past their latest few
  @State private var fullProjects: Set<String> = []
  @Environment(\.now) private var now

  /// Sessions an open project shows before "N more" (the desktop view menu's default)
  private static let perProject = 5

  var body: some View {
    let searching = !query.trimmingCharacters(in: .whitespaces).isEmpty
    let shown = searching ? filtered(model.summaries) : model.summaries
    // Pinned first, then the latest
    let ordered = shown.sorted { ($0.pinned == true ? 1 : 0, $0.updatedAt) > ($1.pinned == true ? 1 : 0, $1.updatedAt) }
    let byProject = view == .project && !searching
    let groups = groups(ordered, searching: searching, byProject: byProject)
    let onNew = { model.path.append(.new(agent: nil, cwd: model.projects.first ?? "")) }
    ScrollView(.vertical) {
      LazyVStack(alignment: .leading, spacing: 0) {
        // The list has no bar: settings sits at the title row's end and comes up as a sheet
        LargeTitle(title: t("mobile.home")) { model.settingsOpen = true }
        if !searching && !model.summaries.isEmpty { ListViewMenu(view: $view) }
        if ordered.isEmpty && !(byProject && !model.listedProjects.isEmpty) {
          Text(web: searching ? t("session.noMatch") : t("session.noneWorkspace"))
            .textStyle(TypeScale.text2).foregroundStyle(Palette.fg3)
            .frame(maxWidth: .infinity, minHeight: Metrics.sheetRow)
        }
        ForEach(groups) { group($0, searching: searching) }
        if byProject { ForEach(model.listedProjects, id: \.self) { projectGroup($0) } }
      }
      .padding(.bottom, Metrics.msgGap)
      .animation(Motion.ease, value: ordered.map(\.id))
      .animation(Motion.ease, value: model.listedProjects)
      .animation(Motion.ease, value: view)
    }
    .scrollIndicators(.hidden)
    .background(Palette.bg0)
    .modifier(SystemListBar(query: $query, onNew: onNew))
    // The large title is the list's first row; a pushed chat shows back without a word beside it
    .navigationTitle("")
    // Rows blur and dim gradually as they scroll up under the status bar
    .topFade(tail: 0)
    .hiddenNavigationBar()
    .alert(t("common.rename"), isPresented: Binding(get: { renaming != nil }, set: { if !$0 { renaming = nil } })) {
      TextField(text: $newTitle) { Text(web: t("session.titleAria")) }
      Button(t("mobile.cancel"), role: .cancel) {}
      Button(t("mobile.save")) { if let s = renaming { model.rename(s.id, to: newTitle) } }
    }
    .confirmationDialog(t("mobile.deleteConfirm", ["title": deleting?.title ?? ""]),
                        isPresented: Binding(get: { deleting != nil }, set: { if !$0 { deleting = nil } }),
                        titleVisibility: .visible) {
      Button(t("common.delete"), role: .destructive) { if let s = deleting { model.delete(s.id) } }
      Button(t("mobile.cancel"), role: .cancel) {}
    }
  }

  @ViewBuilder
  private func group(_ group: SessionGroup, searching: Bool) -> some View {
    if let caption = group.caption { Caption(text: caption) }
    ForEach(Array(group.sessions.enumerated()), id: \.element.id) { i, s in
      row(s, place: projectName(s.cwd),
          // A caption already says "pinned"; one run without captions marks its pins
          pinGlyph: group.caption == nil && s.pinned == true,
          // A day caption already says when; pinned rows and search hits have none, so they keep the time
          time: group.dated ? nil : sessionTime(s.updatedAt, now: now),
          // Captions part the runs and rows sit on space alone; one run without captions parts by hairlines
          divider: i < group.sessions.count - 1 && group.caption == nil,
          inProject: false)
    }
  }

  /// A project in the by-project view: its header, then, open, its sessions not pinned to the top (its own pins
  /// first, then the latest), the latest few only until "N more" is tapped
  @ViewBuilder
  private func projectGroup(_ cwd: String) -> some View {
    let all = model.summaries.filter { $0.cwd == cwd }
    let here = { (s: SessionSummary) in model.projectPins.contains(s.id) ? 1 : 0 }
    let sessions = all.filter { $0.pinned != true }.sorted { (here($0), $0.updatedAt) > (here($1), $1.updatedAt) }
    let open = model.openProjects.contains(cwd)
    let rows = !open ? [] : fullProjects.contains(cwd) || sessions.count <= Self.perProject + 1
      ? sessions : Array(sessions.prefix(Self.perProject))
    let more = open ? sessions.count - rows.count : 0
    VStack(spacing: 0) {
      ProjectHeader(cwd: cwd, pinned: model.pinnedProjects.contains(cwd), open: open, sessions: all,
                    onToggle: { withAnimation(Motion.ease) { model.toggleOpen(cwd) } },
                    onPin: { model.togglePinProject(cwd) },
                    onNew: { model.path.append(.new(agent: nil, cwd: cwd)) })
      ForEach(Array(rows.enumerated()), id: \.element.id) { i, s in
        // Inside its project a row names the agent; no caption says when, so every row keeps its time
        row(s, place: model.agent(s.agent).name, pinGlyph: model.projectPins.contains(s.id),
            time: sessionTime(s.updatedAt, now: now), divider: false, inProject: true)
      }
      if more > 0 {
        MoreRow(count: more) { _ = withAnimation(Motion.ease) { fullProjects.insert(cwd) } }
      }
    }
  }

  /// One session with its menu: pinned to the list's top, and (inside a project's group) pinned within the project
  private func row(_ s: SessionSummary, place: String, pinGlyph: Bool, time: String?, divider: Bool,
                   inProject: Bool) -> some View {
    SessionRow(session: s, agentName: model.agent(s.agent).name, place: place, pinGlyph: pinGlyph, time: time,
               divider: divider,
               onSelect: { model.path.append(.chat(s.id)) },
               onRename: { newTitle = s.title; renaming = s },
               onDelete: { deleting = s }) {
      Button(s.pinned == true ? t("common.unpin") : t("common.pin"), systemImage: s.pinned == true ? "pin.slash" : "pin") {
        model.togglePin(s.id)
      }
      if inProject {
        Button(model.projectPins.contains(s.id) ? t("mobile.unpinInProject") : t("mobile.pinInProject"),
               systemImage: "pin.circle") { model.toggleProjectPin(s.id) }
      }
    }
  }

  /// By time: pinned, then today / yesterday / this week / this month / older, captions over each. By project: the
  /// pinned run alone, the projects follow. A search: one run
  private func groups(_ sessions: [SessionSummary], searching: Bool, byProject: Bool) -> [SessionGroup] {
    if searching {
      return [SessionGroup(id: "all", caption: nil, sessions: sessions)]
    }
    let top = sessions.filter { $0.pinned == true }
    var out: [SessionGroup] = top.isEmpty ? [] : [SessionGroup(id: "pinned", caption: t("session.group.pinned"), sessions: top)]
    guard !byProject else { return out }
    let rest = sessions.filter { $0.pinned != true }
    for key in ["today", "yesterday", "week", "month", "older"] {
      let members = rest.filter { recency($0.updatedAt) == key }
      if !members.isEmpty {
        out.append(SessionGroup(id: key, caption: t("session.when.\(key)"), sessions: members, dated: true))
      }
    }
    return out
  }

  private func recency(_ iso: String) -> String {
    let parser = ISO8601DateFormatter()
    parser.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
    guard let date = parser.date(from: iso) ?? ISO8601DateFormatter().date(from: iso) else { return "older" }
    let calendar = Calendar.current
    if calendar.isDate(date, inSameDayAs: now) { return "today" }
    if let y = calendar.date(byAdding: .day, value: -1, to: now), calendar.isDate(date, inSameDayAs: y) { return "yesterday" }
    let days = now.timeIntervalSince(date) / 86_400
    return days < 7 ? "week" : days < 30 ? "month" : "older"
  }

  private func filtered(_ sessions: [SessionSummary]) -> [SessionSummary] {
    let q = query.trimmingCharacters(in: .whitespaces).lowercased()
    return sessions.filter {
      $0.title.lowercased().contains(q) || model.agent($0.agent).name.lowercased().contains(q)
        || projectName($0.cwd).lowercased().contains(q)
    }
  }
}

/// The list's view under the title, by time or by project: a system menu ticking the current one
struct ListViewMenu: View {
  @Binding var view: SessionListView

  private func name(_ view: SessionListView) -> String { t(view == .time ? "mobile.view.time" : "mobile.view.project") }

  var body: some View {
    Menu {
      Picker(selection: $view) {
        Label(name(.time), systemImage: "clock").tag(SessionListView.time)
        Label(name(.project), systemImage: "folder").tag(SessionListView.project)
      } label: { EmptyView() }
      .pickerStyle(.inline)
    } label: {
      HStack(spacing: Metrics.s1) {
        Text(web: name(view), paragraph: false)
          .textStyle(TypeScale.text2, weight: .semibold)
          .foregroundStyle(Palette.fg1)
        Icon(.chevronDown, size: Metrics.chevron, strokeWidth: 2).foregroundStyle(Palette.fg3)
      }
      .contentShape(Rectangle())
    }
    .menuStyle(.button)
    .buttonStyle(.plain)
    .padding(.horizontal, Metrics.pad)
    .frame(minHeight: Metrics.groupCaption, alignment: .leading)
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

/// Today: the time of day; earlier: month/day — both in the UI locale
func sessionTime(_ iso: String, now: Date = Date()) -> String {
  let parser = ISO8601DateFormatter()
  parser.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
  guard let date = parser.date(from: iso) ?? ISO8601DateFormatter().date(from: iso) else { return "" }
  let locale = Locale(identifier: L10n.locale)
  if Calendar.current.isDate(date, inSameDayAs: now) {
    return date.formatted(Date.FormatStyle(locale: locale).hour(.defaultDigits(amPM: .abbreviated)).minute(.twoDigits))
  }
  return date.formatted(Date.FormatStyle(locale: locale).month(.defaultDigits).day(.defaultDigits))
}

// MARK: - Pieces

/// The list's large title is its first row and the page has no navigation bar (a bar holding only settings read as an
/// empty row over the title; settings sits at the title row's end). The system search field and new session ride the
/// bottom bar (iOS 26 Mail / Notes)
struct SystemListBar: ViewModifier {
  @Binding var query: String
  var onNew: () -> Void

  func body(content: Content) -> some View {
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
      .searchable(text: $query, prompt: Text(web: t("session.search")))
  }
}

struct SessionGroup: Identifiable {
  let id: String
  let caption: String?
  let sessions: [SessionSummary]
  /// A day caption: it says when, so its rows leave the time out
  var dated = false
}

/// The list's iOS large title, with settings as a glass circle at the row's end (the page has no bar)
struct LargeTitle: View {
  let title: String
  var onSettings: (() -> Void)?

  var body: some View {
    Text(web: title, paragraph: false).lineLimit(1).truncationMode(.tail)
      .textStyle(TypeScale.title, weight: .bold)
      .foregroundStyle(Palette.fgStrong)
      .accessibilityAddTraits(.isHeader)
      // A long title truncates before the button instead of running under it
      .padding(.trailing, onSettings == nil ? 0 : Metrics.glassCtl + Metrics.s2)
      .frame(maxWidth: .infinity, alignment: .leading)
      .overlay(alignment: .trailing) {
        if let onSettings { GlassIconButton(icon: .settings, label: t("session.settings"), action: onSettings) }
      }
      .padding(.horizontal, Metrics.pad)
      .frame(maxWidth: .infinity, minHeight: Metrics.largeTitle, alignment: .leading)
  }
}

/// One session, full width, one fact per block: the agent's mark centred on both lines; the title, with the time at
/// its right end where no day caption says it; under it the place (the project, or inside a project's group the
/// agent); then the state's mark alone in a trailing column centred on both lines, the desktop's marks without a word
/// beside them
struct SessionRow<Pins: View>: View {
  let session: SessionSummary
  let agentName: String
  let place: String
  /// A pin before the title, where no "pinned" caption says it (one run, or pinned inside its project)
  var pinGlyph = false
  var time: String?
  var divider = true
  var onSelect: () -> Void
  var onRename: () -> Void = {}
  var onDelete: () -> Void = {}
  /// The pin actions the row offers (the list's top; inside a project's group, the project's own too)
  @ViewBuilder var pins: () -> Pins

  var body: some View {
    Button(action: onSelect) {
      // Marks sit centred on both lines: a mark on the title line read as hanging off the block
      HStack(alignment: .center, spacing: Metrics.gap) {
        // The bare mark, its ink fitted and centred (logos sit off-centre in their own boxes)
        VendorMark(brand: session.agent, name: agentName, size: Metrics.rowMark, fit: true)
          .foregroundStyle(Palette.fg2)
          .frame(width: Metrics.lead, height: Metrics.lead)
        VStack(alignment: .leading, spacing: Metrics.s0_5) {
          HStack(alignment: .firstTextBaseline, spacing: Metrics.s2) {
            if pinGlyph { Icon(.pin, size: Metrics.chevron).foregroundStyle(Palette.fg3) }
            Text(web: session.title).lineLimit(1).truncationMode(.tail)
              .textStyle(TypeScale.field)
              .foregroundStyle(Palette.fgStrong)
              .frame(maxWidth: .infinity, alignment: .leading)
            if let time {
              Text(web: time, paragraph: false)
                .textStyle(TypeScale.text3).monospacedDigit().foregroundStyle(Palette.fg3).fixedSize()
            }
          }
          Text(web: place, paragraph: false).lineLimit(1).truncationMode(.middle)
            .frame(maxWidth: .infinity, alignment: .leading)
            .textStyle(TypeScale.text3)
            .foregroundStyle(Palette.fg3)
        }
        // The state is its mark alone, as on the desktop (its meaning is the mark's accessibility label), in a column
        // of its own centred on both lines like the lead. Every row keeps the slot, so the times above line up
        Group {
          if let state = session.state, state != .unknown { StateMark(state: state, unread: session.unread == true) }
        }
        .frame(width: Metrics.stateSlot)
      }
      .padding(.horizontal, Metrics.pad)
      .frame(minHeight: Metrics.sessionRow)
      .overlay(alignment: .bottom) {
        if divider {
          // From the text column, under the lead
          Rectangle().fill(Palette.line).frame(height: Metrics.hairline)
            .padding(.leading, Metrics.pad + Metrics.lead + Metrics.gap)
        }
      }
      .contentShape(Rectangle())
    }
    .buttonStyle(ListRowStyle(active: false))
    .contextMenu {
      pins()
      Button(t("common.rename"), systemImage: "pencil", action: onRename)
      Button(t("common.delete"), systemImage: "trash", role: .destructive, action: onDelete)
    }
  }
}

/// A project's header in the by-project view: the folder in the marks' column, the name (a pin after it when pinned
/// to the top) and a chevron turning down while open; folded, the project's most pressing state sits in the rows'
/// state column. A tap folds it open or shut
struct ProjectHeader: View {
  let cwd: String
  let pinned: Bool
  let open: Bool
  let sessions: [SessionSummary]
  var onToggle: () -> Void
  var onPin: () -> Void
  var onNew: () -> Void

  var body: some View {
    Button(action: onToggle) {
      HStack(spacing: Metrics.gap) {
        // The folder in the session marks' column, the system's filled folder so it weighs what the filled logos
        // weigh. A pin never takes its place (it says "pinned", not "project")
        Image(systemName: "folder.fill").resizable().scaledToFit()
          .frame(width: Metrics.iconCtl, height: Metrics.iconCtl)
          .foregroundStyle(Palette.fg2)
          .frame(width: Metrics.lead)
        HStack(spacing: Metrics.s1_5) {
          Text(web: projectName(cwd), paragraph: false).lineLimit(1).truncationMode(.tail)
            .textStyle(TypeScale.field, weight: .semibold)
            .foregroundStyle(Palette.fgStrong)
          if pinned { Image(systemName: "pin.fill").font(.system(size: Metrics.chevron)).foregroundStyle(Palette.fg3) }
          Icon(.chevronRight, size: Metrics.chevron, strokeWidth: 2).foregroundStyle(Palette.fg3)
            .rotationEffect(.degrees(open ? 90 : 0))
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        // Open, each row shows its own state
        Group {
          if !open, let mark = Self.pressing(sessions) { StateMark(state: mark.state, unread: mark.unread) }
        }
        .frame(width: Metrics.stateSlot)
      }
      .padding(.horizontal, Metrics.pad)
      .frame(minHeight: Metrics.sheetRow)
      .contentShape(Rectangle())
    }
    .buttonStyle(ListRowStyle(active: false))
    .accessibilityAddTraits(.isHeader)
    .contextMenu {
      Button(pinned ? t("common.unpin") : t("common.pin"), systemImage: pinned ? "pin.slash" : "pin", action: onPin)
      Button(t("session.project.newSession"), systemImage: "square.and.pencil", action: onNew)
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

/// "N more" under an open project's latest few, in the rows' text column; a tap shows the rest in place
struct MoreRow: View {
  let count: Int
  var action: () -> Void

  var body: some View {
    Button(action: action) {
      HStack(spacing: Metrics.gap) {
        Color.clear.frame(width: Metrics.lead, height: Metrics.s1)
        Text(web: t("session.showMore", ["n": String(count)]), paragraph: false)
          .textStyle(TypeScale.text2).foregroundStyle(Palette.fg3)
        Spacer(minLength: 0)
      }
      .padding(.horizontal, Metrics.pad)
      .frame(minHeight: Metrics.sheetRow)
      .contentShape(Rectangle())
    }
    .buttonStyle(ListRowStyle(active: false))
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
/// meanwhile. Motion off stops all but the ring on their resting frame (full dot, lit dot, orbit dot at the top)
struct StateMark: View {
  let state: SessionState
  var unread = false
  @Environment(\.motionEnabled) private var motion

  var body: some View {
    let moving = motion || state == .working
    TimelineView(.animation(paused: !moving || state == .unknown)) { context in
      glyph(moving ? context.date.timeIntervalSinceReferenceDate : nil)
    }
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

extension View {
  /// No navigation bar: the list's large title is its first row
  func hiddenNavigationBar() -> some View {
    #if os(iOS)
    toolbar(.hidden, for: .navigationBar)
    #else
    self
    #endif
  }
}
