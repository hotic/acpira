import AcpiraDesign
import AcpiraModels
import SwiftUI

// chat/subagents/SubagentGroup.tsx and icons.tsx, and the DelegatedSubagents / ChildPermissions parts of Turns.tsx:
// the rows a turn shows for the children it delegated to (at the delegation point, or after the turn's process when
// no delegation row places them), their state marks, and a child's pending approval under its row. A row opens the
// child's inspector (Subagents/SubagentInspector.swift).

/// Opens a child's inspector. Compared by the session it opens into alone: a fresh closure per render of the chat must
/// not count as a change that redraws every row
struct SubagentInspect: Equatable {
  var session: String
  let run: (String) -> Void
  static func == (lhs: Self, rhs: Self) -> Bool { lhs.session == rhs.session }
}

extension EnvironmentValues {
  /// A tap on a subagent row; nil where rows only draw (the full-length render)
  @Entry var inspectSubagent: SubagentInspect? = nil
  /// The session's every node, for descendant counts and provenance that cross turns
  @Entry var allSubagents: [SubagentSummary] = []
  /// The nodes placed at this turn's delegation rows, by delegation (`placeNodes`)
  @Entry var delegations: [String: [SubagentSummary]] = [:]
  /// Inside the subagent inspector: the child's transcript is read-only (no fork, quoting or failure remedies), and
  /// its Working row leads with a static mark, as the root turn owns the only Orb
  @Entry var subagentTranscript = false
}

// MARK: - Rows

/// SubagentGroup: the turn's top-level children in announce order, finished ones included (a workflow's rows are its
/// progress; folding the done ones away made the list jump). No gap between rows: each carries its own padding
struct SubagentGroup: View {
  let nodes: [SubagentSummary]

  var body: some View {
    let rows = rootRows(nodes)
    if !rows.isEmpty {
      VStack(alignment: .leading, spacing: 0) {
        ForEach(rows) { node in
          EntranceOnce(id: "subagent:\(node.id)") { SubagentRowButton(node: node) }
        }
      }
      .accessibilityElement(children: .contain)
      .accessibilityLabel(Text(web: t("subagents.group", ["n": nodes.count])))
    }
  }
}

/// The row as a tap target opening the inspector; a plain row where there is nothing to open
struct SubagentRowButton: View {
  let node: SubagentSummary
  /// Off in the tree, whose indent already shows the descendants
  var descendants = true
  @Environment(\.inspectSubagent) private var inspect

  var body: some View {
    if let inspect {
      Button { inspect.run(node.id) } label: { SubagentRow(node: node, descendants: descendants) }
        .buttonStyle(RowButtonStyle())
    } else {
      SubagentRow(node: node, descendants: descendants)
    }
  }
}

/// An unframed row like the process folds: the state mark, the title with its faint meta, the status and elapsed time
/// at the end, then the live activity / waiting / result excerpt one step fainter. No chevron: the row opens a page.
/// A summoned child (another CLI) carries that CLI's mark on its state mark's corner, its model faintly with the
/// reasoning effort as a badge, and the round from the second one on
struct SubagentRow: View {
  let node: SubagentSummary
  var descendants = true
  @Environment(\.allSubagents) private var all

