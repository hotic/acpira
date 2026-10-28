import AcpiraDesign
import AcpiraModels
import SwiftUI

// chat/Thought.tsx, Plan.tsx, ProcessHistory.tsx and the Compaction / NoticeRow / Outcome rows of Turns.tsx.

// MARK: - Process history

/// `process-rows`: rows and dense rows shrink to --process-row, items sit --process-row-gap apart, and prose inside
/// the process keeps --process-prose-gap from its neighbours
struct ProcessHistory: View {
  let items: [ProcessItem]

  var body: some View {
    VStack(alignment: .leading, spacing: Metrics.processRowGap) {
      ForEach(Array(items.enumerated()), id: \.element.id) { i, item in
        let prose = isProse(item)
        let afterProse = i > 0 && isProse(items[i - 1])
        // Each item enters once under its own id, whatever view draws it
        EntranceOnce(id: itemEntrance(item.id)) { ProcessItemView(item: item) }
          .padding(.top, (prose && i > 0) || afterProse ? Metrics.processProseGap - Metrics.processRowGap : 0)
      }
    }
    .environment(\.rowHeight, WebMetrics.processRow)
    .environment(\.rowDenseHeight, WebMetrics.processRow)
    .frame(maxWidth: .infinity, alignment: .leading)
  }

  private func isProse(_ item: ProcessItem) -> Bool {
    if case .block(_, let block) = item {
      switch block { case .text, .steer: return true; default: return false }
    }
    return false
  }
}

/// One process item: a read / edit group, a single call, prose, or a line block
struct ProcessItemView: View {
  let item: ProcessItem

  var body: some View {
    switch item {
    case .group(_, let kind, let blocks):
      ToolGroupView(kind: kind, blocks: blocks)
    case .block(_, let block):
      switch block {
      case .toolCall(let call): ToolCallView(block: call, grouped: true)
      case .text(let text): StreamProse(block: text)
      default: LineBlockView(block: block)
      }
    }
  }
}

struct LineBlockView: View {
  let block: AgentBlock
  var body: some View {
    switch block {
    case .notice(let n): NoticeRow(block: n)
    case .thought(let th): ThoughtView(block: th)
    case .plan(let p): PlanDetails(entries: p.entries) { RowLabel(t("plan.title")) }
    case .toolCall(let call): ToolCallView(block: call)
    case .compaction(let c): CompactionRow(block: c)
    case .steer(let s): SteeredMessage(block: s)
    case .image(let image): AgentImageView(blob: image.blob, uri: image.uri)
    // The open card is docked above the composer; only a resolved one has a place in the message
    case .question(let q) where q.outcome != nil: QuestionRecord(block: q)
    case .unknown(let type): UnknownBlockRow(type: type)
    default: EmptyView()
    }
  }
}

// MARK: - Question record, unknown blocks

/// Questions.tsx QuestionRecord: an answered or skipped question keeps one row in the message, the answers folded
/// under it (open when answered)
struct QuestionRecord: View {
  let block: QuestionBlock
  @State private var manual: Bool?

  var body: some View {
    let answers = block.answers ?? [:]
    let label = block.outcome == .answered ? (answers.count == 1 ? t("question.answeredOne") : t("question.answered", ["n": answers.count]))
      : block.outcome == .skipped ? t("question.skipped") : t("question.cancelled")
    Disclosure(open: foldBinding($manual, block.outcome == .answered), icon: .messageCircleQuestion) {
      RowLabel(label)
    } content: {
      VStack(alignment: .leading, spacing: Metrics.s1_5) {
        ForEach(block.questions) { q in
          let names = answerNames(q, answers[q.id])
          VStack(alignment: .leading, spacing: 0) {
            Text(web: q.text).foregroundStyle(Palette.fg3)
            Text(web: names.isEmpty ? t("question.noAnswer") : names.joined(separator: t("common.listSep")))
              .foregroundStyle(names.isEmpty ? Palette.fg3 : Palette.fg1)
          }
          .textStyle(WebType.text2)
          .fixedSize(horizontal: false, vertical: true)
        }
      }
    }
  }

