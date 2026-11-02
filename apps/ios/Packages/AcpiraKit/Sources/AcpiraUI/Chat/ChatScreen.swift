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
  @State private var newTitle = ""
  @State private var deleting = false
  /// The move menu's "Add folder" alert
  @State private var addingFolder = false
  @State private var accountOpen = false
  /// The closures the chat hands its views, made once per session and app model (`Memo`): a reconnect swaps the model
  @State private var actionsMemo = Memo<SessionActions>()
  @State private var diffMemo = Memo<((DiffLocation) async -> FullDiffSource?)?>()
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
    return Thread(session: held)
      .modifier(StartingTitle(session: held))
      .environment(\.liveStage, heldStage(session, outgoing: sent, agent: agent.name))
      // An inset reserves scroll clearance without clipping the transcript at the composer's rectangular top edge.
      // Messages pass behind the glass and remain visible beside its rounded corners, as under the navigation bar.
      .safeAreaInset(edge: .bottom, spacing: 0) {
        DockSeat(homeInset: homeInset) {
          Dock(session: session, draft: Bindable(draft).text, staged: Bindable(draft).staged, edit: Bindable(draft).edit)
            .readableWidth()
        }
        // The inset takes the dock's ideal height, including long drafts and attachments above the keyboard.
        .fixedSize(horizontal: false, vertical: true)
      }
      .scrollEdgeEffectHidden(true, for: .bottom)
    // The keyboard's safe area still lifts the dock; only the container's (the home indicator) is taken over
    .ignoresSafeArea(.container, edges: .bottom)
    .modifier(HomeInset(inset: $homeInset))
    // The thread fades into the page as it scrolls up under the bar's glass buttons
    .topFade()
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
    .modifier(SystemChatBar(pinned: pinned, unread: summary.map(isUnread) ?? false, archived: summary?.archived == true,
                            actions: headerActions(session, agent)))
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
    .modifier(MediaLayer(session: session, loader: model.media))
    .environment(\.mentionPersonas, model.personas)
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
    .modifier(AddProjectAlert(isPresented: $addingFolder) { folder in
      model.addProject(folder)
      model.move(id, to: folder)
    })
  }

  private func helpers(_ session: SessionView) -> [SubagentSummary] { (session.subagents ?? []).filter { !isSidekick($0) } }

  private func headerActions(_ session: SessionView, _ agent: AgentInfo) -> HeaderActions {
    HeaderActions(
      back: { if !model.path.isEmpty { model.path.removeLast() } },
      new: { model.path.append(.new(agent: session.agent, cwd: session.cwd)) },
      account: hasAccountMenu(agent) ? { accountOpen = true } : nil,
      pin: { model.togglePin(id) },
      unread: { model.setUnread(id, $0) },
      // An archived chat leaves the list; going back shows where it went
      archive: { archived in
        model.setArchived(id, archived)
        if archived && !model.path.isEmpty { model.path.removeLast() }
      },
      rename: { newTitle = session.title; renaming = true },
      export: { model.export(id, format: $0) },
      terminal: { model.openTerminal(session.cwd) },
      // The sidekick's rounds are the lead's own relay, not subagents: they neither offer the graph nor count in it
      subagents: helpers(session).isEmpty ? nil : { model.path.append(.subagents(session: id)) },
      subagentCounts: countsLabel(helpers(session)),
      settings: { model.settingsOpen = true },
      delete: { deleting = true },
      changes: session.cwd.isEmpty ? nil : { model.path.append(.changes(id)) },
      copy: { kind in Task { await model.copy(id, kind) } },
      cwd: session.cwd,
      projects: model.projects,
      chat: model.isChat(session.cwd),
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
        if let prompt = model.takeQueued(id, item) {
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
  var actions: HeaderActions

  func body(content: Content) -> some View {
    content
      .navigationTitle("")
      .navigationBarTitleDisplayModeInline()
      .backWithoutTitle()
      .toolbar {
        ToolbarItemGroup(placement: .primaryAction) {
          Button(action: actions.new) { Image(systemName: "square.and.pencil") }
            .accessibilityLabel(Text(web: t("session.new")))
          Menu { NativeSessionMenu(pinned: pinned, unread: unread, archived: archived, actions: actions) } label: { Image(systemName: "ellipsis") }
            .accessibilityLabel(Text(web: t("session.more")))
        }
      }
  }
}

// MARK: - Thread

/// The transcript: one section per exchange with the prompt card pinned while its exchange scrolls by
///
/// Shell.tsx Thread: stick-to-bottom following. While a turn runs, growth of the transcript (new content, paced
/// text) keeps the bottom in view until the reader drags the thread; dragging away from the bottom releases the
/// follow, dragging back to it restores it. A turn ending under a following thread keeps the bottom in view while the
/// reply's last lines pace out and the process folds away. Reading history stays put even when the turn ends in the
/// background. Otherwise a settled session never moves on its own, so
/// a fold opened by a tap keeps the row under the finger in place
struct Thread: View {
  let session: SessionView
  /// `-open`: folds pre-opened in the last exchange, as in the full-length render
  @Environment(\.preOpen) private var preOpen
  @State private var position = ScrollPosition(edge: .bottom)
  @State private var follow = true
  @State private var dragging = false
  /// `dragging` for the rows: one that mounts while it holds came in by the scroll and skips its entrance
  @State private var gesture = ScrollGesture()
  /// The offset the thread is at, kept outside SwiftUI's state so scrolling does not redraw the thread
  @State private var offset = ScrollOffset()
  /// The turn just ended under a following thread: growth still follows until the reader drags or a fold moves
  @State private var settling = false
  /// Opened and not yet scrolled or folded: the thread stays on its end while the lazy rows settle their heights
  @State private var opening = true
  /// Far enough from the end to offer the jump back (`ScrollToLatest`); nearer than that, the end holds (`keepEnd`)
  @State private var far = ObservedFlag()
  /// `follow` for the process folds: a reader who let go of the end keeps a fold the turn opened open until they come
  /// back (`CodexFold`). Observed by the folds alone, so a drag does not redraw the thread
  @State private var following = ObservedFlag(true)
  /// A fold the reader opened or closed by tap: size changes keep the offset, not the end, until the next touch or turn
  @State private var held = false
  /// An older engine reopened a finished turn near the end (`reopensAbove`): its fold's lazy rows keep settling for a few
  /// frames, so the end holds until the reader next touches the thread, even while the turn streams
  @State private var flipHold = false
  /// The session as it was before an older engine reopened its turn under a moving thread (`reopensAbove`), shown until
  /// the thread comes to rest. The whole process opened above the reply, which itself joined the process, and the offset
  /// cannot be corrected while the finger or its deceleration owns it (a scroll set then was overridden on the next frame
  /// and the thread flickered between the two). At rest the new state lands and `keepEnd` holds the end through it
  @State private var parked: SessionView?
  @State private var seen = SeenSession()
  @Environment(\.motionEnabled) private var motion

  var body: some View {
    seen.note(session)
    return ScrollView(.vertical) {
      ThreadContent(session: parked ?? session, preOpen: preOpen)
        .environment(\.holdScroll, hold)
        .environment(\.scrollGesture, gesture)
        .environment(\.threadFollowing, following)
    }
    .scrollIndicators(.hidden)
    .scrollPosition($position)
    .defaultScrollAnchor(.bottom, for: .initialOffset)
    .defaultScrollAnchor(.top, for: .alignment)
    // Lazy rows above a just-opened thread trade estimates for real heights as they lay out; without a bottom anchor
    // a long running turn left the thread at its top instead of its end. Later size changes go through `keepEnd`
    .defaultScrollAnchor(opening ? .bottom : nil, for: .sizeChanges)
    // As in ChatGPT and Messages: the thread scrolls freely above an open keyboard, and only a drag that carries on into
    // the keyboard pulls it down with the finger. `.immediately` closed it on any scroll, so history could not be read
    // while typing
    .scrollDismissesKeyboard(.interactively)
    .contentShape(Rectangle())
    .simultaneousGesture(TapGesture().onEnded {
      // Keep transcript buttons and links working on the same tap that dismisses the composer keyboard.
      #if canImport(UIKit)
      UIApplication.shared.sendAction(#selector(UIResponder.resignFirstResponder), to: nil, from: nil, for: nil)
      #endif
    })
    // Outside the tap above, so jumping to the end keeps the keyboard; inside the plan dock's inset, so it floats above
    // the plan card and the composer
    .overlay(alignment: .bottomTrailing) {
      // Not while the thread opens: its lazy rows report the top for a frame or two before the bottom anchor lands, and a
      // drop of glass inserted and removed inside the push never finished leaving (iOS 27 simulator)
      ScrollToLatest(far: far, hidden: opening || session.running && follow, action: toLatest)
        .padding(.trailing, Metrics.pad)
        .padding(.bottom, Metrics.s2)
        // On the composer's right edge, above the send button
        .frame(maxWidth: .infinity, alignment: .trailing)
        .readableWidth()
    }
    // The docked plan floats over the thread's bottom; the inset is also the transcript's scroll clearance. Outside the
    // tap that ends typing, as the jump button is: the card's own taps keep the keyboard
    .safeAreaInset(edge: .bottom, spacing: 0) { ThreadDock(session: session).readableWidth() }
    .onScrollPhaseChange { _, phase, context in
      let wasDragging = dragging
      dragging = phase == .interacting || phase == .tracking || phase == .decelerating
      gesture.active = dragging
      // A new touch ends a tapped fold's hold and the end kept through a change; so does a scroll of the system's own
      // (a tap on the status bar), which the kept end would fight
      if dragging, !wasDragging {
        held = false
        flipHold = false
        offset.pin = nil
        offset.top = nil
      } else if phase == .animating {
        offset.pin = nil
        offset.top = nil
      }
      // The thread at rest (or moved by the system) shows the session as it is
      if !dragging { parked = nil }
      // Any scroll, the reader's or the system's (a tap on the status bar), ends the opening anchor
      if phase != .idle { opening = false }
      if dragging {
        // Release on touch, not on a geometry Boolean changing: a stream may already have made atBottom false
        // before the drag, in which case that callback never fires and the next chunk pulls the reader back.
        follow = false
        settling = false
      } else if wasDragging, phase == .idle {
        let g = context.geometry
        follow = g.gapBelow <= Metrics.s2
        // The turn may have ended during the drag while its paced tail is still arriving.
        settling = follow
        if follow { position.scrollTo(edge: .bottom) }
      }
    }
    .onScrollGeometryChange(for: EndGeometry.self) { EndGeometry($0) } action: { old, new in
      offset.y = new.y
      offset.gap = new.gap
      if session.running || settling, follow, !dragging, new.size > old.size {
        position.scrollTo(edge: .bottom)
      } else {
        keepEnd(old, new)
        if offset.pin == nil { keepTop(old, new) }
      }
    }
    // Only a following thread settles at the end; finishing (including in the background) never overrides a drag.
    .onChange(of: session.running) { was, now in
      // A turn starting adds below what is there, and one ending under a reader who let go of the end keeps its fold
      // open (`CodexFold`); a following reader is kept on the end by the follow and `keepEnd`. Only a finished turn an
      // older engine reopens opens its process above the reply the reader is on (`reopensAbove`): Claude's autonomous
      // cycles did it every few seconds, and with the offset kept from the top a drag near the end flung the thread into
      // the middle of the process and back
      held = false
      let above = now && reopensAbove(session)
      flipHold = above && !far.value
      if dragging {
        if above, parked == nil { parked = seen.before }
      } else {
        offset.pin = above && !far.value ? max(0, offset.gap) : nil
      }
      guard was, !now, follow, !dragging else { return }
      settling = true
      position.scrollTo(edge: .bottom)
    }
    // More than half a screen above the end; written to an observed object the thread itself never reads, so crossing
    // the line redraws the button alone
    .onScrollGeometryChange(for: Bool.self) { $0.gapBelow > $0.containerSize.height / 2 } action: { _, now in far.value = now }
    .onChange(of: follow) { _, now in following.value = now }
  }

  /// The content or the room for it changed size: within half a screen of the end (where the jump button is not
  /// offered either) the distance to the end holds, so the keyboard and the composer's card rising lift the last lines
  /// with them instead of covering them, and for a following reader a process fold closing above the reply leaves the
  /// reply where it was. Further up, the offset from the top holds and reading history stays put. Content changing under
  /// a reader who let go of the end (a reply still pacing out after its turn, a turn streaming) never pulls that reader
  /// along, except through the frames an older engine's reopened turn settles in (`flipHold`); a fold the reader tapped
  /// keeps its row under the finger (`held`).
  /// Done by hand: once the reader has scrolled, a `.bottom` anchor for `.sizeChanges` left the offset where it was. The
  /// distance is pinned when a change begins and holds until the next touch: a fold's lazy rows grow the content over many
  /// frames, each reported before the previous scroll has landed. Never while the finger or its deceleration owns the
  /// offset: a scroll set then was overridden on the next frame and the thread flickered between the two for a second
  /// (iOS 27 simulator)
  private func keepEnd(_ old: EndGeometry, _ new: EndGeometry) {
    guard !dragging, old.size != new.size || old.room != new.room else { return }
    if offset.pin == nil {
      guard !opening, !held, old.gap <= old.room / 2, old.size == new.size || follow || flipHold else { return }
      offset.pin = max(0, old.gap)
    }
    position.scrollTo(y: max(0, new.size - new.room - (offset.pin ?? 0)))
  }

  /// Content that shrank under the reader clamps the offset toward the top: a running turn's rows swapping in a card
  /// (a steer, a question) re-estimate for a frame or two, and the thread was left at the top of the turn once they grew
  /// back ("jumps to the top"). The offset from before the shrink is noted and returned to as the content regrows, until
  /// the reader next touches the thread. Not for a fold the reader tapped, whose shrink is theirs (`held`)
  private func keepTop(_ old: EndGeometry, _ new: EndGeometry) {
    guard !dragging, !opening, !held else { return }
    if new.size < old.size, new.y < old.y - 1 {
      offset.top = offset.top ?? old.y
    } else if let top = offset.top, new.size > old.size {
      let y = min(top, max(0, new.size - new.room))
      position.scrollTo(y: y)
      if y >= top { offset.top = nil }
    }
  }

  /// A finished turn running again with its process fold opening above the reply: an engine that marks the reply final
  /// on a reopen (`mark_reply_final`) has the new work start a stretch under it instead (`splitPlanSections`)
  private func reopensAbove(_ session: SessionView) -> Bool {
    guard case .agent(let turn)? = session.turns.last else { return false }
    return turn.blocks.contains { $0.toolCall != nil }
      && !turn.blocks.contains { if case .text(let text) = $0 { text.phase == .final } else { false } }
  }

  /// The jump button: back to the end, and a running turn is followed again from there
  private func toLatest() {
    offset.pin = nil
    offset.top = nil
    follow = true
    settling = false
    opening = false
    if motion {
      withAnimation(.smooth) { position.scrollTo(edge: .bottom) }
    } else {
      position.scrollTo(edge: .bottom)
    }
  }

  /// A fold about to open or close: unless the thread is following a running turn, the bottom pin (the launch's
  /// `edge: .bottom`, or the last follow) gives way to the current offset, so the tapped row stays under the finger.
  /// A position left on the bottom edge keeps that edge in place as the content grows and shoved the whole thread up
  /// by the fold's height (iOS 27 simulator, 2026-10-27)
  private var hold: HoldScroll {
    let pinned = session.running && follow
    let position = $position, offset = offset, settled = $settling, opening = $opening, held = $held, flip = $flipHold
    return HoldScroll {
      // A fold the reader moved ends the settling follow and the opening anchor: the row under the finger stays put
      settled.wrappedValue = false
      opening.wrappedValue = false
      flip.wrappedValue = false
      offset.pin = nil
      offset.top = nil
      guard !pinned else { return }
      held.wrappedValue = true
      position.wrappedValue.scrollTo(y: offset.y)
    }
  }
}

/// The thread's last content offset (`Thread.hold`, `scrollTo(y:)`'s measure: under the content's top inset), its
/// distance to the end, the distance `Thread.keepEnd` holds through a change and the offset `Thread.keepTop` goes back to. Outside SwiftUI's state, so scrolling
/// does not redraw the thread
final class ScrollOffset: @unchecked Sendable {
  var y: CGFloat = 0
  var gap: CGFloat = 0
  var pin: CGFloat?
  /// The offset a shrink clamped the thread away from (`Thread.keepTop`)
  var top: CGFloat?
}

/// The session the thread showed before its latest change of run state (`Thread.parked`). Noted from the body, outside
/// SwiftUI's state, so it costs no redraw; `onChange` runs after the body has noted the new session
final class SeenSession {
  private(set) var before: SessionView?
  private var current: SessionView?

  func note(_ session: SessionView) {
    if let current, current.running != session.running { before = current }
    current = session
  }
}

/// What `Thread.keepEnd` compares across a change: the offset under the top inset (`contentOffset` counts from the
/// frame's top edge), the content's height, the room the insets leave for it, and the distance from the bottom of that
/// room to the content's end
struct EndGeometry: Equatable {
  var y: CGFloat
  var size: CGFloat
  var room: CGFloat
  var gap: CGFloat

  init(_ g: ScrollGeometry) {
    y = g.contentOffset.y + g.contentInsets.top
    size = g.contentSize.height
    room = g.containerSize.height
    gap = g.gapBelow
  }
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
    GlassEffectContainer {
      if shown {
        Button(action: action) {
          Icon(.arrowDown, size: Metrics.icon)
            .foregroundStyle(Palette.fg1)
            .frame(width: Metrics.ctlSm, height: Metrics.ctlSm)
            .contentShape(Circle())
        }
        .buttonStyle(.plain)
        // The composer's glass, frosted enough for prose passing behind
        .glassEffect(GlassStyle.bar.interactive(), in: Circle())
        // Glass cannot fade; the system's own materialise is how a drop of it comes and goes
        .glassEffectTransition(.materialize)
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
  @Environment(\.sessionActions) private var actions
  @State private var gate = QuietGate()

  var body: some View {
    let stream = liveStream ?? (model?.liveSessions.contains(session.id) == true)
    let quiet = gate.isQuiet(session: session, catchingUp: catchingUp, liveStream: stream)
    let groups = exchanges(session.turns)
    let activeAgent = session.turns.lastIndex { if case .agent = $0 { true } else { false } }
    // Each child reaches only the turn that announced it
    let nodes = nodesByTurn(session.subagents ?? [])
    Group {
      if lazy {
        // A prompt bubble does not stick (it would sit under the floating glass with the thread showing beside it).
        // The last exchange stays out of the lazy stack: the stack sizes the exchanges it has not laid out from the
        // ones it has, and a thread opened at the bottom lays out the last one first. A running turn's open process
        // is many screens tall, so every earlier exchange was taken for that tall, the bottom the thread opened at
        // lay far past the real content, and no row was there to draw: a blank chat until a drag
        VStack(alignment: .leading, spacing: 0) {
          LazyVStack(alignment: .leading, spacing: 0) {
            sections(groups, 0..<max(groups.count - 1, 0), activeAgent, nodes)
          }
          sections(groups, max(groups.count - 1, 0)..<groups.count, activeAgent, nodes)
          queue
        }
      } else {
        VStack(alignment: .leading, spacing: 0) {
          sections(groups, groups.indices, activeAgent, nodes)
          queue
        }
      }
    }
    // The messages' `Metrics.pad` puts prose, row icons and cards on the edge the dock, the composer and the bar's back
    // button share; the thread's own inset moves them a little inside it
    .padding(.horizontal, Metrics.threadInset)
    .readableWidth()
    .padding(.top, Metrics.padY)
    .padding(.bottom, WebMetrics.msgGap)
    .environment(\.quietEntrance, quiet)
    .environment(\.planContext, PlanContext(controls: session.controls, running: session.running, ready: session.status == .ready))
    .environment(\.allSubagents, session.subagents ?? [])
    .environment(\.sidekickModels, SidekickModels(session.controls))
  }

  /// The exchanges in `range` of `groups`; the gaps and pre-opened folds still count from the whole thread
  @ViewBuilder private func sections(_ groups: [Exchange], _ range: Range<Int>, _ activeAgent: Int?,
                                     _ nodes: [Int: [SubagentSummary]]) -> some View {
    // What the prompts' long-press menus offer, and the failed reply that offers Retry
    let editable = canEditPrompts(session)
    let goalPrompt = canSetGoal(session) ? lastTypedPrompt(session.turns) : nil
    let retry = retryableTurn(session)
    ForEach(Array(groups.enumerated())[range], id: \.element.id) { n, exchange in
      // The gap to the next exchange closes this one, so a pinned card keeps no transparent band above it; an
      // exchange ending in a reply's action row only needs a row gap, the row's own height separates them
      let next = n + 1 < groups.count
        ? (endsWithActions(exchange, running: session.running, active: activeAgent) ? Metrics.gap : WebMetrics.msgGap) : 0
      Section {
        VStack(alignment: .leading, spacing: WebMetrics.msgGap) {
          if let user = exchange.user, user.auto == true, user.autoReason == "subagents" {
            let reports = shownReports(user.text)
            if !reports.isEmpty { SubagentReturns(reports: reports) }
          }
          ForEach(exchange.agents, id: \.index) { item in
            AgentMessage(turn: item.turn, running: session.running && item.index == activeAgent && item.turn.stop == nil,
                         scope: "\(session.id):\(item.index)", index: item.index, last: item.index == session.turns.count - 1,
                         settings: exchange.user?.settings, retry: item.index == retry,
                         subagents: nodes[item.index] ?? [])
          }
        }
        .padding(.top, exchange.user != nil && !exchange.agents.isEmpty ? WebMetrics.msgGap : 0)
        .padding(.bottom, next)
        .environment(\.preOpen, n == groups.count - 1 ? preOpen : [])
      } header: {
        if let user = exchange.user, user.auto != true {
          UserMessageCard(turn: user, index: exchange.id, editable: editable, goal: exchange.id == goalPrompt)
        }
      }
    }
  }

  /// Prompts queued behind the running turn, as faint bubbles after it: where each lands once sent, so nothing moves
  /// when it goes. Over the composer they stacked under the floating to-do card and pushed it up
  @ViewBuilder private var queue: some View {
    if let queued = session.queued, !queued.isEmpty, session.external == nil {
      let steer = session.canSteer == true && session.running
      VStack(alignment: .trailing, spacing: WebMetrics.msgGap) {
        ForEach(queued) { item in
          QueueBubble(item: item, steer: steer, onSend: { actions.sendQueued(item.id) },
                      onEdit: { actions.editQueued(item.id) }, onRemove: { actions.removeQueued(item.id) })
        }
      }
      .padding(.horizontal, Metrics.pad)
      .padding(.top, WebMetrics.msgGap)
    }
  }

  private func endsWithActions(_ exchange: Exchange, running: Bool, active: Int?) -> Bool {
    guard let last = exchange.agents.last else { return false }
    let live = running && last.index == active && last.turn.stop == nil
    return showsTurnActions(last.turn, running: live)
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

  init(homeInset: CGFloat, @ViewBuilder content: () -> Content) {
    self.homeInset = homeInset
    self.content = content()
  }

  var body: some View {
    content
      .padding(.bottom, keyboard ? Metrics.s2 : max(Metrics.s2, homeInset - Metrics.barSink))
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
