import AcpiraDesign
import SwiftUI

// ui/Disclosure.tsx + ui/Collapsible: a row that opens a body under it. The body reveals by height (clipped, the way
// the Collapsible panel animates its grid row): --dur-open ease-out when opening, --dur-close on the close curve.

/// Height reveal between 0 and the content's natural height; at rest open the content sizes itself
struct Reveal: ViewModifier, Animatable {
  var progress: Double
  var animatableData: Double {
    get { progress }
    set { progress = newValue }
  }
  @State private var natural: CGFloat = 0

  func body(content: Content) -> some View {
    if progress <= 0 {
      Color.clear.frame(height: 0)
    } else {
      content
        .fixedSize(horizontal: false, vertical: true)
        .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { natural = $0 }
        .frame(height: progress >= 1 ? nil : natural * progress, alignment: .top)
        .clipped()
        // The phone fades the body in over the first part of the reveal, as the system's disclosures do
        .opacity(Reading.phone ? min(1, progress * 2) : 1)
    }
  }
}

extension View {
  /// Collapses to zero height when closed; animate the change with `Motion.open` / `Motion.close`
  func reveal(_ open: Bool) -> some View { modifier(Reveal(progress: open ? 1 : 0)) }
}

/// Folds a screenshot or a preview opens before anyone taps (`-render all`); a tap still wins
public enum PreOpen: Hashable, Sendable {
  case fold, thought, plan, command, edit, results

  /// The render modes: `open` the last process fold, `all` plus thoughts and plans, `every` plus commands, edits and
  /// result folds (groups, search hits, list bodies)
  public static func folds(_ mode: String?) -> Set<PreOpen> {
    switch mode {
    case "open": [.fold]
    case "all": [.fold, .thought, .plan]
    case "every": [.fold, .thought, .plan, .command, .edit, .results]
    default: []
    }
  }
}

extension EnvironmentValues {
  @Entry public var preOpen: Set<PreOpen> = []
}

/// A fold's state: the reader's choice once they tapped, the pre-opened default before
func foldBinding(_ manual: Binding<Bool?>, _ fallback: Bool) -> Binding<Bool> {
  Binding(get: { manual.wrappedValue ?? fallback }, set: { manual.wrappedValue = $0 })
}

/// Keeps a tapped fold's row where it is: the thread trades its bottom pin for the offset it is at before the fold
/// animates (`Thread`), so the body grows downward instead of pushing everything above it up
public struct HoldScroll: Sendable {
  let action: @MainActor @Sendable () -> Void
  @MainActor func callAsFunction() { action() }
}

extension EnvironmentValues {
  @Entry var holdScroll = HoldScroll {}
}

/// The two curves a fold uses, picked by direction
func foldAnimation(opening: Bool, motion: Bool) -> Animation? {
  motion ? (opening ? Motion.open : Motion.close) : nil
}

/// An expandable row. The body is indented to the label column when the row has a lead (`indent`), and a rail runs
/// from the lead icon: down the body (`rail: .body`, the default for an indented body) or icon to icon through the
/// rows under it (`.rows`). Full-width bodies (cards, lists) take `indent: false` and no rail. Without a rail the
/// body keeps `pb-1.5` below it. `root: false` is the edit group's entry fold, which has no rail root of its own,
/// so the group's rail keeps running through its icon
public struct Disclosure<Label: View, Trailing: View, Content: View>: View {
  @Environment(\.motionEnabled) private var motion
  @Environment(\.holdScroll) private var holdScroll
  @State private var inner: Bool
  private var controlled: Binding<Bool>?
  var tone: RowTone
  var dense: Bool
  var icon: LucideIcon?
  var indent: Bool
  var rail: RailMode
  var root: Bool
  var bodyTop: CGFloat
  let label: () -> Label
  let trailing: () -> Trailing
  let content: () -> Content