  /// An answer's option labels (free text as written)
  private func answerNames(_ q: Question, _ answer: QuestionAnswer?) -> [String] {
    let values: [String] = switch answer {
    case .one(let value): [value]
    case .many(let values): values
    case nil: []
    }
    return values.map { v in q.options.first { $0.id == v }?.label ?? v }
  }
}

/// A block type this build does not know (an engine ahead of the app): a quiet row naming it, so the turn does not
/// silently lose a step
struct UnknownBlockRow: View {
  let type: String
  var body: some View {
    Row {
      Icon(.puzzle, size: WebMetrics.icon)
    } content: {
      RowLabel(t("mobile.unknownBlock"))
      RowTarget(type, mono: true, ink: Palette.fg3)
    }
    .foregroundStyle(Palette.fg3)
  }
}

// MARK: - Thought

/// A thought: one action row (Thinking while it streams), its text in thought-tone prose inside a bounded scrollport
struct ThoughtView: View {
  let block: ThoughtBlock
  @Environment(\.rowHeight) private var row
  @Environment(\.preOpen) private var preOpen
  @State private var manual: Bool?

  var body: some View {
    let live = block.streaming == true
    // Thoughts carry no id; the start stamp names them within the turn
    EntranceOnce(id: "thought:\(block.startedAt.map { "\($0)" } ?? "")") {
    Disclosure(open: foldBinding($manual, preOpen.contains(.thought)), icon: .brain) {
      Shimmer(live ? t("host.thinking") : t("thought.label"), active: live)
    } content: {
      Scroller(.vertical) {
        StreamProse(markdown: block.text.trimmingTrailingWhitespace(), tone: .thought, streaming: live)
      }
      .scrollBounceBehavior(.basedOnSize)
      .scrollIndicators(.hidden)
      // --thought-body-max: min(--row × 8, 45dvh)
      .frame(maxHeight: row * 8)
      .fixedSize(horizontal: false, vertical: true)
      .defaultScrollAnchor(live ? .bottom : .top)
    }
    }
  }
}

// MARK: - Plan

/// Historical plans and to-do tool results: one disclosure row with the done count and a caret, the entries below
struct PlanDetails<Label: View, Trailing: View>: View {
  let entries: [PlanEntry]
  let label: Label
  let trailing: Trailing
  @State private var manual: Bool?
  @Environment(\.preOpen) private var preOpen

  init(entries: [PlanEntry], @ViewBuilder label: () -> Label, @ViewBuilder trailing: () -> Trailing) {
    self.entries = entries; self.label = label(); self.trailing = trailing()
  }

  var body: some View {
    let done = entries.filter { $0.status == .completed }.count
    let open = manual ?? preOpen.contains(.plan)
    // Entry status dots are not stops of this rail: it runs from the list icon down the body
    Disclosure(open: foldBinding($manual, preOpen.contains(.plan)), icon: .listTodo) {
      label
      Text(web: "\(done)/\(entries.count)")
        .cssBaseline()
        .textStyle(WebType.text3)
        .monospacedDigit()
        .foregroundStyle(Palette.fg3)
        .fixedSize()
      Icon(.chevronDown, size: WebMetrics.icon)
        .rotationEffect(.degrees(open ? 180 : 0))
        .alignmentGuide(.firstTextBaseline) { $0[VerticalAlignment.center] + WebType.text2.size * 0.35 }
    } trailing: {
      trailing
    } content: {
      PlanEntries(entries: entries)
    }
  }
}

extension PlanDetails where Trailing == EmptyView {
  init(entries: [PlanEntry], @ViewBuilder label: () -> Label) {
    self.init(entries: entries, label: label, trailing: { EmptyView() })
  }
}

/// The entry list: a status dot in the lead slot, the title dimming once done, a faint priority glyph for high / low
struct PlanEntries: View {
  let entries: [PlanEntry]
  var showStatus = true
  /// The dock's entries: the text line plus --plan-entry-pad-y above and below instead of a dense row
  var docked = false

