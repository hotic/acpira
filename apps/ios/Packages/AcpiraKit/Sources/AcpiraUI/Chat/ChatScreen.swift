import AcpiraDesign
import AcpiraFixtures
import AcpiraModels
import Combine
import SwiftUI
#if canImport(UIKit)
import UIKit
#endif

// chat/Shell.tsx at phone width: the header bar, the thread (exchanges whose prompt card sticks within its own
// exchange) with the live to-do card floating over its bottom, and the dock (Dock.swift).

/// One session, live: its transcript and dock bound to the app model, every control wired to it
struct ChatScreen: View {
  let id: String
  @Environment(AppModel.self) private var model
  /// The bottom safe area (the home indicator), read before the dock takes part of it over
  @State private var homeInset: CGFloat = 0
  @State private var renaming = false
  /// A composer picker is out: its dim covers the page up to the screen's top, and the bar steps aside under it
  @State private var picking = false
  @Environment(\.motionEnabled) private var motion
  @State private var newTitle = ""
  @State private var deleting = false
  /// The move menu's "Add folder" alert
  @State private var addingFolder = false
  @State private var accountOpen = false
  /// The closures the chat hands its views, made once per session and app model (`Memo`): a reconnect swaps the model
  @State private var actionsMemo = Memo<SessionActions>()
  @State private var diffMemo = Memo<((DiffLocation) async -> FullDiffSource?)?>()
  /// The header's closures and the draft's bindings, made once per session, app model (and draft) for the same reason:
  /// a fresh binding or closure never compares equal, so the dock and the navigation bar were rebuilt on every chunk
  @State private var headerMemo = Memo<HeaderActions>()
  @State private var draftMemo = Memo<DraftBindings>()
  @Environment(\.initialPopup) private var initialPopup
  @Environment(\.initialDraft) private var initialDraft
  @Environment(\.syncing) private var syncing

