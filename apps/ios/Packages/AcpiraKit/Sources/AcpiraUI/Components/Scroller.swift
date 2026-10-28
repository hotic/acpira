import SwiftUI
#if canImport(UIKit)
import UIKit
#endif

// Containers a full-length render can draw. ImageRenderer cannot draw the UIKit views behind ScrollView, Menu or
// TextField, so under `\.snapshot` a scroller lays its content out at its ideal size along the scrolling axes,
// clipped to the box a ScrollView would have taken, and a menu draws only its label.

extension EnvironmentValues {
  /// Set by the full-length render: no UIKit-backed views (scrollers show their first viewport, menus their label)
  @Entry public var snapshot = false
}

public struct Scroller<Content: View>: View {
  let axes: Axis.Set
  let content: Content
  @Environment(\.snapshot) private var snapshot

  public init(_ axes: Axis.Set = .vertical, @ViewBuilder content: () -> Content) {
    self.axes = axes
    self.content = content()
  }

  public var body: some View {
    if snapshot {
      OverflowLayout(axes: axes) { content }.clipped()
    } else if axes == .horizontal {
      // A sideways strip inside the thread (table, code, math) stops dead at its ends: pulled past them it showed
      // empty room beside the content, as if the table had slid out of its frame
      ScrollView(axes, showsIndicators: false) { content.background(SideBounceOff()) }
    } else {
      ScrollView(axes, showsIndicators: false) { content }
    }
  }
}

/// Turns off the sideways bounce of the scroll view it sits in. SwiftUI only offers `.basedOnSize`, which still
/// bounces whenever the content overflows
private struct SideBounceOff: View {
  var body: some View {
    #if canImport(UIKit)
    Probe().allowsHitTesting(false).accessibilityHidden(true)
    #else
    Color.clear
    #endif
  }

  #if canImport(UIKit)
  private struct Probe: UIViewRepresentable {
    func makeUIView(context: Context) -> Finder { Finder() }
    func updateUIView(_ view: Finder, context: Context) {}
  }

  /// Walks up to the nearest scroll view, the strip's own, once it is in the window
  final class Finder: UIView {
    override func didMoveToWindow() {
      super.didMoveToWindow()
      var view = superview
      while let v = view, !(v is UIScrollView) { view = v.superview }
      guard let scroll = view as? UIScrollView else { return }
      scroll.bouncesHorizontally = false
      scroll.alwaysBounceHorizontal = false
    }
  }
  #endif
}

/// ScrollView's sizing without the scrolling: along a scrolling axis the child gets no size limit and the box takes
/// the proposed length (the child's own length when none is proposed); the child sits at the top leading corner
struct OverflowLayout: Layout {
  let axes: Axis.Set

  private func inner(_ proposal: ProposedViewSize) -> ProposedViewSize {
    ProposedViewSize(width: axes.contains(.horizontal) ? nil : proposal.width,
                     height: axes.contains(.vertical) ? nil : proposal.height)
  }

  func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
    guard let child = subviews.first else { return .zero }
    let size = child.sizeThatFits(inner(proposal))
    return CGSize(width: axes.contains(.horizontal) ? proposal.width ?? size.width : size.width,
                  height: axes.contains(.vertical) ? proposal.height ?? size.height : size.height)
  }

  func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
    let fill = ProposedViewSize(width: axes.contains(.horizontal) ? nil : bounds.width,
                                height: axes.contains(.vertical) ? nil : bounds.height)
    subviews.first?.place(at: bounds.origin, anchor: .topLeading, proposal: fill)
  }
}

/// A system menu; in a snapshot its label as a plain button, so the button style still draws it
public struct PopupMenu<Content: View, Label: View>: View {
  let content: Content
  let label: Label
  @Environment(\.snapshot) private var snapshot

  public init(@ViewBuilder content: () -> Content, @ViewBuilder label: () -> Label) {
    self.content = content()
    self.label = label()
  }

  public var body: some View {
    if snapshot {
      Button {} label: { label }
    } else {
      Menu { content } label: { label }
    }
  }
}