  var body: some View {
    let title = subagentTitle(node)
    let status = statusLabel(node)
    let line = secondLine(node)
    // The detail line repeats nothing the status already says
    let detail = line != status ? line : nil
    let below = descendants ? descendantCount(node.id, all) : 0
    Row {
      SubagentLead(node: node)
    } content: {
      VStack(alignment: .leading, spacing: Metrics.gap) {
        HStack(alignment: .firstTextBaseline, spacing: Metrics.s2) {
          // The web shrinks the faint parts four times faster than the title; at phone width a part cut to "t…"
          // says nothing, so whole parts drop instead, the least telling first, and the title truncates last
          let parts = metaParts(title: title, descendants: below)
          ViewThatFits(in: .horizontal) {
            ForEach(Array(Set(parts.map(\.rank)).sorted().reversed()), id: \.self) { rank in
              titleLine(title, parts.filter { $0.rank <= rank }).fixedSize()
            }
            titleLine(title, [])
          }
          .frame(maxWidth: .infinity, alignment: .leading)
          HStack(alignment: .firstTextBaseline, spacing: Metrics.s2) {
            Meta(status, ink: statusInk(node))
            SubagentElapsed(node: node)
          }
          .fixedSize()
        }
        if let detail {
          Text(web: detail).textStyle(WebType.text3).foregroundStyle(Palette.fg3).lineLimit(1).truncationMode(.tail)
        }
      }
    }
    // A row's breathing room above and below, so the pressed fill does not hug the title and the detail line
    .padding(.vertical, Metrics.gapHalf)
    .accessibilityElement(children: .ignore)
    .accessibilityLabel(Text(web: ([title, node.harness != nil ? node.role : nil, status, detail].compactMap { $0 })
      .joined(separator: t("common.listSep"))))
  }

  /// The title and the faint parts after it, each part whole
  private func titleLine(_ title: String, _ parts: [MetaPart]) -> some View {
    HStack(alignment: .firstTextBaseline, spacing: Metrics.s2) {
      Text(web: title).foregroundStyle(Palette.fg1).lineLimit(1).truncationMode(.tail).cssBaseline()
      ForEach(parts) { part in
        switch part.kind {
        case .text: Meta(part.text).fixedSize()
        case .effort: EffortTag(label: part.text)
        }
      }
    }
  }

  /// The title line's faint parts in order. `rank`: the narrower the line, the fewer kept (rank 1 survives longest).
  /// A summoned round number tells a follow-up apart, so it stays; model and role are the first to go
  private func metaParts(title: String, descendants: Int) -> [MetaPart] {
    var parts: [MetaPart] = []
    if let h = node.harness {
      if let model = node.model { parts.append(MetaPart(id: "model", kind: .text, text: model, rank: 3)) }
      if let effort = h.effort {
        let label = ComposerControls.effortLabel(SessionOption(id: effort, name: effort))
        parts.append(MetaPart(id: "effort", kind: .effort, text: label, rank: 2))
      }
      if h.round > 1 { parts.append(MetaPart(id: "round", kind: .text, text: t("subagents.round", ["n": h.round]), rank: 1)) }
    } else if let role = node.role, role != title {
      parts.append(MetaPart(id: "role", kind: .text, text: role, rank: 3))
    }
    if descendants > 0 {
      parts.append(MetaPart(id: "descendants", kind: .text, text: t("subagents.descendants", ["n": descendants]), rank: 2))
    }
    return parts
  }

  /// A waiting child takes the warn tone; how it ended colours the rest
  private func statusInk(_ node: SubagentSummary) -> Color {
    if isWaiting(node) { return Palette.warn }
    switch node.state {
    case .completed: return Palette.ok
    case .failed: return Palette.danger
    default: return Palette.fg3
    }
  }
}

/// One faint part of a row's title line
private struct MetaPart: Identifiable {
  enum Kind { case text, effort }
  let id: String
  let kind: Kind
  let text: String
  let rank: Int
}

/// A faint text-3 part of the row's title line
private struct Meta: View {
  let text: String
  var ink: Color = Palette.fg3
  init(_ text: String, ink: Color = Palette.fg3) { self.text = text; self.ink = ink }

  var body: some View {
    Text(web: text, size: WebType.text3.size).cssBaseline().textStyle(WebType.text3)
      .foregroundStyle(ink).lineLimit(1).truncationMode(.tail)
  }
}

/// ChipTag small: the composer chip's level name on a quiet fill
private struct EffortTag: View {
  let label: String

  var body: some View {
    Text(web: label, size: WebType.text4.size)
      .inlineFont(WebType.text4)
      .foregroundStyle(Palette.fg3)
      .padding(.horizontal, Metrics.s1)
      .frame(height: WebMetrics.chipTagSmall)
      .background(RoundedRectangle(cornerRadius: Metrics.rXs).fill(Palette.active))
      .fixedSize()
      // Sits on the line's baseline the way the inline-flex badge does
      .alignmentGuide(.firstTextBaseline) { $0[VerticalAlignment.center] + WebType.text4.size * 0.35 }
  }
}

