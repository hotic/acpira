import AcpiraDesign
import AcpiraModels
import SwiftUI

// chat/subagents/SidekickRelay.tsx: a round of the built-in agent's sidekick in the transcript. The lead hands over (its
// mark flips into the sidekick's while a light crosses the row), the sidekick's latest steps stream through a fixed-height
// window, and the handback lands in the window's last slot with the same light. Nothing changes height while the round
// runs or ends; the inspector holds the outputs.

/// The lead model and the sidekick's display names, from the session's controls. The handoff's first mark is the
/// lead's, and a round carries only the sidekick select's value
struct SidekickModels: Equatable {
  struct Lead: Equatable {
    var name: String
    var brand: String?
  }
  var lead: Lead?
  var names: [String: String] = [:]

  init() {}

  init(_ controls: SessionControls) {
    let split = ComposerControls.split(controls.options)
    if let control = split.models.first, let picked = control.options.first(where: { $0.id == control.value }) {
      lead = Lead(name: picked.name, brand: ModelCatalog.optionBrand(picked))
    }
    names = Dictionary((split.sidekick.first?.options ?? []).map { ($0.id, $0.name) }, uniquingKeysWith: { first, _ in first })
  }
}

extension EnvironmentValues {
  @Entry var sidekickModels = SidekickModels()
}

/// What one mounted relay remembers across its renders. A class, as `EntranceClaim`: settling it while the body is
/// evaluated must not invalidate the view
private final class RelayMemo {
  /// Rows the window holds; the engine sends as many steps (`SIDEKICK_STEPS`)
  static let window = 4

  /// Decided once, at mount: whether it plays the handoff
  var mounted = false
  var handoff = false
  /// A round seen running keeps the whole window for good, so its end never shrinks the block
  var reserve = false
  /// Steps already there at mount stay still
  var seen: Set<String> = []
  var kept: [ToolCallBlock] = []
  var wasRunning: Bool?
  var handback = false
  /// The window's rows at the last render, and the push to play back
  var ids: [String]?
  var push = 0
  var rise = 0

  /// The engine sends the newest few steps; the one it just dropped stays a moment longer, so the push that brings a
  /// new step in moves real rows instead of opening a gap at the top. Idempotent for the same steps
  func keep(_ steps: [ToolCallBlock]?) -> [ToolCallBlock] {
    let now = steps ?? []
    let ids = Set(now.map(\.id))
    kept = Array((kept.filter { !ids.contains($0.id) } + now).suffix(Self.window + 1))
    return kept
  }

  /// The window sits on its bottom edge once it overflows, so a row appended there pushes the rest up at once; the
  /// push is played back as a rise of one row per new row
  func track(_ next: [String], slots: Int, live: Bool) {
    defer { ids = next }
    guard let before = ids, before != next, live else { return }
    let added = before.last.flatMap { next.lastIndex(of: $0) }.map { next.count - 1 - $0 } ?? next.count
    let rise = min(max(0, added), max(0, next.count - slots))
    if rise > 0 {
      self.rise = rise
      push += 1
    }
  }
}

/// Rounds seen running, so their window keeps every slot across a remount (a lazy stack scrolling the turn back in)
private enum RelayLedger {
  static var reserved = Set<String>()

  static func reserve(_ key: String) {
    if reserved.count >= EntranceLedger.cap { reserved.removeAll() }
    reserved.insert(key)
  }
}

/// One slot of the window: a step, or the handback once the round ended
private enum RelayItem: Identifiable {
  case step(ToolCallBlock)
  case back
  var id: String {
    switch self {
    case .step(let b): b.id
    case .back: "\u{0}back"
    }
  }
}

struct SidekickRelay: View {
  let node: SubagentSummary
  @Environment(\.sidekickModels) private var models
  @Environment(\.inspectSubagent) private var inspect
  @Environment(\.entranceScope) private var scope
  @Environment(\.motionEnabled) private var motion
  @Environment(\.accessibilityReduceMotion) private var reduceMotion
  @Environment(\.scrollGesture) private var gesture
  @State private var memo = RelayMemo()

