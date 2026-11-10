#if canImport(UIKit)
import UIKit
#endif

// The app's haptic vocabulary. Views name the moment (`Haptics.play(.send)`), never a generator or a style: what each
// moment feels like is decided once, in `Haptics.play`, after Apple's HIG (selection for a value changing in a picker,
// impact for a physical event such as a commit, a detent or a drop, notification for the outcome of a task). Sparse on
// purpose: nothing plays on streamed output, plain navigation, or where a system control already plays its own (Toggle,
// system Menu and context menus, a long press lifting a row, List reordering, pull to refresh, the keyboard).
// There is no in-app switch: the system's own haptics setting governs, and generators stay silent off iPhone.
// Calls are imperative rather than `sensoryFeedback(trigger:)`: the transcript's cells are hosted in a UICollectionView,
// where neither SwiftUI's environment nor observation-driven redraws can be relied on, and a call there behaves the same.

/// A moment that plays haptic feedback (Android's counterpart: `ui/Haptics.kt`)
public enum Haptic: CaseIterable, Sendable {
  /// A prompt, or a question's answers, leave the phone
  case send
  /// The running turn, or one of its background tasks, is told to stop
  case stop
  /// A permission or a plan is allowed
  case approve
  /// A permission is rejected
  case reject
  /// A value changes in one of the app's own pickers (the composer's beans, the agent ring, a question's options)
  case pick
  /// A drag passes a detent: a swiped row reaches its stop (`Detent`)
  case detent
  /// A dragged row lands on its drop target
  case drop
  /// A button put text on the pasteboard
  case copy
  /// The turn on screen finished
  case finished
  /// The turn on screen ended in an error, or a pairing was refused
  case failed
  /// A computer paired
  case paired
}

@MainActor
public enum Haptics {
  /// Plays `haptic`; a no-op where the platform has no haptics (the macOS test build)
  public static func play(_ haptic: Haptic) {
    #if canImport(UIKit)
    switch haptic {
    case .pick: selection.selectionChanged()
    case .send, .copy: impact(.light)
    case .approve, .detent: impact(.medium)
    case .reject, .stop: impact(.rigid)
    case .drop, .finished: impact(.soft)
    case .failed: notification.notificationOccurred(.error)
    case .paired: notification.notificationOccurred(.success)
    }
    #endif
  }

  #if canImport(UIKit)
  private static let selection = UISelectionFeedbackGenerator()
  private static let notification = UINotificationFeedbackGenerator()
  private static var impacts: [UIImpactFeedbackGenerator.FeedbackStyle: UIImpactFeedbackGenerator] = [:]

  private static func impact(_ style: UIImpactFeedbackGenerator.FeedbackStyle) {
    let generator = impacts[style] ?? UIImpactFeedbackGenerator(style: style)
    impacts[style] = generator
    generator.impactOccurred()
  }
  #endif
}

/// A detent a drag can pass: `cross` says when to play `.detent`, once per crossing outwards, never for a drag that
/// starts already past it nor while the finger stays past it
public struct Detent: Sendable {
  private var past: Bool

  /// `past`: where the drag starts (a row that already stands open)
  public init(past: Bool = false) { self.past = past }

  /// Moves to `now` (past the detent or not); true when this move crossed it outwards
  public mutating func cross(_ now: Bool) -> Bool {
    defer { past = now }
    return now && !past
  }
}
