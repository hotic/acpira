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
  /// The software keyboard is up: the system already lifts the dock over it, so the dock keeps only a small gap
  @State private var keyboard = false
  @State private var renaming = false
  @State private var newTitle = ""
  @State private var deleting = false
  @State private var accountOpen = false
  @Environment(\.initialPopup) private var initialPopup
  @Environment(\.initialDraft) private var initialDraft

  var body: some View {
    Group {
      if let session = model.views[id] {
        screen(session)
      } else {
        // Deleted while on screen (the stack pops it on the same pass), or the engine's view still on its way
        Palette.bg0.ignoresSafeArea()
      }
    }
    // Coming on screen, pushed or popped back to, switches the engine's view to this session
    .onAppear { model.open(id) }
    .onDisappear { model.parkDraft(id) }
  }

  private func screen(_ session: SessionView) -> some View {
    let agent = model.agent(session.agent)
    let pinned = model.summaries.first { $0.id == id }?.pinned == true
    // The unsent prompt and attachments live in the app model, so leaving the chat and coming back keeps them
    let draft = model.draft(for: id)
    return VStack(spacing: 0) {
      Thread(session: session)
      // The dock reaches down into the home indicator's band and rests where the list's system bottom bar does
      // (Metrics.barSink), leaving the thread more room; on a phone without the indicator it keeps the plain gap
      Dock(session: session, draft: Bindable(draft).text, staged: Bindable(draft).staged, edit: Bindable(draft).edit)
        .padding(.bottom, keyboard ? Metrics.s2 : max(Metrics.s2, homeInset - Metrics.barSink))
    }
    // The keyboard's safe area still lifts the dock; only the container's (the home indicator) is taken over
    .ignoresSafeArea(.container, edges: .bottom)
    // The bottom inset counts the keyboard too while it is up (34 → ~300 on an iPhone 18 Pro), and padding by it
    // lifted the dock a second keyboard's height, to the top of the screen; it also fed back into itself (the padding
    // moves the inset). So it is read only with the keyboard down, where it is the home indicator's band
    .onGeometryChange(for: CGFloat.self) { $0.safeAreaInsets.bottom } action: { if !keyboard { homeInset = $0 } }
    .modifier(KeyboardWatch(up: $keyboard))
    // The thread blurs and dims gradually as it scrolls up under the bar's glass buttons
    .topFade()
    .onAppear {
      if let initialDraft, draft.text.isEmpty { draft.text = initialDraft }
      // `-popup account`: the account sheet open on launch (screenshots); `-popup toast`: a moment later, the toast of
      // switching to the agent's first account
      if initialPopup == "account" { accountOpen = true }
      if initialPopup == "quotes" { model.seedQuotes(id) }
      if initialPopup == "edit" { model.beginEditLast(id) }
      if initialPopup == "toast", let account = model.accounts.first(where: { $0.agent == agent.id }) {
        Task {
          try? await Task.sleep(for: .seconds(1))
          model.selectAccount(id, account.id)
        }
      }
    }
    // The composer's liquid pickers, drawn over the whole screen so every piece takes taps
    .gooLayer()
    .modifier(SystemChatBar(pinned: pinned, actions: headerActions(session, agent)))
    .background(Palette.bg0)
    .environment(\.agentAccent, Palette.accent(for: session.agent))
    .environment(\.chatAgent, agent)
    .environment(\.sessionActions, sessionActions)
    // A subagent row pushes that child's inspector
    .environment(\.inspectSubagent, SubagentInspect(session: id) { model.path.append(.subagent(session: id, id: $0)) })
    .modifier(MediaLayer(session: session, loader: model.media))
    .environment(\.mentionPersonas, model.personas)
    .environment(\.compactAt, model.compactAt)
    .environment(\.hiddenModels, model.settings?.hiddenOptions[session.agent] ?? [:])
    .environment(\.diffSource, model.live ? { [model, id] in await model.diffSource(id, $0) } : nil)
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
  }

  private func headerActions(_ session: SessionView, _ agent: AgentInfo) -> HeaderActions {
    HeaderActions(
      back: { if !model.path.isEmpty { model.path.removeLast() } },
      new: { model.path.append(.new(agent: session.agent, cwd: session.cwd)) },
      account: hasAccountMenu(agent) ? { accountOpen = true } : nil,
      pin: { model.togglePin(id) },
      rename: { newTitle = session.title; renaming = true },
      export: { model.export(id, json: $0) },
      terminal: { model.openTerminal(session.cwd) },
      subagents: (session.subagents ?? []).isEmpty ? nil : { model.path.append(.subagents(session: id)) },
      subagentCounts: countsLabel(session.subagents ?? []),
      settings: { model.settingsOpen = true },
      delete: { deleting = true },
      changes: session.cwd.isEmpty ? nil : { model.path.append(.changes(id)) },
      copy: { kind in Task { await model.copy(id, kind) } },
      cwd: session.cwd,
      projects: model.projects,
      move: { model.move(id, to: $0) })
  }

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
        // Edit takes the prompt out of the queue and back into the composer
        if let prompt = model.takeQueued(id, item) {
          let draft = model.draft(for: id)
          draft.text = prompt.text
          draft.staged = prompt.attachments ?? []
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

/// What the header's buttons and its "…" menu do; `account` is nil when the agent has no account menu
struct HeaderActions {
  var back: () -> Void = {}
  var new: () -> Void = {}
  var account: (() -> Void)?
  var pin: () -> Void = {}
  var rename: () -> Void = {}
  var export: (Bool) -> Void = { _ in }
  /// The project's terminal page
  var terminal: () -> Void = {}
  /// The session's subagent graph page; nil while the session has no subagents. `subagentCounts` is its subtitle
  var subagents: (() -> Void)?
  var subagentCounts = ""
  var settings: () -> Void = {}
  var delete: () -> Void = {}
  /// The files the session changed (panel.changes)
  var changes: (() -> Void)?
  var copy: (SessionCopy) -> Void = { _ in }
  /// The session's folder and the others it can move to
  var cwd = ""
  var projects: [String] = []
  var move: ((String) -> Void)?
}

/// The chat's navigation bar: the system back button on the left, new session and the "…" menu as bar buttons on the
/// right (the system groups them on one glass capsule), no title: the thread scrolls under the glass
struct SystemChatBar: ViewModifier {
  var pinned: Bool
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
          Menu { NativeSessionMenu(pinned: pinned, actions: actions) } label: { Image(systemName: "ellipsis") }
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
/// follow, dragging back to it restores it. A settled session never moves on its own, so a fold opened by a tap keeps
/// the row under the finger in place
struct Thread: View {
  let session: SessionView
  /// `-open`: folds pre-opened in the last exchange, as in the full-length render
  @Environment(\.preOpen) private var preOpen
  @State private var position = ScrollPosition(edge: .bottom)
  @State private var follow = true
  @State private var dragging = false
  /// The offset the thread is at, kept outside SwiftUI's state so scrolling does not redraw the thread
  @State private var offset = ScrollOffset()

  var body: some View {
    ScrollView(.vertical) {
      ThreadContent(session: session, preOpen: preOpen)
        .environment(\.holdScroll, hold)
    }
    // The docked plan floats over the thread's bottom; the inset is also the transcript's scroll clearance
    .safeAreaInset(edge: .bottom, spacing: 0) { ThreadDock(session: session) }
    .scrollIndicators(.hidden)
    .scrollPosition($position)
    .defaultScrollAnchor(.bottom, for: .initialOffset)
    .defaultScrollAnchor(.top, for: .alignment)
    .scrollDismissesKeyboard(.interactively)
    .onScrollPhaseChange { _, phase in dragging = phase == .interacting || phase == .tracking || phase == .decelerating }
    .onScrollGeometryChange(for: Bool.self) { g in
      // At the bottom within a hair: the last line may sit a fraction of a point short after a layout pass
      g.contentOffset.y + g.containerSize.height - g.contentInsets.bottom >= g.contentSize.height - Metrics.s2
    } action: { _, atBottom in
      if dragging { follow = atBottom }
    }
    .onScrollGeometryChange(for: CGFloat.self) { $0.contentSize.height } action: { old, new in
      guard session.running, follow, !dragging, new > old else { return }
      position.scrollTo(edge: .bottom)
    }
    // `scrollTo(y:)` takes the point under the content's top inset; `contentOffset` counts from the frame's top edge
    .onScrollGeometryChange(for: CGFloat.self) { $0.contentOffset.y + $0.contentInsets.top } action: { _, y in offset.y = y }
  }

  /// A fold about to open or close: unless the thread is following a running turn, the bottom pin (the launch's
  /// `edge: .bottom`, or the last follow) gives way to the current offset, so the tapped row stays under the finger.
  /// A position left on the bottom edge keeps that edge in place as the content grows and shoved the whole thread up
  /// by the fold's height (iOS 27 simulator, 2026-10-27)
  private var hold: HoldScroll {
    let pinned = session.running && follow
    let position = $position, offset = offset
    return HoldScroll {
      guard !pinned else { return }
      position.wrappedValue.scrollTo(y: offset.y)
    }
  }
}

/// The thread's last content offset (`Thread.hold`)
final class ScrollOffset: @unchecked Sendable {
  var y: CGFloat = 0
}

/// The exchanges themselves; `lazy: false` lays every exchange out at once (a full-length render)
struct ThreadContent: View {
  let session: SessionView
  var lazy = true
  /// Folds opened in the last exchange before any tap (renders and previews)
  var preOpen: Set<PreOpen> = []

  var body: some View {
    let groups = exchanges(session.turns)
    let activeAgent = session.turns.lastIndex { if case .agent = $0 { true } else { false } }
    // Each child reaches only the turn that announced it
    let nodes = nodesByTurn(session.subagents ?? [])
    Group {
      if lazy {
        // A prompt bubble does not stick (it would sit under the floating glass with the thread showing beside it)
        LazyVStack(alignment: .leading, spacing: 0) {
          sections(groups, activeAgent, nodes)
        }
      } else {
        VStack(alignment: .leading, spacing: 0) { sections(groups, activeAgent, nodes) }
      }
    }
    .padding(.horizontal, Metrics.s1)
    .padding(.top, Metrics.padY)
    .padding(.bottom, WebMetrics.msgGap)
    .environment(\.planContext, PlanContext(controls: session.controls, running: session.running, ready: session.status == .ready))
    .environment(\.allSubagents, session.subagents ?? [])
  }

  @ViewBuilder private func sections(_ groups: [Exchange], _ activeAgent: Int?, _ nodes: [Int: [SubagentSummary]]) -> some View {
    // What the prompts' long-press menus offer, and the failed reply that offers Retry
    let editable = canEditPrompts(session)
    let goalPrompt = canSetGoal(session) ? lastTypedPrompt(session.turns) : nil
    let retry = retryableTurn(session)
    ForEach(Array(groups.enumerated()), id: \.element.id) { n, exchange in
      // The gap to the next exchange closes this one, so a pinned card keeps no transparent band above it; an
      // exchange ending in a reply's action row only needs a row gap, the row's own height separates them
      let next = n + 1 < groups.count
        ? (endsWithActions(exchange, running: session.running, active: activeAgent) ? Metrics.gap : WebMetrics.msgGap) : 0
      Section {
        VStack(alignment: .leading, spacing: WebMetrics.msgGap) {
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
    VStack(spacing: 0) {
      ThreadContent(session: session, lazy: false, preOpen: preOpen)
      ThreadDock(session: session)
      Dock(session: session, draft: .constant(""), staged: .constant([]))
        .padding(.bottom, Metrics.page)
    }
    .frame(width: width)
    .fixedSize(horizontal: false, vertical: true)
    .background(Palette.bg0)
    .environment(\.agentAccent, Palette.accent(for: session.agent))
    .environment(\.media, MediaScope(session: session.id))
    .environment(\.chatAgent, Fixtures.agents.first { $0.id == session.agent })
  }
}

/// Tracks the software keyboard from the system's will-show / will-hide notifications, which arrive before the
/// keyboard's safe area changes (no keyboard off iOS). The new-session screen's composer reads it too
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