  var body: some View {
    VStack(alignment: .leading, spacing: 0) {
      ForEach(Array(entries.enumerated()), id: \.offset) { _, entry in
        let ink = entry.status == .completed ? Palette.fg2 : entry.status == .in_progress ? Palette.fgStrong : Palette.fg1
        if docked {
          // min-h of the text line plus the padding, the padding inside the row
          PlanEntryRow(entry: entry, ink: ink, showStatus: showStatus)
            .environment(\.rowDenseHeight, WebType.text2.lineHeight)
            .padding(.vertical, Metrics.planEntryPadY)
        } else {
          PlanEntryRow(entry: entry, ink: ink, showStatus: showStatus)
        }
      }
    }
  }
}

struct PlanEntryRow: View {
  let entry: PlanEntry
  let ink: Color
  var showStatus: Bool

  var body: some View {
    Row(dense: true, leadTop: true) {
      if showStatus { PlanDot(status: entry.status) }
    } content: {
      Text(web: entry.title)
        .foregroundStyle(ink)
        .fixedSize(horizontal: false, vertical: true)
    } trailing: {
      if entry.priority == "high" {
        Icon(.chevronsUp, size: WebMetrics.chevron, strokeWidth: 1.75)
      } else if entry.priority == "low" {
        Icon(.chevronsDown, size: WebMetrics.chevron, strokeWidth: 1.75)
      }
    }
  }
}

/// Plan entry status: a filled check, a ring around an accent dot, an empty ring
struct PlanDot: View {
  let status: PlanStatus
  @Environment(\.agentAccent) private var accent
  private let size = WebMetrics.chevron
  private let ring: CGFloat = 1.5

  var body: some View {
    switch status {
    case .completed:
      Circle().fill(Palette.fg3).frame(width: size, height: size)
        .overlay { Icon(.check, size: Metrics.s2, strokeWidth: 3).foregroundStyle(Palette.bg0) }
    case .in_progress:
      Circle().strokeBorder(accent, lineWidth: ring).frame(width: size, height: size)
        .overlay { Circle().fill(accent).frame(width: Metrics.s1_5, height: Metrics.s1_5) }
    default:
      Circle().strokeBorder(Palette.fg3, lineWidth: ring).frame(width: size, height: size)
    }
  }
}

// MARK: - Compaction, notices, outcome

/// Context compaction as an action row (Turns.tsx Compaction): the live status shimmers until the agent reports the
/// outcome while the icon's four arrows keep pushing inward. A compaction watched to its end lets the loop reach its
/// rest beat, then clamps the arrows shut once and lands the new label bright before it eases to the action ink
struct CompactionRow: View {
  let block: CompactionBlock
  @Environment(\.motionEnabled) private var motion
  @Environment(\.accessibilityReduceMotion) private var reduceMotion
  @State private var phase = CompactionPhase.idle
  /// When this row's running loop began
  @State private var loopStart = Date()
  @State private var settleStart = Date()
  /// The pending phase changes, dropped when the status moves on (the web's effect cleanup)
  @State private var beat: Task<Void, Never>?

  var body: some View {
    let animate = motion && !reduceMotion
    // A completion still waiting for the arrows' rest beat keeps the running look, so label and icon change together
    let running = block.status == .in_progress || phase == .finishing
    let settling = phase == .settling && animate
    let label = running ? t("turns.compacting") : block.status == .completed ? t("turns.compacted")
      : block.status == .failed ? t("turns.compactFailed") : t("turns.compactCancelled")
    Row(tone: .action) {
      CompactionIcon(failed: block.status == .failed, loop: running && animate ? loopStart : nil,
                     settle: settling ? settleStart : nil)
    } content: {
      if settling {
        // acp-compact-label: from fg-strong to the row's ink, ease-out
        TimelineView(.animation) { context in
          let p = min(1, max(0, context.date.timeIntervalSince(settleStart) / CompactionMotion.settle))
          RowLabel(label).foregroundStyle(Palette.fgStrong.mix(with: Palette.actionFg, by: UnitCurve.easeOut.value(at: p)))
        }
      } else {
        RowLabel(label, shimmer: running)
      }
      if block.status == .failed, let error = block.error { RowTarget(error, ink: Palette.fg3) }
    }
    .onAppear { follow(block.status, loop: nil) }
    .onChange(of: block.status) { _, status in follow(status, loop: loopStart) }
  }