  var body: some View {
    Group {
      if let session = model.views[id] {
        screen(session)
      } else {
        // The list arrives before the transcript, which can take a while over a paired link. There is no byte
        // progress on this wire, so keep an indeterminate indicator until the session itself arrives.
        ProgressView {
          Text(web: t("session.import.loading")).textStyle(TypeScale.text2).foregroundStyle(Palette.fg3)
        }
        .tint(Palette.fg2)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Palette.bg0.ignoresSafeArea())
        .navigationTitle("")
        .navigationBarTitleDisplayModeInline()
        .backWithoutTitle()
      }
    }
    // Coming on screen, pushed or popped back to, switches the engine's view to this session
    .onAppear { model.open(id) }
    .onDisappear { model.parkDraft(id) }
  }

  private func screen(_ session: SessionView) -> some View {
    let agent = model.agent(session.agent)
    let summary = model.summaries.first { $0.id == id }
    let pinned = summary?.pinned == true
    // The unsent prompt and attachments live in the app model, so leaving the chat and coming back keeps them
    let draft = model.draft(for: id)
    // A prompt the session itself holds back (starting, replaying remembered controls) shows as sent (HeldPrompt.swift)
    // and so does the phone's own prompt still on its way to the engine; its working row says which stage holds it
    let sent = model.outgoing[id] ?? []
    let held = heldPrompt(session, outgoing: sent)
    let bindings = draftMemo("\(memoKey):\(ObjectIdentifier(draft).hashValue)") { DraftBindings(draft) }
    return Thread(session: held)
      .modifier(StartingTitle(active: held.status == .starting && !held.running && held.external == nil,
                              agent: held.agent, aside: picking))
      .environment(\.liveStage, heldStage(session, outgoing: sent, agent: agent.name))
      // An inset reserves scroll clearance without clipping the transcript at the composer's rectangular top edge.
      // Messages pass behind the glass and remain visible beside its rounded corners, as under the navigation bar.
      .safeAreaInset(edge: .bottom, spacing: 0) {
        DockSeat(homeInset: homeInset) {
          // The dock's copy of the session leaves out the streamed transcript, so a chunk does not rebuild the dock
          Dock(session: dockSession(session), turns: session.turns.count, draft: bindings.text, staged: bindings.staged,
               edit: bindings.edit)
            .readableWidth()
        }
        // The inset takes the dock's ideal height, including long drafts and attachments above the keyboard.
        .fixedSize(horizontal: false, vertical: true)
      }
      .scrollEdgeEffectHiddenIfAvailable(for: .bottom)
    // The keyboard's safe area still lifts the dock; only the container's (the home indicator) is taken over
    .ignoresSafeArea(.container, edges: .bottom)
    .modifier(HomeInset(inset: $homeInset))
    // The thread fades into the page as it scrolls up under the bar's glass buttons. Not while a picker is out: the
    // fade lies over the composer too, and a column reaching the bar came out washed in the page's colour
    .topFade(hidden: picking)
    .onAppear {
      if let initialDraft, draft.text.isEmpty { draft.text = initialDraft }
      // `-popup account`: the account sheet open on launch (screenshots); `-popup toast`: a moment later, the toast of
      // switching to the agent's first account
      if initialPopup == "account" { accountOpen = true }
      if initialPopup == "quotes" { model.seedQuotes(id) }
      // `staged`, `staged.expand`, and any popup ending in `.staged` (`focus.model.staged`) seed the staged strip
      if let popup = initialPopup, popup == "staged" || popup == "staged.expand" || popup.hasSuffix(".staged") {
        model.seedStaged(id)
      }
      if initialPopup == "edit" { model.beginEditLast(id) }
      // `-popup sidekick.demo` (recordings): the sidekick's round replayed from its handoff
      if initialPopup == "sidekick.demo" { Task { await model.replaySidekick(id) } }
      if initialPopup == "toast", let account = model.accounts.first(where: { $0.agent == agent.id }) {
        Task {
          try? await Task.sleep(for: .seconds(1))
          model.selectAccount(id, account.id)
        }
      }
    }
    // The composer's liquid pickers, drawn over the whole screen so every piece takes taps
    .gooLayer()
    // The bar's items leave as the beans come out and come back as the dim lifts (the new session's hero)
    .onPreferenceChange(ComposerPickerKey.self) { now in
      withAnimation(motion ? now ? .easeOut(duration: Motion.durOpen) : Motion.gooDimOut : nil) { picking = now }
    }
    .modifier(SystemChatBar(pinned: pinned, unread: summary.map(isUnread) ?? false, archived: summary?.archived == true,
                            aside: picking, actions: headerActions(session, agent)))
    // Under the keyboard too: its rounded corners show the page's ground, not the hosting view's black (a colour's own
    // background ignores the container's safe area only)
    .background(Palette.bg0.ignoresSafeArea())
    .environment(\.agentAccent, Palette.accent(for: session.agent))
    .environment(\.chatAgent, agent)
    // A view carried over from the wire before stays syncing until this wire sends the session again
    .environment(\.syncing, syncing || !model.shown(id))
    .environment(\.sessionActions, actionsMemo(memoKey) { sessionActions })
    // A subagent row pushes that child's inspector
    .environment(\.inspectSubagent, SubagentInspect(session: id) { model.path.append(.subagent(session: id, id: $0)) })
    .modifier(MediaLayer(session: id, loader: model.media))
    .environment(\.mentionPersonas, model.personas)
    .environment(\.promptCommands, session.commands.map(\.name))
    .environment(\.compactAt, model.compactAt)
    .environment(\.hiddenModels, model.settings?.hiddenOptions[session.agent] ?? [:])
    .environment(\.diffSource, diffMemo("\(memoKey):\(model.live)") { model.live ? { [model, id] in await model.diffSource(id, $0) } : nil })
    .environment(model.fileSearch)
    // The accounts as a system sheet with a form
    .sheet(isPresented: $accountOpen) {
      NativeAccountSheet(agent: agent, accounts: model.accounts.filter { $0.agent == agent.id }, accountId: session.accountId,
                         onSelect: { model.selectAccount(id, $0) },
                         onAdd: { model.addAccount(agent.id) },
                         onRemove: { model.removeAccount($0) })
        // The quotas are fetched when an account list comes into view, as on the desktop
        .onAppear { model.refreshQuota(agent.id) }
    }
    .alert(t("common.rename"), isPresented: $renaming) {
      TextField(text: $newTitle) { Text(web: t("session.titleAria")) }
      Button(t("mobile.cancel"), role: .cancel) {}
      Button(t("mobile.save")) { model.rename(id, to: newTitle) }
    }
    .confirmationDialog(t("mobile.deleteConfirm", ["title": session.title]), isPresented: $deleting, titleVisibility: .visible) {
      Button(t("common.delete"), role: .destructive) { model.delete(id) }
      Button(t("mobile.cancel"), role: .cancel) {}
    }
    // As in the new session's folder menu, a typed folder joins the project list; the session moves into it
    .modifier(AddProjectPicker(isPresented: $addingFolder) { folder in
      model.addProject(folder)
      model.move(id, to: folder)
    })
  }

  private func helpers(_ session: SessionView) -> [SubagentSummary] { (session.subagents ?? []).filter { !isSidekick($0) } }

  private func headerActions(_ session: SessionView, _ agent: AgentInfo) -> HeaderActions {
    var actions = headerMemo(memoKey) { headerClosures }
    // The sidekick's rounds are the lead's own relay, not subagents: they neither offer the graph nor count in it
    let helpers = helpers(session)
    if !hasAccountMenu(agent) { actions.account = nil }
    if helpers.isEmpty { actions.subagents = nil }
    if session.cwd.isEmpty { actions.changes = nil }
    actions.subagentCounts = countsLabel(helpers)
    actions.cwd = session.cwd
    actions.projects = model.projects
    actions.chat = model.isChat(session.cwd)
    return actions
  }

  /// Every header action, made once per session and app model (`headerMemo`): each reads the session when it runs
  /// rather than capturing the one on screen when the bar was built, so the bar compares equal across streamed chunks
  private var headerClosures: HeaderActions {
    HeaderActions(
      back: { if !model.path.isEmpty { model.path.removeLast() } },
      new: {
        guard let session = model.views[id] else { return }
        model.path.append(.new(agent: session.agent, cwd: session.cwd))
      },
      account: { accountOpen = true },
      pin: { model.togglePin(id) },
      unread: { model.setUnread(id, $0) },
      // An archived chat leaves the list; going back shows where it went
      archive: { archived in
        model.setArchived(id, archived)
        if archived && !model.path.isEmpty { model.path.removeLast() }
      },
      rename: { newTitle = model.views[id]?.title ?? ""; renaming = true },
      export: { model.export(id, format: $0) },
      terminal: { if let cwd = model.views[id]?.cwd { model.openTerminal(cwd) } },
      subagents: { model.path.append(.subagents(session: id)) },
      settings: { model.settingsOpen = true },
      delete: { deleting = true },
      changes: { model.path.append(.changes(id)) },
      copy: { kind in Task { await model.copy(id, kind) } },
      move: { model.move(id, to: $0) },
      addFolder: { addingFolder = true })
  }

  /// What the memoised closures are bound to: this session on this app model
  private var memoKey: String { "\(ObjectIdentifier(model).hashValue):\(id)" }

  private var sessionActions: SessionActions {
    SessionActions(
      send: { model.send(id, text: $0, attachments: $1) },
      stop: { model.stop(id) },
      permission: { model.choosePermission(id, block: $0, option: $1) },
      answer: { model.answer(id, question: $0, answers: $1, skipped: $2) },
      fork: { model.fork(id, through: $0) },
      toast: { model.show($0, icon: $1) },
      sendQueued: { model.sendQueued(id, item: $0) },
      editQueued: { item in
        // Edit takes the prompt out of the queue and back into the composer, ahead of whatever is being typed there
        // rather than over it
        Task {
          guard let prompt = await model.editQueued(id, item: item) else { return }
          let draft = model.draft(for: id)
          let typed = draft.text.trimmingCharacters(in: .whitespacesAndNewlines)
          draft.text = typed.isEmpty ? prompt.text : prompt.text + "\n\n" + draft.text
          draft.staged = (prompt.attachments ?? []) + draft.staged
        }
      },
      removeQueued: { model.takeQueued(id, $0) },
      retry: { model.retry(id) },
      carryOn: { model.carryOn(id) },
      setMode: { model.setMode(id, $0) },
      setConfig: { model.setConfig(id, $0, $1) },
      buildPlan: { model.buildPlan(id, plan: $0, option: $1, model: $2) },
      compact: { model.compact(id) },
      newSession: { model.continueNew(id) },
      restart: { model.restart(id) },
      login: { model.login(id, method: $0) },
      takeOver: { model.takeOver(id) },
      failureAction: { model.failureAction(id, $0) },
      quote: { model.quote(id, $0) },
      editPrompt: { model.beginEdit(id, turn: $0) },
      sendEdit: { model.sendEdit(id, text: $0, attachments: $1) },
      cancelEdit: { model.cancelEdit(id) },
      goal: { model.goal(id, $0, objective: $1) },
      stopTask: { model.stopTask(id, task: $0) })
  }
}

