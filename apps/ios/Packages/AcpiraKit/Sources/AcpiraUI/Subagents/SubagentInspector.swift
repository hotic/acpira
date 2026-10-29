import AcpiraDesign
import AcpiraModels
import SwiftUI

// chat/subagents/SubagentInspector.tsx as a pushed page: one delegated child's transcript, read-only. The delegated
// task is the child's prompt, so it heads the page as a user card; the child's turns follow with the main thread's
// views. Nothing here steers the session: no composer, no prompt, edit, retry or fork. The child's own approval and
// question cards answer through the parent session, and a running child the agent lets stop has "Request stop" in the
// bar's menu. The page follows exactly one child: it observes once the parent's view is here and stops on disappear
// (SubagentFeed).

struct SubagentInspector: View {
  let session: String
  let id: String
  @Environment(AppModel.self) private var model

  var body: some View {
    Group {
      if let root = model.views[session], let node = root.subagents?.first(where: { $0.id == id }) {
        page(root, node)
      } else {
        // The parent's view still on its way (a page opened from launch), or the child gone with an edit
        Palette.bg0.ignoresSafeArea()
      }
    }
    // Observed once the parent's view is here, as the web observes only the active session's child: an observe the
    // engine reads before it shows the session has no transcript to answer with, and is not answered later. A parent
    // carried over a reconnect counts once the new wire shows it
    .task(id: model.shown(session)) {
      if model.shown(session) { model.observeSubagent(session, id) }
    }
    .onDisappear { model.unobserveSubagent(session, id) }
  }

  private func page(_ root: SessionView, _ node: SubagentSummary) -> some View {
    let agent = node.harness?.agent ?? root.agent
    let transcript = model.subagentTranscript(session, id)
    let cancelable = node.state == .running && node.controls.cancel
    return InspectorThread(view: childView(root, node, transcript), node: node, loaded: transcript != nil)
      .safeAreaInset(edge: .bottom, spacing: 0) {
        // The child's open question: the parent's dock does not show it on the phone, so its card sits here
        if let question = node.question {
          QuestionCard(block: question) { model.answer(session, question: question.id, answers: $0, skipped: $1) }
            .id(question.id)
            .padding(.horizontal, Metrics.pad)
            .padding(.bottom, Metrics.s2)
        }
      }
      .topFade()
      .background(Palette.bg0)
      .navigationTitle(subagentTitle(node))
      .navigationSubtitle(statusLabel(node))
      .navigationBarTitleDisplayModeInline()
      .backWithoutTitle()
      .toolbar {
        if cancelable {
          ToolbarItem(placement: .primaryAction) {
            Menu {
              Button(role: .destructive) { model.cancelSubagent(session, id) } label: {
                Label(t(node.cancelRequested == true ? "subagents.cancelling" : "subagents.cancel"), systemImage: "stop.circle")
              }
              .disabled(node.cancelRequested == true)
            } label: {
              Image(systemName: "ellipsis")
            }
            .accessibilityLabel(Text(web: t("session.more")))
          }
        }
      }
      .environment(\.subagentTranscript, true)
      // A diff's whole files are asked for in the child's own transcript
      .environment(\.diffSource, model.live ? { [model, session, id] in await model.diffSource(session, $0, subagentId: id) } : nil)
      .environment(\.agentAccent, Palette.accent(for: agent))
      .environment(\.chatAgent, model.agent(agent))
      // Only the child's cards act, through the parent session; everything else is left at its no-op default
      .environment(\.sessionActions, SessionActions(
        permission: { model.choosePermission(session, block: $0, option: $1) },
        answer: { model.answer(session, question: $0, answers: $1, skipped: $2) },
        toast: { model.show($0, icon: $1) }))
      .modifier(MediaLayer(session: root, loader: model.media))
  }

  /// The child as a session the transcript views can draw: the task as its opening prompt, then its turns
  private func childView(_ root: SessionView, _ node: SubagentSummary, _ transcript: SubagentTranscript?) -> SessionView {
    let task = node.task.map { [Turn.user(UserTurn(text: $0))] } ?? []
    return SessionView(id: "subagent:\(node.id)", agent: node.harness?.agent ?? root.agent, title: subagentTitle(node),
                       cwd: root.cwd, turns: task + (receiptOnly(node) ? [] : transcript?.turns ?? []),
                       running: !receiptOnly(node) && transcript?.running == true)
  }
}