  var body: some View {
    let running = node.state == .running
    let model = node.model.map { models.names[$0] ?? $0 } ?? t("subagents.untitled")
    let brand = node.model.flatMap { VendorBrand.of($0) } ?? VendorBrand.of(model)
    let live = settle(running: running)
    let kept = memo.keep(node.steps)
    let items: [RelayItem] = kept.map { .step($0) } + (running ? [] : [.back])
    let slots = memo.reserve ? RelayMemo.window : max(1, min(RelayMemo.window, items.count))
    let _ = memo.track(items.map(\.id), slots: slots, live: live)
    let mark = SidekickGlyph(brand: brand, name: model)

    VStack(alignment: .leading, spacing: Metrics.processRowGap) {
      tappable { Head(node: node, model: model, mark: mark, lead: models.lead, play: memo.handoff) }
      window(items, slots: slots, mark: mark, live: live)
    }
    .environment(\.rowHeight, WebMetrics.processRow)
    .environment(\.rowDenseHeight, WebMetrics.processRow)
    // The relay runs its own entrances; Row's staggered part fades would put the words ahead of the light
    .environment(\.rowEnter, false)
    .accessibilityElement(children: .contain)
    .accessibilityLabel(Text(web: t("sidekick.relay", ["model": model])))
  }

  /// The mount's decisions, and the handback once the round ends in front of the reader. Entrances play only for what
  /// happens on screen in a live turn: restored history, a lazy stack scrolling the turn back in and a fling stay still.
  /// Whether motion may play is read at each event, so a round mounted mid-fling still slides in its later steps and a
  /// motion setting changed mid-round holds from the next event on
  private func settle(running: Bool) -> Bool {
    let live = scope.live && motion && !reduceMotion && !gesture.active
    if !memo.mounted {
      memo.mounted = true
      let key = "\(scope.scope)\u{0}sidekick:\(node.id)"
      // Claimed by the first mount whatever it shows, so a round first met in history never hands off later
      memo.handoff = EntranceLedger.claim(key) && running && live
      if running { RelayLedger.reserve(key) }
      memo.reserve = RelayLedger.reserved.contains(key)
      memo.seen = Set((node.steps ?? []).map(\.id))
    }
    if memo.wasRunning == true && !running && live { memo.handback = true }
    memo.wasRunning = running
    return live
  }

  /// The fixed-height window: once it overflows the rows sit on its bottom edge and the top one fades out under a mask
  private func window(_ items: [RelayItem], slots: Int, mark: SidekickGlyph, live: Bool) -> some View {
    let pitch = WebMetrics.processRow + Metrics.processRowGap
    let height = CGFloat(slots) * WebMetrics.processRow + CGFloat(slots - 1) * Metrics.processRowGap
    let more = items.count > slots
    let rise = memo.rise
    return VStack(alignment: .leading, spacing: Metrics.processRowGap) {
      ForEach(items) { item in
        switch item {
        case .step(let block):
          Step(block: block, mark: mark, play: live && !memo.seen.contains(block.id))
        case .back:
          // Played once: a handback rebuilt later stands still
          let memo = memo
          tappable { Handback(node: node, mark: mark, play: memo.handback, played: { memo.handback = false }) }
        }
      }
    }
    .fixedSize(horizontal: false, vertical: true)
    .keyframeAnimator(initialValue: CGFloat.zero, trigger: memo.push) { content, y in
      content.offset(y: y)
    } keyframes: { _ in
      KeyframeTrack {
        MoveKeyframe(CGFloat(rise) * pitch)
        LinearKeyframe(CGFloat.zero, duration: Motion.sidekickSlide, timingCurve: Motion.sidekickPush)
      }
    }
    .frame(maxWidth: .infinity, minHeight: height, maxHeight: height, alignment: more ? .bottom : .top)
    // The pressed fill reaches past the text column, so the clip does too
    .padding(.horizontal, Metrics.hit)
    .clipped()
    .mask {
      if more {
        LinearGradient(stops: [.init(color: .clear, location: 0), .init(color: .black, location: WebMetrics.processRow * RelayKeys.maskRows / height)],
                       startPoint: .top, endPoint: .bottom)
      } else {
        Rectangle()
      }
    }
    .padding(.horizontal, -Metrics.hit)
  }

  /// A row opening the round's inspector; a plain row where rows only draw
  @ViewBuilder private func tappable(@ViewBuilder _ row: () -> some View) -> some View {
    if let inspect {
      let id = node.id
      Button { inspect.run(id) } label: { row() }.buttonStyle(RowButtonStyle())
    } else {
      row()
    }
  }
}

/// The sidekick's model mark, leading the handoff, every step and the handback
private struct SidekickGlyph: View {
  let brand: String?
  let name: String

  /// Its vendor's own colour, else the neutral ink: a mark tinted in a colour of the app's own read as the wrong vendor
  var tint: Color { VendorBrand.color(brand) ?? Palette.fg1 }