// MARK: - Header

/// What the header's buttons and its "…" menu do; `account` is nil when the agent has no account menu. The session list's
/// row menu is the same menu with the chat's own entries (account, changes, subagents, terminal, settings) left nil
struct HeaderActions {
  var back: () -> Void = {}
  var new: () -> Void = {}
  var account: (() -> Void)?
  var pin: () -> Void = {}
  /// "Mark as unread / read"; nil where it is not offered
  var unread: ((Bool) -> Void)?
  /// "Archive / unarchive"; nil where it is not offered
  var archive: ((Bool) -> Void)?
  var rename: () -> Void = {}
  var export: (ExportFormat) -> Void = { _ in }
  /// JSON export is offered: it is built from the view the phone holds, which a list row's session may not have
  var exportJSON = true
  /// The project's terminal page
  var terminal: (() -> Void)?
  /// The session's subagent graph page; nil while the session has no subagents. `subagentCounts` is its subtitle
  var subagents: (() -> Void)?
  var subagentCounts = ""
  var settings: (() -> Void)?
  /// Nil in a list row's menu, where the row's swipe deletes
  var delete: (() -> Void)?
  /// The files the session changed (panel.changes)
  var changes: (() -> Void)?
  var copy: (SessionCopy) -> Void = { _ in }
  /// The session's folder and the others it can move to
  var cwd = ""
  var projects: [String] = []
  /// The session is a chat: its own folder is no project to tick in the move menu
  var chat = false
  var move: ((String) -> Void)?
  /// "Add folder" in the move menu: the screen raises its path alert, then moves the session there
  var addFolder: (() -> Void)?
}