/// The child's thread: the main thread's views and its stick-to-bottom following (ChatScreen's `Thread`), with what
/// the inspector adds around the turns: the loading row, a receipt node's note and result, a nested child's result,
/// the note on a child whose outcome never arrived
private struct InspectorThread: View {
  let view: SessionView
  let node: SubagentSummary
  /// The first push arrived (receipt nodes never get one: their task and result are all there is)
  var loaded: Bool
  @State private var position = ScrollPosition(edge: .bottom)
  @State private var follow = true
  @State private var dragging = false
  /// Rows mounting while the reader scrolls skip their entrance (`ScrollGesture`)
  @State private var gesture = ScrollGesture()
  @State private var offset = ScrollOffset()

  var body: some View {
    let receipt = receiptOnly(node)
    let empty = !view.turns.contains { if case .agent = $0 { true } else { false } }
    let loading = !receipt && (!loaded || (empty && node.state == .running))
    // The result the parent received: all a receipt node has, a nested child's closing word, and what is left of a
    // settled child with no process of its own (a summoned round that never started)
    let result = receipt || node.visibility == .nested || (loaded && empty && node.state != .running) ? node.result : nil
    ScrollView(.vertical) {
      VStack(alignment: .leading, spacing: 0) {
        ThreadContent(session: view)
          .environment(\.holdScroll, hold)
          .environment(\.scrollGesture, gesture)
        if receipt || loading || result != nil || node.state == .disconnected {
          VStack(alignment: .leading, spacing: WebMetrics.msgGap) {
            if receipt {
              Text(web: t("subagents.receiptOnly")).foregroundStyle(Palette.fg3).fixedSize(horizontal: false, vertical: true)
            } else if loading {
              Row { Shimmer(t("subagents.loading")).foregroundStyle(Palette.fg3) }
            }
            if let result { ResultSection(markdown: result) }
            if node.state == .disconnected {
              Row { LeadIcon(.unplug) } content: {
                Text(web: t("subagents.disconnectedNote")).foregroundStyle(Palette.fg3)
                  .fixedSize(horizontal: false, vertical: true)
              }
            }
          }
          .textStyle(WebType.text2)
          .padding(.horizontal, Metrics.s1 + Metrics.pad)
          .padding(.bottom, WebMetrics.msgGap)
        }
      }
    }
    .scrollIndicators(.hidden)
    .scrollPosition($position)
    .defaultScrollAnchor(.bottom, for: .initialOffset)
    .defaultScrollAnchor(.top, for: .alignment)
    .onScrollPhaseChange { _, phase in
      dragging = phase == .interacting || phase == .tracking || phase == .decelerating
      gesture.active = dragging
    }
    .onScrollGeometryChange(for: Bool.self) { g in
      g.contentOffset.y + g.containerSize.height - g.contentInsets.bottom >= g.contentSize.height - Metrics.s2
    } action: { _, atBottom in
      if dragging { follow = atBottom }
    }
    // Stick to the bottom while the child streams, as the main thread does; dragging away releases the follow
    .onScrollGeometryChange(for: CGFloat.self) { $0.contentSize.height } action: { old, new in
      guard view.running, follow, !dragging, new > old else { return }
      position.scrollTo(edge: .bottom)
    }
    .onScrollGeometryChange(for: CGFloat.self) { $0.contentOffset.y + $0.contentInsets.top } action: { _, y in offset.y = y }
  }

  /// A fold about to move keeps the tapped row in place unless the thread follows a streaming child (`Thread.hold`)
  private var hold: HoldScroll {
    let pinned = view.running && follow
    let position = $position, offset = offset
    return HoldScroll {
      guard !pinned else { return }
      position.wrappedValue.scrollTo(y: offset.y)
    }
  }
}

/// The result the parent received, under a caption, as prose
private struct ResultSection: View {
  let markdown: String

  var body: some View {
    VStack(alignment: .leading, spacing: Metrics.s1) {
      Text(web: t("subagents.result")).textStyle(WebType.text3).foregroundStyle(Palette.fg3)
      StreamProse(block: TextBlock(markdown: markdown))
    }
  }
}
