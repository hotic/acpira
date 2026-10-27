import AcpiraDesign
import SwiftUI

// ui/ConnectedRail.tsx: one measured line per lead icon, from under the icon to the next icon (or to the end of the
// rail's root), the last one ending on a solid dot. The line fades in over --rail-fade below the icon. Icons in a
// row's lead slot publish their painted span (`RailLeadKey`); a rail root draws the ones under it and stops them
// there, so a nested root's icons never join an outer rail (`lead.closest('.connected-rail') === root`).

/// Which leads a rail takes: its own row only (`body`, the line runs down an indented body), every row under it
/// (`rows`, icon to icon through a result list), or none (`off`, a full-width card body)
public enum RailMode: Sendable {
  case body, rows, off
}

/// A process row's lead icon (toolLine = icon: every process row leads with one)
struct LeadIcon: View {
  let icon: LucideIcon
  init(_ icon: LucideIcon) { self.icon = icon }
  var body: some View { Icon(icon, size: WebMetrics.icon) }
}

struct ConnectedRail: ViewModifier {
  /// Drawn at all; a disabled root still stops the icons under it
  var enabled: Bool
  /// Result rails end at the last row, before anything full-width below it
  var endAtLastRow: Bool
  var color: Color
  @Environment(\.motionEnabled) private var motion

  func body(content: Content) -> some View {
    content
      .backgroundPreferenceValue(RailLeadKey.self) { leads in
        GeometryReader { proxy in
          let segments = Self.segments(leads, in: proxy, endAtLastRow: endAtLastRow)
          ForEach(segments.indices, id: \.self) { i in
            RailSegment(segment: segments[i], color: color)
          }
        }
        // Opening shows the rail at once (it follows the panel as it grows); closing fades it on the close curve
        // while the panel retracts, as `[data-retracting]` does
        .opacity(enabled ? 1 : 0)
        .animation(enabled || !motion ? nil : Motion.close, value: enabled)
        // `.connected-rail:has(> .process-row-enter) > .rail-segment`: enters with the file rows
        .rowEnterFade(Motion.rowEnterFile)
        .allowsHitTesting(false)
      }
      .transformPreference(RailLeadKey.self) { $0 = [] }
  }

  struct Segment {
    var x: CGFloat
    var top: CGFloat
    var bottom: CGFloat
    var terminal: Bool
  }

  static func segments(_ leads: [RailLead], in proxy: GeometryProxy, endAtLastRow: Bool) -> [Segment] {
    let icons = leads.map { lead -> (x: CGFloat, top: CGFloat, bottom: CGFloat) in
      let r = proxy[lead.icon]
      return (r.midX, r.minY + r.height * lead.top, r.minY + r.height * lead.bottom)
    }
    let lastRow = endAtLastRow ? leads.last?.row.map { proxy[$0].maxY } : nil
    var out: [Segment] = []
    for (i, icon) in icons.enumerated() {
      let end = i < icons.count - 1 ? icons[i + 1].top : (lastRow ?? proxy.size.height)
      if end > icon.bottom {
        out.append(Segment(x: icon.x, top: icon.bottom, bottom: end, terminal: i == icons.count - 1))
      }
    }
    return out
  }
}

/// One segment: the line with its faded top, and the end dot hanging off its bottom edge
private struct RailSegment: View {
  let segment: ConnectedRail.Segment
  let color: Color

  var body: some View {
    let length = segment.bottom - segment.top
    // A short icon-to-icon link fades over at most 40 % of itself, or it would stay translucent
    let fade = min(Metrics.railFade, length * 0.4) / length
    Rectangle()
      .fill(LinearGradient(stops: [.init(color: color.opacity(0), location: 0), .init(color: color, location: fade)],
                           startPoint: .top, endPoint: .bottom))
      .frame(width: Metrics.railWidth, height: length)
      .position(x: segment.x, y: segment.top + length / 2)
    if segment.terminal {
      Circle().fill(color)
        .frame(width: Metrics.railDot, height: Metrics.railDot)
        .position(x: segment.x, y: segment.bottom)
    }
  }
}

extension View {
  /// A rail root (ConnectedRail): draws a line from each lead icon under it when `enabled`
  func connectedRail(enabled: Bool, endAtLastRow: Bool = false, action: Bool = true) -> some View {
    modifier(ConnectedRail(enabled: enabled, endAtLastRow: endAtLastRow, color: action ? Palette.railAction : Palette.rail))
  }
}