/// The chat's navigation bar: the system back button on the left, new session and the "…" menu as bar buttons on the
/// right (the system groups them on one glass capsule), no title: the thread scrolls under the glass
struct SystemChatBar: ViewModifier {
  var pinned: Bool
  var unread = false
  var archived = false
  /// A composer picker is out: the items leave the bar (`barAside`)
  var aside = false
  var actions: HeaderActions

  func body(content: Content) -> some View {
    content
      .navigationTitle("")
      .navigationBarTitleDisplayModeInline()
      .backWithoutTitle()
      .barAside(aside)
      .toolbar {
        if !aside {
          ToolbarItemGroup(placement: .primaryAction) {
            // The pencil's ink sits low in its box: lifted, it reads centred in the glass
            Button(action: actions.new) { Image(systemName: "square.and.pencil").offset(y: -Metrics.pencilLift) }
              .accessibilityLabel(Text(web: t("session.new")))
            Menu { NativeSessionMenu(pinned: pinned, unread: unread, archived: archived, actions: actions) } label: { Image(systemName: "ellipsis") }
              .accessibilityLabel(Text(web: t("session.more")))
          }
        }
      }
  }
}

// MARK: - Thread

/// The transcript, following its end the way chat apps do: the end stays in view while the reader is on it (new
/// output, the keyboard, a dock growing), a drag away lets go of it and a drag that ends near it takes it up again.
/// Reading history stays put whatever arrives below. On iOS the rows sit in a collection view (`TranscriptCollection`)
struct Thread: View {
  let session: SessionView
  /// `-open`: folds pre-opened in the last exchange, as in the full-length render
  @Environment(\.preOpen) private var preOpen
  @Environment(\.motionEnabled) private var motion
  @Environment(AppModel.self) private var model: AppModel?
  @State private var gate = QuietGate()
  #if os(iOS)
  @State private var control = TranscriptControl()
  #endif