  /// useCompactionPhase: a compaction seen running settles once when it completes, after the loop reaches its rest
  /// beat. Restored history (never seen running) stays idle, and the id leaves the set on its first settle. A row
  /// rebuilt between the two states has no loop left to wait for (`loop` nil) and settles at once, as the web's does
  private func follow(_ status: CompactionStatus, loop: Date?) {
    beat?.cancel()
    if status == .in_progress { CompactionMotion.seen.insert(block.id); loopStart = .now; phase = .idle; return }
    guard CompactionMotion.seen.remove(block.id) != nil, status == .completed, motion && !reduceMotion else { return }
    let at = loop.map { Date().timeIntervalSince($0).truncatingRemainder(dividingBy: CompactionMotion.cycle) }
    let wait = at.map { $0 >= CompactionMotion.rest ? 0 : CompactionMotion.rest - $0 } ?? 0
    phase = wait > 0 ? .finishing : .settling
    settleStart = .now.addingTimeInterval(wait)
    beat = Task { @MainActor in
      if wait > 0 { try? await Task.sleep(for: .seconds(wait)) }
      guard !Task.isCancelled else { return }
      phase = .settling
      try? await Task.sleep(for: .seconds(CompactionMotion.settle))
      guard !Task.isCancelled else { return }
      phase = .idle
    }
  }
}

enum CompactionPhase { case idle, finishing, settling }

/// motion.css `acp-compact-in` / `acp-compact-snap` / `acp-compact-label`
enum CompactionMotion {
  /// One loop; every arrow is back at rest from `rest` to the loop's end, where the settle starts
  static let cycle = 1.4, rest = 1.2
  /// The arrows' stagger, the clamp and the label's landing
  static let stagger = 0.12, snapLength = 0.55, settle = 0.9
  /// Each arrow's way out, in icon units (--compact-x / --compact-y): the Shrink paths in order
  static let offsets: [CGSize] = [CGSize(width: 3, height: 3), CGSize(width: -3, height: 3),
                                  CGSize(width: 3, height: -3), CGSize(width: -3, height: -3)]
  static let loopCurve = UnitCurve.bezier(startControlPoint: UnitPoint(x: 0.5, y: 0), endControlPoint: UnitPoint(x: 0.2, y: 1))
  static let snapCurve = UnitCurve.bezier(startControlPoint: UnitPoint(x: 0.4, y: 0), endControlPoint: UnitPoint(x: 0.2, y: 1))
  /// Compactions seen running here: the row can be rebuilt between its running and its completed state
  static var seen = Set<String>()

  /// Arrow `i` at `t` seconds into the loop: a share of its way out and its opacity (0%: out at 0.3, 60–100%: home)
  static func loop(_ i: Int, _ t: Double) -> (out: Double, opacity: Double) {
    let local = t - Double(i) * stagger
    // Before its delay an arrow keeps its base style, at home
    guard local >= 0 else { return (0, 1) }
    let p = local.truncatingRemainder(dividingBy: cycle) / cycle
    guard p < 0.6 else { return (0, 1) }
    let e = loopCurve.value(at: p / 0.6)
    return (1 - e, 0.3 + 0.7 * e)
  }

