import SwiftUI

// Standing loops (the list's state marks, the loading glyph, the shimmer's sweep) as SwiftUI animations instead of
// `TimelineView(.animation)` content. A timeline re-evaluates its content on every display frame and SwiftUI then walks
// its host's attribute graph: at rest, with a few marks turning, the session list spent about a tenth of the main thread
// on it, twice that on a 120 Hz screen. An animation is interpolated without the main thread.
//
// The loop's place comes from the wall clock, as the timelines' did: every mark of a kind moves in step, and a row
// coming back on screen carries on mid-loop rather than from its first frame.

/// How far a loop has moved at each point of its period, as the share of the change it animates
nonisolated enum LoopShape: Hashable, Sendable {
  /// 0 up to 1, then back at 0: a turn, a sweep
  case ramp
  /// The same with a quadratic ease-out (CSS `ease-out` on a ripple)
  case easeOut
  /// Keyframes `(at, share)` from 0 to 1, linear between them or eased in and out (a smoothstep, close to CSS
  /// `ease-in-out` over a few points of travel)
  case keys([LoopKey], eased: Bool)

  func share(at p: Double) -> Double {
    switch self {
    case .ramp: return p
    case .easeOut: return 1 - (1 - p) * (1 - p)
    case .keys(let keys, let eased):
      guard let upper = keys.firstIndex(where: { $0.at >= p }), upper > 0 else { return keys.first?.share ?? 0 }
      let a = keys[upper - 1], b = keys[upper]
      var x = b.at > a.at ? (p - a.at) / (b.at - a.at) : 1
      if eased { x = x * x * (3 - 2 * x) }
      return a.share + (b.share - a.share) * x
    }
  }
}

nonisolated struct LoopKey: Hashable, Sendable {
  var at: Double
  var share: Double

  init(_ at: Double, _ share: Double) { self.at = at; self.share = share }
}

/// The animation behind `Loop`: never finishes, and reads its place from `start` seconds into the period
nonisolated struct LoopAnimation: CustomAnimation {
  var period: Double
  var shape: LoopShape
  var start: Double

  func animate<V: VectorArithmetic>(value: V, time: TimeInterval, context: inout AnimationContext<V>) -> V? {
    let p = ((time + start) / period).truncatingRemainder(dividingBy: 1)
    return value.scaled(by: shape.share(at: p))
  }
}

/// `content` moved between its resting form (`far` false) and the loop's far end by `move`, over and over: a turn is
/// `rotationEffect(.degrees(far ? 360 : 0))` under `.ramp`. `move` may only change animatable values.
/// Restarts cleanly at the wall clock's phase whenever the view returns to screen (navigation pop, scroll reuse) or
/// the app returns to foreground, since UIKit strips layer animations while a view is off-screen while SwiftUI keeps
/// `@State` alive.
struct Loop<Content: View, Moved: View>: View {
  var period: Double
  var shape: LoopShape = .ramp
  @ViewBuilder var content: Content
  var move: (Content, Bool) -> Moved
  @State private var run = 0
  @State private var armed = false
  @Environment(\.scenePhase) private var phase

  var body: some View {
    LoopRun(period: period, shape: shape, content: content, move: move)
      .id(run)
      .transition(.identity)
      .onAppear {
        if armed { restart() }
        armed = true
      }
      .onChange(of: phase) { old, now in
        if old != .active, now == .active, armed { restart() }
      }
  }

  private func restart() {
    var transaction = Transaction()
    transaction.disablesAnimations = true
    withTransaction(transaction) { run += 1 }
  }
}

private struct LoopRun<Content: View, Moved: View>: View {
  var period: Double
  var shape: LoopShape
  var content: Content
  var move: (Content, Bool) -> Moved
  @State private var far = false

  var body: some View {
    move(content, far)
      .onAppear {
        guard !far else { return }
        let start = Date().timeIntervalSinceReferenceDate.truncatingRemainder(dividingBy: period)
        withTransaction(Transaction(animation: Animation(LoopAnimation(period: period, shape: shape, start: start)))) {
          far = true
        }
      }
  }
}