  var body: some View {
    #if os(iOS)
    let stream = model?.liveSessions.contains(session.id) == true
    let quiet = gate.isQuiet(session: session, catchingUp: false, liveStream: stream)
    let control = control
    let session = session
    // The collection view spans the bars and docks and scrolls behind their glass; how far it reaches past the safe
    // area on each side (the bars above, the docks below) is its content inset
    // The reader keeps to the safe frame (SwiftUI reports a view that ignores the safe area at its safe frame while its
    // platform view still spans the bars). Read in the reader's body rather than copied into state, so the collection
    // view gets it in the transaction that moved it and can tell an animated dock from one that moves at once
    GeometryReader { proxy in
      TranscriptCollection(items: threadItems(session, preOpen: preOpen), safe: proxy.frame(in: .global), control: control,
                           running: session.running, motion: motion) { values in
        values.applyThread(session, quiet: quiet)
        values.holdScroll = control.holdScroll
        values.scrollGesture = control.gesture
        values.threadFollowing = control.following
        values.foldMemory = control.folds
      }
      .ignoresSafeArea(.container)
    }
    // Another session is another thread: its rows, offset and follow start afresh
    .id(session.id)
    // Inside the plan dock's inset, so it floats above the plan card and the composer
    .overlay(alignment: .bottomTrailing) {
      ScrollToLatest(far: control.far, hidden: session.running && control.following.value) {
        control.toLatest(animated: motion)
      }
      .padding(.trailing, Metrics.pad)
      .padding(.bottom, Metrics.s2)
      // On the composer's right edge, above the send button
      .frame(maxWidth: .infinity, alignment: .trailing)
      .readableWidth()
    }
    // The docked plan floats over the thread's bottom; the inset is also the transcript's scroll clearance
    .safeAreaInset(edge: .bottom, spacing: 0) { ThreadDock(session: session).readableWidth() }
    #else
    ScrollView(.vertical) { ThreadContent(session: session, preOpen: preOpen) }
      .defaultScrollAnchor(.bottom)
      .safeAreaInset(edge: .bottom, spacing: 0) { ThreadDock(session: session).readableWidth() }
    #endif
  }
}

/// A scroll view's last content offset (`scrollTo(y:)`'s measure: under the content's top inset), kept outside SwiftUI's
/// state so scrolling does not redraw (the subagent inspector's fold hold)
final class ScrollOffset: @unchecked Sendable {
  var y: CGFloat = 0
}

extension ScrollGeometry {
  /// How far the content runs on below the part of the frame the bottom insets leave clear (0 at the end).
  /// `contentOffset` is UIKit's raw offset from the frame's top, while `containerSize` already excludes both insets:
  /// adding the container to the offset and taking the bottom inset off counted that inset twice, so a thread resting
  /// on its end read as a dock's height above it and never resumed following after a drag back down
  var gapBelow: CGFloat { contentSize.height - (contentOffset.y + contentInsets.top + containerSize.height) }
}

/// A flag one view writes and another observes, so the writer never redraws when it flips
@Observable final class ObservedFlag {
  var value: Bool

  init(_ value: Bool = false) { self.value = value }
}

extension EnvironmentValues {
  /// Whether the thread follows its end (`Thread.following`); nil outside a thread, which counts as following
  @Entry var threadFollowing: ObservedFlag?
}

/// Back to the latest message: a small glass drop on the composer's right edge, there while the thread is more than
/// half a screen above its end. ChatGPT centres a similar arrow; on the right it sits under the thumb and clear of the
/// prose. Hidden while the thread follows a running turn, where streamed growth briefly opens a gap the follow closes
struct ScrollToLatest: View {
  let far: ObservedFlag
  var hidden: Bool
  var action: () -> Void
  @Environment(\.motionEnabled) private var motion

  var body: some View {
    let shown = far.value && !hidden
    GlassGroup {
      if shown {
        Button(action: action) {
          Icon(.arrowDown, size: Metrics.icon)
            .foregroundStyle(Palette.fg1)
            .frame(width: Metrics.ctlSm, height: Metrics.ctlSm)
            .contentShape(Circle())
        }
        .buttonStyle(.plain)
        // The composer's glass, frosted enough for prose passing behind
        .glassSurface(GlassStyle.bar.interactive(), in: Circle())
        // Glass cannot fade; the system's own materialise is how a drop of it comes and goes
        .glassMorph(.materialize)
        .accessibilityLabel(Text(web: t("mobile.scrollToLatest")))
      }
    }
    .animation(motion ? .smooth(duration: Motion.dur * 2) : nil, value: shown)
  }
}

