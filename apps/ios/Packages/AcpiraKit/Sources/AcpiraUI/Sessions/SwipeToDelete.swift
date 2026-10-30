import AcpiraDesign
import SwiftUI
#if canImport(UIKit)
import UIKit
#endif

// The session list's swipe to delete. The list is a lazy stack in a scroll view, which never gets `List`'s swipe
// actions, so the row slides itself: a sideways drag towards the leading edge bares a red Delete at the row's end, and a
// tap on it deletes (the toast's undo brings the session back). A tap on any row, another row's swipe or a scroll puts
// an open row back; one row is open at a time.

/// A list row that bares Delete when swiped towards the leading edge. `open` holds the id of the row standing open,
/// shared by every row of the list
struct SwipeToDelete: ViewModifier {
  let id: String
  @Binding var open: String?
  var onDelete: () -> Void
  /// How far the row has slid: zero at rest, negative while Delete shows
  @State private var offset: CGFloat = 0
  /// The offset the current drag started from
  @State private var origin: CGFloat = 0

  func body(content: Content) -> some View {
    #if os(iOS)
    content
      .offset(x: offset)
      // The overlay keeps the row's own frame, so it covers exactly the room the slide bared. At rest there is none:
      // an empty button left at the row's end could still take a tap meant for the row
      .overlay(alignment: .trailing) { if offset < 0 { action } }
      .gesture(SwipePan(open: open == id, began: began, changed: changed, ended: ended))
      .onChange(of: open) { _, now in
        if now != id && offset != 0 { withAnimation(Motion.swipe) { offset = 0 } }
      }
      // VoiceOver reaches Delete without the gesture
      .accessibilityAction(named: Text(web: t("common.delete")), onDelete)
    #else
    content
    #endif
  }

  /// The bared room: a red capsule as wide as the slide allows, its word fading in as the row reaches its stop
  private var action: some View {
    let width = max(0, -offset)
    return Button {
      open = nil
      onDelete()
    } label: {
      // The system red, as on the system's own swipe actions. The word rides an overlay clipped to the capsule, so its
      // own width never sizes the button past the bared room
      Capsule().fill(Color.red)
        .overlay {
          Text(web: t("common.delete"), paragraph: false)
            .textStyle(TypeScale.text2, weight: .medium)
            .foregroundStyle(Color.white)
            .lineLimit(1)
            .fixedSize()
            .opacity(min(1, width / Metrics.swipeAction))
        }
        .clipShape(Capsule())
        .padding(.vertical, Metrics.s2)
        .padding(.leading, Metrics.s2)
        .padding(.trailing, Metrics.pad)
        .contentShape(Rectangle())
    }
    .buttonStyle(.plain)
    .frame(width: width)
    .clipped()
    .accessibilityHidden(true)
  }

  private func began() {
    origin = offset
    // Starting this row's swipe puts any other open row back
    open = id
  }

  private func changed(_ dx: CGFloat) {
    let raw = min(0, origin + dx)
    let stop = Metrics.swipeAction
    // Past its stop the row resists, as UIKit's swipe actions do: the extra drag eases towards one more stop's width
    offset = raw < -stop ? -stop - Self.resist(-raw - stop, over: stop) : raw
  }

  private func ended(_ velocity: CGFloat) {
    // A flick decides by its direction; a slow release by whether the row passed half its stop
    let opens = velocity < -Motion.swipeFlick || (velocity < Motion.swipeFlick && -offset > Metrics.swipeAction / 2)
    withAnimation(Motion.swipe) { offset = opens ? -Metrics.swipeAction : 0 }
    if !opens && open == id { open = nil }
  }

  /// Rubber banding: `extra` drag past the stop moves the row less and less, never past `limit`
  private static func resist(_ extra: CGFloat, over limit: CGFloat) -> CGFloat {
    extra * limit / (extra + limit)
  }
}

#if os(iOS)
/// A pan that begins only on a sideways drag, and towards the leading edge unless the row stands open: an upward or
/// downward drag still scrolls the list, and a tap or a long press still reaches the row's button and menu
struct SwipePan: UIGestureRecognizerRepresentable {
  var open: Bool
  var began: () -> Void
  var changed: (CGFloat) -> Void
  /// Called with the horizontal velocity on release (zero when the system cancels the drag)
  var ended: (CGFloat) -> Void

  func makeCoordinator(converter: CoordinateSpaceConverter) -> Coordinator { Coordinator() }

  func makeUIGestureRecognizer(context: Context) -> UIPanGestureRecognizer {
    let pan = UIPanGestureRecognizer()
    pan.delegate = context.coordinator
    return pan
  }

  func updateUIGestureRecognizer(_ recognizer: UIPanGestureRecognizer, context: Context) {
    context.coordinator.open = open
  }

  func handleUIGestureRecognizerAction(_ recognizer: UIPanGestureRecognizer, context: Context) {
    switch recognizer.state {
    case .began:
      began()
      changed(recognizer.translation(in: recognizer.view).x)
    case .changed:
      changed(recognizer.translation(in: recognizer.view).x)
    case .ended:
      ended(recognizer.velocity(in: recognizer.view).x)
    case .cancelled, .failed:
      ended(0)
    default:
      break
    }
  }

  final class Coordinator: NSObject, UIGestureRecognizerDelegate {
    var open = false

    func gestureRecognizerShouldBegin(_ recognizer: UIGestureRecognizer) -> Bool {
      guard let pan = recognizer as? UIPanGestureRecognizer else { return false }
      let velocity = pan.velocity(in: pan.view)
      return abs(velocity.x) > abs(velocity.y) && (velocity.x < 0 || open)
    }
  }
}
#endif