  /// The handoff light: the mark's colour, or a colour logo's inks, faint, in the order the logo lays them left to right
  /// and its base last (SidekickRelay.tsx brandStyle)
  var sheen: [Gradient.Stop] {
    let washes = brand.flatMap { VendorMarks.layers[$0] } ?? []
    guard !washes.isEmpty else {
      return [.init(color: .clear, location: 0), .init(color: tint.opacity(RelayKeys.handoffTint), location: RelayKeys.handoffPeak),
              .init(color: .clear, location: 1)]
    }
    let inks = washes.sorted { $0.from.0 < $1.from.0 }.map { Swatch(RGBA(hex: $0.color), RGBA(hex: $0.color)).color } + [tint]
    let (from, to) = RelayKeys.sheenSpan
    return [.init(color: .clear, location: 0)]
      + inks.enumerated().map { i, ink in
        .init(color: ink.opacity(RelayKeys.sheenInk), location: from + (to - from) * Double(i) / Double(inks.count - 1))
      }
      + [.init(color: .clear, location: 1)]
  }

  var body: some View {
    VendorMark(brand: brand, name: name, size: WebMetrics.icon, colored: true).foregroundStyle(tint)
  }
}

/// The handoff row: the lead's mark flipping into the sidekick's, "<model> takes <task>", the level and the clock
private struct Head: View {
  let node: SubagentSummary
  let model: String
  let mark: SidekickGlyph
  let lead: SidekickModels.Lead?
  let play: Bool
  @Environment(\.clockOffset) private var offset
  var body: some View {
    let title = subagentTitle(node)
    let waiting = isWaiting(node)
    let status: String? = waiting ? statusLabel(node) : node.state == .running ? stateLabel(node) : nil
    Row {
      FlipMark(from: lead.map { VendorMark(brand: $0.brand, name: $0.name, size: WebMetrics.icon).foregroundStyle(Palette.fg3) },
               to: mark)
        .relayEnter(play, .flip, duration: Motion.sidekickFlip, delay: Motion.sidekickFlipDelay, curve: Motion.sidekickSpring)
    } content: {
      // At phone width the level drops whole first, then the task truncates; the model name and the verb stay
      ViewThatFits(in: .horizontal) {
        label(title, level: true).fixedSize()
        label(title, level: false)
      }
      .relayEnter(play, .slide, duration: Motion.sidekickLabel, delay: Motion.sidekickLabelDelay, curve: Motion.sidekickExpo)
    } trailing: {
      HStack(alignment: .firstTextBaseline, spacing: Metrics.s1) {
        if let status { Text(web: status).foregroundStyle(waiting ? Palette.warn : Palette.fg3) }
        SubagentElapsed(node: node)
      }
      .cssBaseline().textStyle(WebType.text3).lineLimit(1)
      .relayEnter(play, .fade, duration: Motion.sidekickTrail, delay: Motion.sidekickTrailDelay, curve: .easeOut)
    }
    .overlay { Sheen(play: play, stops: mark.sheen, delay: Motion.sidekickSheenDelay) }
    .accessibilityElement(children: .ignore)
    .accessibilityLabel(Text(web: [t("sidekick.relay", ["model": model]), t("sidekick.takes"), title, status,
                                   elapsedText(node, now: (Date().timeIntervalSince1970 + offset) * 1000)]
      .compactMap { $0 }.joined(separator: " ")))
  }

  /// "<model> takes <task>", with the round from the second one on and, when it fits, the sidekick's level
  private func label(_ title: String, level: Bool) -> some View {
    HStack(alignment: .firstTextBaseline, spacing: Metrics.s2) {
      Text(web: model).foregroundStyle(Palette.fg1).lineLimit(1).truncationMode(.tail).cssBaseline().layoutPriority(1)
      Text(web: t("sidekick.takes")).foregroundStyle(Palette.fg2).lineLimit(1).fixedSize().cssBaseline().layoutPriority(2)
      if let round = node.harness?.round, round > 1 {
        Text(web: t("subagents.round", ["n": round])).cssBaseline().textStyle(WebType.text3)
          .foregroundStyle(Palette.fg3).lineLimit(1).fixedSize().layoutPriority(2)
      }
      Text(web: title).foregroundStyle(Palette.fg3).lineLimit(1).truncationMode(.tail).cssBaseline()
      if level, let effort = node.harness?.effort {
        EffortTag(label: ComposerControls.effortLabel(SessionOption(id: effort, name: effort)))
      }
    }
  }
}