  /// All arrows at `t` seconds into the clamp: home, half their way in at 40%, a tenth out at 70%, home
  static func snap(_ t: Double) -> Double {
    let p = min(1, max(0, t / snapLength))
    let stops: [(Double, Double)] = [(0, 0), (0.4, -0.5), (0.7, 0.1), (1, 0)]
    for k in 1..<stops.count where p <= stops[k].0 {
      let (p0, v0) = stops[k - 1], (p1, v1) = stops[k]
      return v0 + (v1 - v0) * snapCurve.value(at: (p - p0) / (p1 - p0))
    }
    return 0
  }
}

/// The compaction lead: the Shrink icon, its four arrows moved one by one while it loops or clamps. The plain icon
/// stays underneath, unseen, so the lead slot keeps its rail anchor
struct CompactionIcon: View {
  var failed: Bool
  var loop: Date?
  var settle: Date?

  var body: some View {
    let size = WebMetrics.icon
    if failed || (loop == nil && settle == nil) {
      Icon(failed ? .triangleAlert : .shrink, size: size)
    } else {
      Icon(.shrink, size: size).opacity(0).overlay {
        TimelineView(.animation) { context in
          ZStack {
            ForEach(Array(LucideIcon.shrink.elements.enumerated()), id: \.offset) { i, element in
              let (out, opacity) = loop.map { CompactionMotion.loop(i, context.date.timeIntervalSince($0)) }
                ?? (settle.map { CompactionMotion.snap(context.date.timeIntervalSince($0)) } ?? 0, 1)
              let way = CompactionMotion.offsets[i % CompactionMotion.offsets.count]
              IconElementShape(element: element)
                .stroke(style: StrokeStyle(lineWidth: 1.5 * size / 24, lineCap: .round, lineJoin: .round))
                .offset(x: way.width * out * size / 24, y: way.height * out * size / 24)
                .opacity(opacity)
            }
          }
          .frame(width: size, height: size)
        }
      }
    }
  }
}

/// One element of a lucide icon, scaled from its 24-unit box
nonisolated struct IconElementShape: Shape {
  let element: IconElement
  func path(in rect: CGRect) -> Path {
    let s = min(rect.width, rect.height) / 24
    return element.path.applying(CGAffineTransform(a: s, b: 0, c: 0, d: s, tx: rect.minX, ty: rect.minY))
  }
}

/// Which notice rows of a turn may offer the adapter's remedies (the session's last turn, settled), the notice the
/// turn's own error card already explains (`suppressId`, never drawn twice), and the error notices its closing row
/// carries instead (`absorbedNotices`)
struct NoticeScope {
  var showActions = false
  var suppressId: String?
  var absorbed: [NoticeBlock] = []
}

extension EnvironmentValues {
  @Entry var noticeScope = NoticeScope()
}

/// Workspace hook notices (`acp/session/hooks.rs`) carry this id prefix and the gate's shield glyph
private let hookNotice = "hooks:"

/// One row per failure (Turns.tsx NoticeRow). A notice with details folds like a thought, the title alone on the row
/// and the details on the rail once open; an error opens by default and marks only its glyph with the warn tone. An
/// info notice only informs, without the triangle. The adapter's remedies follow as small buttons at the label column;
/// sign-in is left to the session card, which says where it happens
struct NoticeRow: View {
  let block: NoticeBlock
  @Environment(\.noticeScope) private var scope
  @Environment(\.sessionActions) private var actions
  @Environment(\.motionEnabled) private var motion
  @Environment(\.holdScroll) private var holdScroll
  @State private var manual: Bool?

  var body: some View {
    if block.id != scope.suppressId && !scope.absorbed.contains(where: { $0.id == block.id }) {
      let error = block.severity == "error"
      let buttons = error && scope.showActions ? (block.actions ?? []).filter { $0 == .retry || $0 == .new_session } : []
      VStack(alignment: .leading, spacing: Metrics.s0_5) {
        if let details = block.details {
          fold(details, error: error)
        } else {
          Row { glyph(error) } content: {
            Text(web: block.title)
              .foregroundStyle(error ? Palette.fg2 : Palette.fg3)
              .fixedSize(horizontal: false, vertical: true)
          }
        }
        if !buttons.isEmpty {
          FlowLayout(spacing: Metrics.gap) {
            ForEach(buttons, id: \.self) { action in
              Button(web: action == .retry ? t("common.retry") : t("notice.continueNew")) { actions.failureAction(action) }
                .buttonStyle(AcpButtonStyle(.secondary, compact: true))
            }
          }
          .padding(.leading, WebMetrics.indent)
          .padding(.top, Metrics.s1)
        }
      }
    }
  }

