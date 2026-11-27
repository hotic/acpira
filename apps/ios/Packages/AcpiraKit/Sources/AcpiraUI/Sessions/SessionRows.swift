import AcpiraDesign
import AcpiraFixtures
import AcpiraModels
import SwiftUI

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

enum SectionPrompt {
  case create(project: String?)
  case rename(SessionSection)

  var title: String {
    switch self {
    case .create: t("session.section.new")
    case .rename: t("common.rename")
    }
  }
}

/// A custom project section's caption in the by-project view: taps fold or unfold its projects, a long press renames,
/// reorders or deletes it
struct SectionCaptionHeader: View {
  let section: SessionSection
  let open: Bool
  let canMoveUp: Bool
  let canMoveDown: Bool
  var onToggle: () -> Void
  var onRename: () -> Void
  var onMoveUp: () -> Void
  var onMoveDown: () -> Void
  var onDelete: () -> Void

  var body: some View {
    Button(action: onToggle) {
      HStack(spacing: Metrics.s1_5) {
        Text(web: section.name.isEmpty ? t("session.section.untitled") : section.name, paragraph: false)
          .lineLimit(1)
          .truncationMode(.tail)
          .textStyle(TypeScale.text3, weight: .medium)
          .foregroundStyle(Palette.fg3)
        Icon(.chevronRight, size: Metrics.chevron, strokeWidth: 2)
          .foregroundStyle(Palette.fg3)
          .rotationEffect(.degrees(open ? 90 : 0))
        Spacer(minLength: 0)
      }
      .padding(.horizontal, Metrics.pad)
      .frame(maxWidth: .infinity, minHeight: Metrics.groupCaption, alignment: .bottomLeading)
      .padding(.bottom, Metrics.s1)
      .contentShape(Rectangle())
    }
    .buttonStyle(.plain)
    .accessibilityAddTraits(.isHeader)
    .contextMenu {
      Section {
        Button(t("common.rename"), systemImage: "pencil", action: onRename)
        if canMoveUp { Button(t("session.section.moveUp"), systemImage: "arrow.up", action: onMoveUp) }
        if canMoveDown { Button(t("session.section.moveDown"), systemImage: "arrow.down", action: onMoveDown) }
      }
      Section {
        Button(t("session.section.delete"), systemImage: "trash", role: .destructive, action: onDelete)
      }
    }
  }
}