/// One of the sidekick's tool rows, its mark in the lead slot. Rows stay plain: the inspector holds the outputs
private struct Step: View {
  let block: ToolCallBlock
  let mark: SidekickGlyph
  let play: Bool

  var body: some View {
    let running = block.status == .in_progress && block.observation != .unknown
    let target = block.target?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
    // A read names its file: the window is narrow, and the path's head is the same for every step
    let shown = block.kind == .read ? target.split(whereSeparator: { $0 == "/" || $0 == "\\" }).last.map(String.init) ?? target : target
    Row {
      mark
    } content: {
      RowLabel(toolVerb(block), shimmer: running)
      if !shown.isEmpty { RowTarget(shown, mono: block.targetMono == true) }
    } trailing: {
      if block.kind == .execute {
        ToolTrailing(block: block, execute: true)
      } else if block.status == .failed {
        CommandOutcome(status: block.status)
      }
    }
    .relayEnter(play, .slide, duration: Motion.sidekickStep, curve: Motion.sidekickExpo)
  }
}

/// The round's end in the window's last slot: what came back, or how it stopped. Mark, words and light start on one
/// beat, and the words ride the light's curve across the row, so nothing reads before the light reaches it
private struct Handback: View {
  let node: SubagentSummary
  let mark: SidekickGlyph
  let play: Bool
  let played: () -> Void
  @Environment(\.clockOffset) private var offset

  var body: some View {
    let ok = node.state == .completed
    let line = node.result.flatMap { result in
      resultText(result).split(whereSeparator: \.isNewline).lazy.map { $0.trimmingCharacters(in: .whitespaces) }.first { !$0.isEmpty }
    }
    Row {
      mark.relayEnter(play, .pop, duration: Motion.sidekickPop, curve: Motion.sidekickSpring)
    } content: {
      HStack(alignment: .firstTextBaseline, spacing: Metrics.s2) {
        Text(web: ok ? t("sidekick.handback") : stateLabel(node))
          .foregroundStyle(ok ? Palette.fg2 : node.state == .failed ? Palette.danger : Palette.fg3)
          .lineLimit(1).fixedSize().cssBaseline()
        if let line { Text(web: line).foregroundStyle(Palette.fg1).lineLimit(1).truncationMode(.tail).cssBaseline() }
      }
      .relayEnter(play, .slide, duration: Motion.sidekickBackText, curve: Motion.sidekickSweep)
    } trailing: {
      HStack(alignment: .firstTextBaseline, spacing: Metrics.s1) {
        Text(web: t("sidekick.steps", ["n": node.toolCount])).cssBaseline().textStyle(WebType.text3)
        SubagentElapsed(node: node)
      }
      .relayEnter(play, .fade, duration: Motion.sidekickBackTrail, delay: Motion.sidekickBackTrailDelay, curve: .easeOut)
    }
    .overlay {
      Sheen(play: play, stops: [.init(color: .clear, location: 0), .init(color: Palette.fg1.opacity(RelayKeys.handbackTint), location: RelayKeys.handbackPeak),
                                .init(color: .clear, location: 1)], delay: 0)
    }
    .onAppear { if play { played() } }
    .accessibilityElement(children: .ignore)
    .accessibilityLabel(Text(web: [ok ? t("sidekick.handback") : stateLabel(node), line, t("sidekick.steps", ["n": node.toolCount]),
                                   elapsedText(node, now: (Date().timeIntervalSince1970 + offset) * 1000)]
      .compactMap { $0 }.joined(separator: " ")))
  }
}

// MARK: - Motion

/// The keyframes' shape (subagents.css): the light's band and run as fractions of the row (from one band off its start
/// to 260 % of a band past it), its tints, the pop's start and the mask's reach in rows
private enum RelayKeys {
  static let band = 0.4, travel = 3.6, sheenIn = 0.3
  static let handoffTint = 0.18, handoffPeak = 0.55, handbackTint = 0.09, handbackPeak = 0.45
  /// A colour logo's light: each ink's strength, and where across the band the inks sit
  static let sheenInk = 0.14, sheenSpan = (0.2, 0.8)
  static let popFrom = 0.5, popIn = 0.4
  static let maskRows = 1.1
}

/// The relay's entrance shapes: words and steps slide in from the gutter, the tallies fade, the handback's mark pops
/// and the handoff's mark flips
private enum RelayShape { case slide, fade, pop, flip }