  private func glyph(_ error: Bool) -> some View {
    let icon: LucideIcon = block.id.hasPrefix(hookNotice) ? (error ? .shieldX : .shieldAlert) : block.info == true ? .info : .triangleAlert
    return Icon(icon, size: WebMetrics.icon).foregroundStyle(error ? Palette.warn : Palette.fg3)
  }

  /// Disclosure's row and body rail with the notice's own tinted glyph in the lead
  private func fold(_ details: String, error: Bool) -> some View {
    let open = manual ?? error
    return VStack(alignment: .leading, spacing: 0) {
      Button {
        holdScroll()
        withAnimation(foldAnimation(opening: !open, motion: motion)) { manual = !open }
      } label: {
        Row(tone: .action) { glyph(error) } content: {
          RowLabel(block.title).lineLimit(1)
        }
      }
      .buttonStyle(RowButtonStyle())
      .accessibilityAddTraits(.isButton)
      .accessibilityValue(open ? Text(web: "expanded") : Text(web: "collapsed"))
      NoticeDetails(text: details)
        .transformPreference(RailLeadKey.self) { $0 = [] }
        .padding(.top, Metrics.s1)
        .padding(.leading, WebMetrics.indent)
        .frame(maxWidth: .infinity, alignment: .leading)
        .reveal(open)
    }
    .connectedRail(enabled: open, endAtLastRow: false, action: true)
  }
}

/// Details under an open notice: `- path` lines read as file rows, the rest verbatim
struct NoticeDetails: View {
  let text: String
  var body: some View {
    VStack(alignment: .leading, spacing: 0) {
      ForEach(Array(text.trimmingTrailingWhitespace().components(separatedBy: "\n").enumerated()), id: \.offset) { _, line in
        if let path = line.wholeMatch(of: /\s*-\s+(\S+)/).map({ String($0.1) }), path.contains("/") || path.contains(".") {
          HStack(spacing: Metrics.gap) {
            Icon(.fileText, size: WebMetrics.icon).foregroundStyle(Palette.fg3)
            Text(path).textStyle(WebType.mono).foregroundStyle(Palette.fg2).lineLimit(1).truncationMode(.middle)
          }
        } else {
          Text(web: line.isEmpty ? "\u{00a0}" : line).foregroundStyle(Palette.fg3).fixedSize(horizontal: false, vertical: true)
        }
      }
    }
    .textStyle(WebType.text3)
  }
}

/// One faint row closing the message: how the turn ended, the error's own words as the target. Error notices the
/// outcome absorbed follow as quiet detail lines at the label column
struct OutcomeRow: View {
  let turn: AgentTurn
  @Environment(\.noticeScope) private var scope
  var body: some View {
    if let outcome = turnOutcome(turn) {
      // A warning glyph for the short stops, none for "stopped" / "no reply"
      let warn = turn.stop != .cancelled && turn.stop != .end_turn
      VStack(alignment: .leading, spacing: Metrics.s0_5) {
        Row {
          if warn { LeadIcon(.triangleAlert) }
        } content: {
          RowLabel(outcome)
          if turn.stop == .error, let message = turn.error?.message { RowTarget(message, ink: Palette.fg3) }
        }
        ForEach(scope.absorbed, id: \.id) { notice in
          Text(web: notice.details.map { "\(notice.title)\n\($0)" } ?? notice.title)
            .textStyle(WebType.text3)
            .fixedSize(horizontal: false, vertical: true)
            .padding(.leading, warn ? WebMetrics.indent : 0)
        }
      }
      .foregroundStyle(Palette.fg3)
    }
  }
}
