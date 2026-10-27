import SwiftUI

// The anchors ui/ConnectedRail.tsx measures: a row's lead icon, where its painted outline starts and ends on the
// icon's centre column (an open outline such as Brain or Pencil would leave a gap under its bounding box), and the
// row it leads. Every icon offers itself (`IconAnchorKey`); a row's lead slot turns the icons in it into rail leads
// (`railLeadSlot`), and a rail root collects the leads it owns. The slot does this rather than an environment flag
// because Group(subviews:) hands the row its lead as resolved subviews that keep the environment they were built in.

/// One lead icon of a rail: the icon's frame, the painted span on its centre column as fractions of that frame,
/// and the row it leads (filled in by the row)
public struct RailLead {
  public var icon: Anchor<CGRect>
  public var top: CGFloat
  public var bottom: CGFloat
  public var row: Anchor<CGRect>?
}

public struct RailLeadKey: PreferenceKey {
  public static var defaultValue: [RailLead] { [] }
  public static func reduce(value: inout [RailLead], nextValue: () -> [RailLead]) { value.append(contentsOf: nextValue()) }
}

/// Every drawn icon: a rail lead if it sits in a row's lead slot
public struct IconAnchorKey: PreferenceKey {
  public static var defaultValue: [RailLead] { [] }
  public static func reduce(value: inout [RailLead], nextValue: () -> [RailLead]) { value.append(contentsOf: nextValue()) }
}

extension View {
  /// A row's lead slot: the icons drawn in it become rail leads, and stop offering themselves further up
  public func railLeadSlot() -> some View {
    backgroundPreferenceValue(IconAnchorKey.self) { icons in
      Color.clear.preference(key: RailLeadKey.self, value: icons)
    }
    .transformPreference(IconAnchorKey.self) { $0 = [] }
  }
}

extension LucideIcon {
  /// The painted span on the centre column (x = 12) of the 24-unit box at `strokeWidth`, as fractions of the box:
  /// the same 1/8-unit scan ConnectedRail runs with isPointInStroke / isPointInFill
  public func railSpan(strokeWidth: CGFloat = 1.5) -> (top: CGFloat, bottom: CGFloat) {
    RailSpanCache.span(self, strokeWidth)
  }
}

private enum RailSpanCache {
  struct Key: Hashable { var icon: LucideIcon; var stroke: CGFloat }
  nonisolated(unsafe) static var spans: [Key: (CGFloat, CGFloat)] = [:]
  static let lock = NSLock()

  static func span(_ icon: LucideIcon, _ stroke: CGFloat) -> (top: CGFloat, bottom: CGFloat) {
    lock.lock(); defer { lock.unlock() }
    let key = Key(icon: icon, stroke: stroke)
    if let hit = spans[key] { return hit }
    let painted = icon.path.strokedPath(StrokeStyle(lineWidth: stroke, lineCap: .round, lineJoin: .round))
    var top: CGFloat?
    var bottom: CGFloat?
    var y: CGFloat = 0
    while y <= 24 {
      if painted.contains(CGPoint(x: 12, y: y)) {
        top = top ?? y
        bottom = y
      }
      y += 0.125
    }
    let result = ((top ?? 0) / 24, (bottom ?? 24) / 24)
    spans[key] = result
    return result
  }
}