/// One of the user's sections with the projects of it the list shows
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
        // Leading in the sidebar as on the phone: the system's toggle stands alone at the column's trailing end
        if let filter { ToolbarItem(placement: .topBarLeading) { filter } }
        if !inSidebar {
          ToolbarItem(placement: .topBarTrailing) {
            Button(action: onSettings) { Image(systemName: "gearshape") }
              .barButtonBase()
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
  var onNewSection: () -> Void = {}
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
        sectionMenu
        Button(t("session.project.copyPath"), systemImage: "doc.on.doc", action: onCopyPath)
      }
      Section { Button(t("session.project.remove"), systemImage: "minus.circle", action: onRemove) }
    }
  }

  /// The sections as a checked picker, the folder's own ticked, then "New section…" and "Remove from section"
  private var sectionMenu: some View {
    let current = sections.first { $0.projects.contains(cwd) }?.id
    return Menu {
      if !sections.isEmpty {
        Picker(selection: Binding(get: { current }, set: { if let id = $0, id != current { onSection(id) } })) {
          ForEach(sections) { section in
            Text(verbatim: section.name.isEmpty ? t("session.section.untitled") : section.name).tag(Optional(section.id))
          }
        } label: {
          EmptyView()
        }
        .pickerStyle(.inline)
      }
      Section {
        Button(t("session.section.newEllipsis"), systemImage: "plus", action: onNewSection)
        if current != nil { Button(t("session.section.remove"), systemImage: "minus.circle") { onSection(nil) } }
      }
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

/// A host folder's header end (SessionListSections.tsx GroupHeader): the host's alias, then its connection as a dot:
/// green connected, amber failed, breathing while connecting, faint when off. No dot until the engine has said how the
/// host is (an older gateway keeps the hosts from devices)
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

/// SessionListSections.tsx MoreRow, under an open project's latest few. It tells what the rest holds: how many, the
/// agents among them (their marks, the first few) and the most urgent state inside, on the rows' state column. A tap
/// brings out the next page above it; with all of them out it reads "show less", the chevron turned over, and a tap
/// folds them away
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

/// The empty chats group's only row (SessionListSections.tsx EmptyProjectRow, flush): a faint plus in the marks' column
/// and "New chat"
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

/// The end of a pinned run: the desktop's short faded rule (SessionListSections.tsx `PinDivider`) instead of a pin on
/// each row, which pushed the row's title off the text line
struct PinRule: View {
  var body: some View {
    LinearGradient(colors: [.clear, Palette.line, .clear], startPoint: .leading, endPoint: .trailing)
      .frame(height: Metrics.hairline)
      .padding(.horizontal, Metrics.pad)
      .padding(.vertical, Metrics.s1)
  }
}

/// The pressed fill only. On the phone rows run edge to edge, so no radius; in the sidebar the fill is the desktop
/// column's: rounded and held in from both edges, where a square one ran into the column's edge
struct ListRowStyle: ButtonStyle {
  var active: Bool
  @Environment(\.listInSidebar) private var inSidebar
  func makeBody(configuration: Configuration) -> some View {
    let fill = active ? Palette.active : configuration.isPressed ? Palette.hover : .clear
    configuration.label
      .background {
        if inSidebar {
          RoundedRectangle(cornerRadius: Metrics.rLg, style: .continuous).fill(fill).padding(.horizontal, Metrics.s2)
        } else {
          Rectangle().fill(fill)
        }
      }
  }
}

/// The desktop list's state marks (packages/ui chat/SessionRows.tsx StateMark, motion in styles/motion.css), one slot
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
    glyph(moving: !held && (motion || state == .working))
      .breathing(held)
      .frame(width: Metrics.stateSlot, height: Metrics.stateSlot)
      .accessibilityElement()
      .accessibilityLabel(Text(web: t(state == .background && unread ? "session.state.backgroundUnread" : "session.state.\(state.rawValue)")))
  }

  /// The mark in its loop (`Loop`: an animation, so a list at rest costs the main thread nothing), or on its resting
  /// frame. The two are separate branches, so a mark that starts moving starts its loop afresh
  @ViewBuilder
  private func glyph(moving: Bool) -> some View {
    switch state {
    case .working:
      // animate-spin: one turn a second, linear
      let ring = Icon(.loaderCircle, size: Metrics.stateRing, strokeWidth: 2.5).foregroundStyle(Palette.fg2)
      if moving {
        Loop(period: 1) { ring } move: { ring, far in ring.rotationEffect(.degrees(far ? 360 : 0)) }
      } else {
        ring
      }
    case .waiting:
      let dot = Circle().fill(Palette.warn).frame(width: Metrics.stateDot, height: Metrics.stateDot)
      if moving {
        Loop(period: 1.4, shape: Self.beat) { dot } move: { dot, far in dot.scaleEffect(far ? 2 : 1) }
      } else {
        dot
      }
    case .error:
      let dot = Circle().fill(Palette.danger).frame(width: Metrics.stateDot, height: Metrics.stateDot)
      if moving {
        Loop(period: 3, shape: Self.flash) { dot } move: { dot, far in dot.opacity(far ? 0 : 1) }
      } else {
        dot
      }
    case .unread:
      let dot = Circle().fill(Palette.fgStrong).frame(width: Metrics.stateUnread, height: Metrics.stateUnread)
      ZStack {
        // The halo: a copy of the dot growing to 2.6 times while it fades, ease-out over 2 s; invisible at rest
        if moving {
          Loop(period: 2, shape: .easeOut) { dot } move: { dot, far in dot.scaleEffect(far ? 2.6 : 1).opacity(far ? 0 : 0.6) }
        }
        dot
      }
    case .background:
      ZStack {
        Icon(.circle, size: Metrics.stateRing, strokeWidth: 2.5).foregroundStyle(unread ? Palette.fgStrong : Palette.fg3)
        // The orbit rides the ring's own line (Lucide's circle: r 10 in a 24-unit box), one turn in 3.6 s
        let orbit = Circle().fill(unread ? Palette.fgStrong : Palette.fg2)
          .frame(width: Metrics.stateOrbit, height: Metrics.stateOrbit)
          .offset(y: -Metrics.stateRing * 10 / 24)
        if moving {
          Loop(period: 3.6) { orbit } move: { orbit, far in orbit.rotationEffect(.degrees(far ? 360 : 0)) }
        } else {
          orbit
        }
        if unread { Circle().fill(Palette.fgStrong).frame(width: Metrics.stateInner, height: Metrics.stateInner) }
      }
    case .unknown:
      EmptyView()
    }
  }

  /// acp-mark-beat, as growth over the resting dot: 0 / 100% 1, 20% 1.5, 40% 1.1, 55% 1.35, 75% 1, eased in and out
  /// between keys
  private static let beat = LoopShape.keys([LoopKey(0, 0), LoopKey(0.2, 0.5), LoopKey(0.4, 0.1), LoopKey(0.55, 0.35),
                                            LoopKey(0.75, 0), LoopKey(1, 0)], eased: true)

  /// acp-mark-flash, as the light lost: lit, then two dips to 0.2 at 70% and 82%, linear between keys
  private static let flash = LoopShape.keys([LoopKey(0, 0), LoopKey(0.64, 0), LoopKey(0.7, 0.8), LoopKey(0.76, 0),
                                             LoopKey(0.82, 0.8), LoopKey(0.88, 0), LoopKey(1, 0)], eased: false)
}