/// Snapshot of the parts of a session that change as a turn streams, used by `QuietGate` and `AppModel.quietSessions`
/// to tell an initial or caught-up view apart from a live stream update
struct QuietSignature: Equatable {
  var rev: Int?
  var turnCount: Int
  var lastAgent: AgentTurn?
  var subagents: [SubagentSummary]?

  init(_ session: SessionView) {
    rev = session.rev
    turnCount = session.turns.count
    lastAgent = session.turns.reversed().lazy.compactMap { if case .agent(let t) = $0 { t } else { nil } }.first
    subagents = session.subagents
  }
}

/// Tracks whether the current pass restores history or applies the catch-up push after returning to a session
/// (`Shell.tsx` `opening` / `QuietEntranceContext`). Seeded into `EntranceLedger` so lazy rows and closed folds
/// mounted on later frames stay still too
final class QuietGate {
  private var sessionId: String?
  private var settled: QuietSignature?
  private var wasCatchingUp = false

  func isQuiet(session: SessionView, catchingUp: Bool, liveStream: Bool) -> Bool {
    let current = QuietSignature(session)
    if sessionId != session.id || catchingUp || wasCatchingUp || !liveStream {
      sessionId = session.id
      settled = current
      wasCatchingUp = catchingUp
      EntranceLedger.seed(session)
      return true
    }
    return current == settled
  }
}

/// The exchanges themselves; `lazy: false` lays every exchange out at once (a full-length render)
struct ThreadContent: View {
  let session: SessionView
  var lazy = true
  /// True while the caller is still waiting for the first transcript push (e.g. a subagent inspector before its first
  /// `subagent` message lands)
  var catchingUp = false
  /// True when the caller drives `session` from its own stream outside `AppModel.liveSessions` (e.g. the subagent
  /// inspector once its initial transcript has loaded)
  var liveStream: Bool?
  /// Folds opened in the last exchange before any tap (renders and previews)
  var preOpen: Set<PreOpen> = []
  @Environment(AppModel.self) private var model: AppModel?
  @State private var gate = QuietGate()

  var body: some View {
    let stream = liveStream ?? (model?.liveSessions.contains(session.id) == true)
    let quiet = gate.isQuiet(session: session, catchingUp: catchingUp, liveStream: stream)
    let items = threadItems(session, preOpen: preOpen)
    let lastExchange = exchanges(session.turns).last?.id
    Group {
      if lazy {
        // A prompt bubble does not stick (it would sit under the floating glass with the thread showing beside it).
        // The last exchange stays out of the lazy stack: the stack sizes the exchanges it has not laid out from the
        // ones it has, and a thread opened at the bottom lays out the last one first. A running turn's open process
        // is many screens tall, so every earlier exchange was taken for that tall, the bottom the thread opened at
        // lay far past the real content, and no row was there to draw: a blank chat until a drag
        VStack(alignment: .leading, spacing: 0) {
          LazyVStack(alignment: .leading, spacing: 0) {
            ForEach(items.filter { $0.exchange != lastExchange }, id: \.id) { ThreadItemView(item: $0) }
          }
          ForEach(items.filter { $0.exchange == lastExchange }, id: \.id) { ThreadItemView(item: $0) }
        }
      } else {
        VStack(alignment: .leading, spacing: 0) {
          ForEach(items, id: \.id) { ThreadItemView(item: $0) }
        }
      }
    }
    .padding(.top, Metrics.padY)
    .padding(.bottom, WebMetrics.msgGap)
    .transformEnvironment(\.self) { $0.applyThread(session, quiet: quiet) }
  }
}

// MARK: - Full-length render

/// The whole chat at phone width with the thread snapshot: every exchange, then the dock (the navigation bar is the
/// system's and does not render here). Rendered to a PNG by `-render`, so a transcript longer than the screen compares
/// against the LAB's full-page reference in one image
public struct ChatPoster: View {
  let session: SessionView
  var width: CGFloat
  var preOpen: Set<PreOpen>

  public init(session: SessionView, width: CGFloat, preOpen: Set<PreOpen>) {
    self.session = session; self.width = width; self.preOpen = preOpen
  }