/// One part of an entrance, decided at mount and played once; `p` runs 0 → 1 on the part's curve. Not playing, the
/// part simply stands at rest, which is also what motion off shows
private struct RelayEnter: ViewModifier {
  let play: Bool
  let shape: RelayShape
  let duration: Double
  let delay: Double
  let curve: UnitCurve
  @State private var done: Bool?

  func body(content: Content) -> some View {
    content
      .modifier(RelayFrame(shape: shape, p: done ?? !play ? 1 : 0))
      .onAppear {
        guard done == nil else { return }
        if play {
          withAnimation(.timingCurve(curve, duration: duration).delay(delay)) { done = true }
        } else {
          done = true
        }
      }
  }
}

/// A part's look at `p` (the CSS keyframes of subagents.css). The spring curves run past 1, so the pop and the flip
/// overshoot and settle as theirs do
private struct RelayFrame: ViewModifier, Animatable {
  let shape: RelayShape
  var p: Double
  var animatableData: Double {
    get { p }
    set { p = newValue }
  }

  func body(content: Content) -> some View {
    switch shape {
    case .slide:
      content.opacity(min(1, max(0, p))).offset(x: -(1 - p) * Motion.sidekickNudge)
    case .fade:
      content.opacity(min(1, max(0, p)))
    case .pop:
      content.opacity(min(1, max(0, p / RelayKeys.popIn))).scaleEffect(RelayKeys.popFrom + (1 - RelayKeys.popFrom) * p)
    case .flip:
      content.environment(\.flipTurn, p)
    }
  }
}

extension EnvironmentValues {
  /// How far the handoff mark has turned, 0 (the lead's face) to 1 (the sidekick's, at rest)
  @Entry fileprivate var flipTurn: Double = 1
}

extension View {
  fileprivate func relayEnter(_ play: Bool, _ shape: RelayShape, duration: Double, delay: Double = 0, curve: UnitCurve) -> some View {
    modifier(RelayEnter(play: play, shape: shape, duration: duration, delay: delay, curve: curve))
  }
}

/// The handoff mark: the lead's on the front face, the sidekick's on the back; at rest the back shows. A face shows
/// while it looks forward, so the turn swaps them at its quarter
private struct FlipMark<From: View, To: View>: View {
  let from: From?
  let to: To
  @Environment(\.flipTurn) private var turn

  var body: some View {
    let angle = turn * 180
    ZStack {
      if angle < 90 {
        if let from {
          from.rotation3DEffect(.degrees(angle), axis: (x: 0, y: 1, z: 0))
        } else {
          to.rotation3DEffect(.degrees(angle), axis: (x: 0, y: 1, z: 0))
        }
      } else {
        to.rotation3DEffect(.degrees(angle - 180), axis: (x: 0, y: 1, z: 0))
      }
    }
    .frame(width: WebMetrics.icon, height: WebMetrics.icon)
    .accessibilityHidden(true)
  }
}

/// The light that crosses a relay row as it lands: a band two fifths of the row wide, from just off its start to well
/// past its end, bright for the middle of the run. Invisible at rest
private struct Sheen: View {
  let play: Bool
  /// The band's colours across its width, clear at both ends
  let stops: [Gradient.Stop]
  let delay: Double
  @State private var done: Bool?

  var body: some View {
    SheenBand(p: done ?? !play ? 1 : 0, stops: stops)
      .allowsHitTesting(false)
      .accessibilityHidden(true)
      .onAppear {
        guard done == nil else { return }
        if play {
          withAnimation(.timingCurve(Motion.sidekickSweep, duration: Motion.sidekickSheen).delay(delay)) { done = true }
        } else {
          done = true
        }
      }
  }
}

private struct SheenBand: View, Animatable {
  var p: Double
  let stops: [Gradient.Stop]
  var animatableData: Double {
    get { p }
    set { p = newValue }
  }

  var body: some View {
    GeometryReader { box in
      let band = box.size.width * RelayKeys.band
      LinearGradient(stops: stops, startPoint: .leading, endPoint: .trailing)
        .frame(width: band)
        .offset(x: band * (-1 + RelayKeys.travel * p))
        // sidekick-sheen: in over its first part, out over the rest
        .opacity(p < RelayKeys.sheenIn ? p / RelayKeys.sheenIn : max(0, 1 - (p - RelayKeys.sheenIn) / (1 - RelayKeys.sheenIn)))
    }
    .clipShape(RoundedRectangle(cornerRadius: Metrics.rMd))
  }
}