  /// `rail` defaults to `.body` for an indented body and `.off` otherwise (`rail = indent ? 'body' : false`)
  public init(open: Binding<Bool>? = nil, defaultOpen: Bool = false, tone: RowTone = .action, dense: Bool = false,
              icon: LucideIcon? = nil, indent: Bool = true, rail: RailMode? = nil, root: Bool = true,
              bodyTop: CGFloat = Metrics.s1,
              @ViewBuilder label: @escaping () -> Label, @ViewBuilder trailing: @escaping () -> Trailing,
              @ViewBuilder content: @escaping () -> Content) {
    controlled = open
    _inner = State(initialValue: defaultOpen)
    self.tone = tone; self.dense = dense; self.icon = icon; self.indent = indent
    self.rail = rail ?? (indent ? .body : .off)
    self.root = root; self.bodyTop = bodyTop
    self.label = label; self.trailing = trailing; self.content = content
  }

  private var isOpen: Bool { controlled?.wrappedValue ?? inner }

  private func toggle() {
    let next = !isOpen
    holdScroll()
    withAnimation(foldAnimation(opening: next, motion: motion)) {
      inner = next
      controlled?.wrappedValue = next
    }
  }

  public var body: some View {
    let lead = icon
    let column = VStack(alignment: .leading, spacing: 0) {
      Button {
        toggle()
      } label: {
        Row(tone: tone, dense: dense) {
          if let lead { Icon(lead, size: WebMetrics.icon) }
        } content: {
          label()
        } trailing: {
          trailing()
        }
      }
      .buttonStyle(RowButtonStyle())
      .accessibilityAddTraits(.isButton)
      .accessibilityValue(isOpen ? Text(web: "expanded") : Text(web: "collapsed"))
      .foldProbe(toggle)

      Group {
        // A body rail anchors on this row's own icon only; rows inside the body are not its stops
        if rail == .body {
          content().transformPreference(RailLeadKey.self) { $0 = [] }
        } else {
          content()
        }
      }
        .padding(.top, bodyTop)
        .padding(.bottom, rail == .off ? Metrics.s1_5 : 0)
        .padding(.leading, indent && lead != nil ? WebMetrics.indent : 0)
        .frame(maxWidth: .infinity, alignment: .leading)
        .reveal(isOpen)
    }
    if root {
      column.connectedRail(enabled: rail != .off && isOpen && lead != nil, endAtLastRow: rail == .rows, action: tone == .action)
    } else {
      column
    }
  }
}

extension Disclosure where Trailing == EmptyView {
  public init(open: Binding<Bool>? = nil, defaultOpen: Bool = false, tone: RowTone = .action, dense: Bool = false,
              icon: LucideIcon? = nil, indent: Bool = true, rail: RailMode? = nil, root: Bool = true,
              bodyTop: CGFloat = Metrics.s1,
              @ViewBuilder label: @escaping () -> Label, @ViewBuilder content: @escaping () -> Content) {
    self.init(open: open, defaultOpen: defaultOpen, tone: tone, dense: dense, icon: icon, indent: indent, rail: rail,
              root: root, bodyTop: bodyTop, label: label, trailing: { EmptyView() }, content: content)
  }
}

/// `-popup fold:N` (debug builds): the Nth fold to appear after launch is tapped twice, open then closed, so a
/// recording shows the reveal both ways (scripted taps do not reach the simulator)
enum FoldProbe {
  nonisolated(unsafe) static var seen = 0
}

private struct FoldProbeTap: ViewModifier {
  let toggle: () -> Void
  @Environment(\.initialPopup) private var initialPopup
  @State private var index: Int?

  func body(content: Content) -> some View {
    #if DEBUG
    content.task {
      guard index == nil, let popup = initialPopup, popup.hasPrefix("fold:"),
            let target = Int(popup.dropFirst("fold:".count)) else { return }
      FoldProbe.seen += 1
      index = FoldProbe.seen
      guard index == target else { return }
      try? await Task.sleep(for: .seconds(2))
      toggle()
      try? await Task.sleep(for: .seconds(2))
      toggle()
    }
    #else
    content
    #endif
  }
}

extension View {
  fileprivate func foldProbe(_ toggle: @escaping () -> Void) -> some View { modifier(FoldProbeTap(toggle: toggle)) }
}

/// The fold chevron: ChevronRight size-3 rotated 90° when open
struct FoldChevron: View {
  var open: Bool
  var body: some View {
    Icon(.chevronRight, size: WebMetrics.chevron, strokeWidth: 1.75)
      .rotationEffect(.degrees(open ? 90 : 0))
  }
}