  public var body: some View {
    let held = heldPrompt(session)
    VStack(spacing: 0) {
      ThreadContent(session: held, lazy: false, preOpen: preOpen)
      ThreadDock(session: held)
      Dock(session: session, draft: .constant(""), staged: .constant([]))
        .padding(.bottom, Metrics.page)
    }
    .frame(width: width)
    .fixedSize(horizontal: false, vertical: true)
    .background(Palette.bg0)
    .environment(\.liveStage, heldStage(session, agent: Fixtures.agents.first { $0.id == session.agent }?.name ?? session.agent))
    .environment(\.agentAccent, Palette.accent(for: session.agent))
    .environment(\.media, MediaScope(session: session.id))
    .environment(\.chatAgent, Fixtures.agents.first { $0.id == session.agent })
  }
}

/// Where a screen's dock rests: sunk into the home indicator's band with the keyboard down (`Metrics.barSink`), a
/// small gap over the keyboard with it up (the keyboard's safe area lifts it). The keyboard flag is this view's own
/// state, so a keyboard change redraws the dock alone, never the transcript or hero around it
struct DockSeat<Content: View>: View {
  var homeInset: CGFloat
  let content: Content
  @State private var keyboard = false
  @Environment(\.horizontalSizeClass) private var widthClass
  @Environment(\.motionEnabled) private var motion

  init(homeInset: CGFloat, @ViewBuilder content: () -> Content) {
    self.homeInset = homeInset
    self.content = content()
  }

  var body: some View {
    content
      // A regular width keeps the dock out of the home band, level with the sidebar's foot
      .padding(.bottom, keyboard ? Metrics.s2 : max(Metrics.s2, homeInset - (widthClass == .regular ? 0 : Metrics.barSink)))
      // On the composer's curve: the will-show notice lands a moment before the keyboard moves, and the dock (with a
      // thread at its end) dropped by the difference in one frame first
      .animation(motion ? Motion.composerMorph : nil, value: keyboard)
      .modifier(KeyboardWatch(up: $keyboard))
  }
}

/// The home indicator's band: the bottom safe area of a layer that ignores the keyboard, so the keyboard's height
/// never gets in. Read off the screen's own geometry it arrived with the keyboard a moment before the will-show
/// notification, and the dock padded by it inside the keyboard's animation: the chat dropped a keyboard's height (a
/// dark band over the top) and flew back up on every show, and the dock leapt to the top on every hide. Applied
/// outside `ignoresSafeArea(.container)`, where the inset is non-zero
struct HomeInset: ViewModifier {
  @Binding var inset: CGFloat

  func body(content: Content) -> some View {
    content.background {
      Color.clear
        .onGeometryChange(for: CGFloat.self) { $0.safeAreaInsets.bottom } action: { inset = $0 }
        .ignoresSafeArea(.keyboard)
    }
  }
}

/// A value made once per key and handed out again while the key holds. For closures a screen puts in the environment:
/// a fresh closure never compares equal, so a new one on every rebuild of the chat (each streamed chunk) told every
/// visible message that reads it that its inputs had changed, and redrew them all
final class Memo<Value> {
  private var made: (key: String, value: Value)?

  func callAsFunction(_ key: String, make: () -> Value) -> Value {
    if let made, made.key == key { return made.value }
    let value = make()
    made = (key, value)
    return value
  }
}

/// The draft's bindings the dock writes through, made once per draft (`ChatScreen.draftMemo`): a binding made on each
/// rebuild holds a new location and never compares equal
struct DraftBindings {
  let text: Binding<String>
  let staged: Binding<[Attachment]>
  let edit: Binding<TurnEdit?>

  init(_ draft: ComposerDraft) {
    let bindable = Bindable(draft)
    text = bindable.text
    staged = bindable.staged
    edit = bindable.edit
  }
}

/// Tracks the software keyboard from the system's will-show / will-hide notifications (no keyboard off iOS)
struct KeyboardWatch: ViewModifier {
  @Binding var up: Bool

  func body(content: Content) -> some View {
    #if canImport(UIKit)
    content
      .onReceive(NotificationCenter.default.publisher(for: UIResponder.keyboardWillShowNotification)) { _ in up = true }
      .onReceive(NotificationCenter.default.publisher(for: UIResponder.keyboardWillHideNotification)) { _ in up = false }
    #else
    content
    #endif
  }
}