/// useElapsed: a running child ticks on the app's clock; a finished one reads its engine timestamps
private struct SubagentElapsed: View {
  let node: SubagentSummary
  @Environment(\.clockOffset) private var offset

  var body: some View {
    Group {
      if node.state == .running && node.endedAt == nil {
        LiveTime(since: node.announcedAt) { ms in Text(web: commandDuration(ms: ms, whole: true)) }
      } else {
        Text(web: elapsedText(node, now: (Date().timeIntervalSince1970 + offset) * 1000))
      }
    }
    .cssBaseline().textStyle(WebType.text3).monospacedDigit().foregroundStyle(Palette.fg3).lineLimit(1)
  }
}

// MARK: - Marks

/// The lead slot: the state mark at --subagent-mark, and a summoned child's CLI mark on its corner over a page tile
struct SubagentLead: View {
  let node: SubagentSummary
  @Environment(AppModel.self) private var model: AppModel?

  var body: some View {
    SubagentStateIcon(node: node)
      .frame(width: WebMetrics.subagentMark, height: WebMetrics.subagentMark)
      .overlay(alignment: .bottomTrailing) {
        if let agent = node.harness?.agent {
          VendorMark(brand: agent, name: model?.agent(agent).name ?? agent,
                     size: WebMetrics.subagentBadge - 2 * Metrics.hairline)
            .padding(Metrics.hairline)
            .background(RoundedRectangle(cornerRadius: Metrics.rSm).fill(Palette.bg0))
            .foregroundStyle(Palette.fg1)
            .offset(x: -WebMetrics.subagentBadgeInset, y: -WebMetrics.subagentBadgeInset)
        }
      }
  }
}

/// icons.tsx stateIcon: a pending card outranks the state, as that is what the child is blocked on. Running and
/// completed share one view (SubagentMark), so a child finishing in front of the reader plays the bars → ✓ finish
struct SubagentStateIcon: View {
  let node: SubagentSummary

  var body: some View {
    if !(node.permissions ?? []).isEmpty {
      Icon(.hand, size: WebMetrics.icon).foregroundStyle(Palette.warn)
    } else if node.question != nil {
      Icon(.messageCircleQuestion, size: WebMetrics.icon).foregroundStyle(Palette.warn)
    } else if node.state == .running || node.state == .completed {
      SubagentMark(done: node.state == .completed)
    } else if node.state == .failed {
      Icon(.triangleAlert, size: WebMetrics.icon).foregroundStyle(Palette.danger)
    } else if node.state == .cancelled {
      Icon(.x, size: WebMetrics.icon)
    } else {
      Icon(.unplug, size: WebMetrics.icon)
    }
  }
}

/// Three dotted bars bob while the child works: each bar is five fixed beads, and it "grows" by fading its outer
/// beads in (subagents.css). When a live child completes, each bar pulls its beads into its centre, the three centres
/// land on the ✓'s vertices and the stroke draws through them. A node first seen completed is a plain ✓; with motion
/// off the bars stand still, every bead shown, as the web's kill switch leaves them
struct SubagentMark: View {
  let done: Bool
  @Environment(\.motionEnabled) private var motion
  @Environment(\.accessibilityReduceMotion) private var reduceMotion
  /// When the child completed in front of the reader; nil: no finish to play
  @State private var finishedAt: Date?

  var body: some View {
    let animate = motion && !reduceMotion
    Group {
      if done && finishedAt == nil {
        // The settled ✓ at --icon in the slot, in the row's quiet ink like every other Check
        Icon(.check, size: WebMetrics.icon)
      } else if !animate {
        Canvas { ctx, size in MarkPainter.bars(&ctx, size: size, at: nil) }
      } else {
        TimelineView(.animation) { context in
          Canvas { ctx, size in
            let now = context.date.timeIntervalSinceReferenceDate
            if let finishedAt {
              MarkPainter.finish(&ctx, size: size, elapsed: context.date.timeIntervalSince(finishedAt),
                                 startedAt: finishedAt.timeIntervalSinceReferenceDate)
            } else {
              MarkPainter.bars(&ctx, size: size, at: now)
            }
          }
        }
      }
    }
    .frame(width: WebMetrics.subagentMark, height: WebMetrics.subagentMark)
    .accessibilityHidden(true)
    .onChange(of: done) { was, now in
      if !was && now && animate { finishedAt = .now }
    }
    .task(id: finishedAt) {
      guard finishedAt != nil else { return }
      try? await Task.sleep(for: .seconds(Motion.subagentFinish))
      finishedAt = nil
    }
  }
}

/// icons.tsx's geometry on lucide's 24 grid and the finish's timeline (playFinish); the ✓ is lucide Check's own path
private enum MarkPainter {
  /// The bars span about as much of the slot as the Orb's dotted ring does in its own
  static let spread = 1.3
  static let barX = [12 - 5 * spread, 12, 12 + 5 * spread]
  /// Ring 0 is the centre bead; rings 1 and 2 step out by the pitch
  static let pitch = 3 * spread
  static let beads = [0, -1, 1, -2, 2]
  static let beadR = 1.0
  static let check: [(Double, Double)] = [(4, 12), (9, 17), (20, 6)]
  /// playFinish's beats, in seconds: the gather, its per-bar lag, the ✓'s draw, the bars' fade, the settle pop
  static let gather = 0.34, lag = 0.018, draw = 0.28, fade = 0.18, pop = 0.26, ink = 0.4

  /// The bead loop: a bar's half-height h(t) = 3.5 + 2.5·sin(2πt) in thirds of the pitch, each outer ring fading over
  /// one pitch (the keyframes of subagents.css)
  static func opacity(ring: Int, phase: Double) -> Double {
    guard ring > 0 else { return 1 }
    let h = (3.5 + 2.5 * sin(2 * .pi * phase)) / 3
    return min(1, max(0, h - Double(ring) + 0.5))
  }

  /// Bar i's place in the loop at `time`: each bar runs a third of a loop ahead of the one before it
  static func phase(_ i: Int, _ time: Double) -> Double {
    ((time + Double(i) * Motion.subagentBarStagger) / Motion.subagentLoop).truncatingRemainder(dividingBy: 1)
  }

  /// The running bars at `time`; nil stands them still with every bead shown
  static func bars(_ ctx: inout GraphicsContext, size: CGSize, at time: Double?) {
    let s = size.width / 24
    for (i, x) in barX.enumerated() {
      for k in beads {
        let o = time.map { opacity(ring: abs(k), phase: phase(i, $0)) } ?? 1
        let r = beadR * (k == 0 ? 1 : 0.4 + 0.6 * o) * s
        let c = CGPoint(x: x * s, y: (12 + Double(k) * pitch) * s)
        ctx.fill(Path(ellipseIn: CGRect(x: c.x - r, y: c.y - r, width: 2 * r, height: 2 * r)),
                 with: .color(Palette.fg1.opacity(o)))
      }
    }
  }

  /// The finish, `elapsed` seconds in; the loop is frozen where it stood at `startedAt`
  static func finish(_ ctx: inout GraphicsContext, size: CGSize, elapsed e: Double, startedAt: Double) {
    let s = size.width / 24
    let scale = Double(WebMetrics.subagentCheckScale)
    let vertex = check.map { (12 + ($0.0 - 12) * scale, 12 + ($0.1 - 12) * scale) }
    let settle = gather + 0.22
    // A small pop while the ink settles from the running colour into the row's resting one
    let popped = clamp((e - settle) / pop)
    let grow = 1 + 0.18 * sin(.pi * popped)
    ctx.translateBy(x: size.width / 2, y: size.height / 2)
    ctx.scaleBy(x: grow, y: grow)
    ctx.translateBy(x: -size.width / 2, y: -size.height / 2)
    let barsAlpha = 1 - clamp((e - (gather + 0.2)) / fade)
    let pulled = easeOut(clamp(e / (gather * 0.8)))
    for (i, x) in barX.enumerated() {
      let moved = easeOut(clamp((e - Double(i) * lag) / gather))
      let cx = x + (vertex[i].0 - x) * moved, cy = 12 + (vertex[i].1 - 12) * moved
      for k in beads {
        let start = opacity(ring: abs(k), phase: phase(i, startedAt))
        // Outer beads slide into the centre and vanish; the centre bead is the point that lands on the vertex
        let o = (k == 0 ? 1 : start * (1 - pulled)) * barsAlpha
        let r = beadR * (k == 0 ? 1 : (0.4 + 0.6 * start) * (1 - pulled) + 0.4 * pulled) * s
        let c = CGPoint(x: cx * s, y: (cy + Double(k) * pitch * (1 - pulled)) * s)
        ctx.fill(Path(ellipseIn: CGRect(x: c.x - r, y: c.y - r, width: 2 * r, height: 2 * r)),
                 with: .color(Palette.fg1.opacity(o)))
      }
    }
    var path = Path()
    path.addLines(vertex.map { CGPoint(x: $0.0 * s, y: $0.1 * s) })
    let drawn = easeInOut(clamp((e - (gather - 0.06)) / draw))
    let stroke = StrokeStyle(lineWidth: 1.5 * s * scale, lineCap: .round, lineJoin: .round)
    let rest = clamp((e - settle) / ink)
    let line = path.trimmedPath(from: 0, to: drawn)
    ctx.stroke(line, with: .color(Palette.fg1.opacity(1 - rest)), style: stroke)
    var settled = ctx
    settled.opacity = rest
    settled.stroke(line, with: .foreground, style: stroke)
  }

  static func clamp(_ x: Double) -> Double { min(1, max(0, x)) }
  /// cubic-bezier(.3, .7, .2, 1), near enough
  static func easeOut(_ x: Double) -> Double { 1 - pow(1 - x, 3) }
  /// cubic-bezier(.5, 0, .2, 1), near enough
  static func easeInOut(_ x: Double) -> Double { x < 0.5 ? 4 * x * x * x : 1 - pow(-2 * x + 2, 3) / 2 }
}

// MARK: - Placement in the turn

/// DelegatedSubagents: the rows of one delegation point (a parallel launch is one point), read from the turn's placed
/// nodes, with any pending child approval right under them
struct DelegatedSubagents: View {
  let ids: [String]
  @Environment(\.delegations) private var delegations

  var body: some View {
    let nodes = ids.flatMap { delegations[$0] ?? [] }
    if !nodes.isEmpty {
      VStack(alignment: .leading, spacing: Metrics.gap) {
        SubagentGroup(nodes: nodes)
        ChildPermissions(nodes: nodes)
      }
    }
  }
}

/// ChildPermissions: a child's pending approval under the rows. A summoned child's sits at the label column with
/// nothing above it, since the row already names who asks; another child's gets a provenance line naming the path
/// to it. The card answers through the root session, which routes it to the child
struct ChildPermissions: View {
  let nodes: [SubagentSummary]
  @Environment(\.allSubagents) private var all
  @Environment(\.sessionActions) private var actions

  private struct Card: Identifiable {
    let node: SubagentSummary
    let block: PermissionBlock
    var id: String { block.id }
  }

  var body: some View {
    let cards = nodes.flatMap { node in (node.permissions ?? []).map { Card(node: node, block: $0) } }
    ForEach(cards) { card in
      if card.node.harness != nil && card.block.planId == nil {
        PermissionCard(block: card.block) { actions.permission(card.block.id, $0) }
          .padding(.leading, WebMetrics.indent)
      } else {
        let path = breadcrumb(card.node.id, all.isEmpty ? nodes : all).map(subagentTitle).joined(separator: " › ")
        Row(dense: true) {
          Text(web: t("subagents.provenance", ["path": path])).textStyle(WebType.text3).foregroundStyle(Palette.fg3)
        }
        PermissionCard(block: card.block) { actions.permission(card.block.id, $0) }
      }
    }
  }
}
