import AcpiraDesign
import SwiftUI

// ui/Shimmer.tsx: the running verb. A muted copy stays put and a strong copy shows through a soft window that sweeps
// left to right over --shimmer-cycle. Under motion = none (and reduced motion) the label is plain fg-2.

public struct Shimmer: View {
  @Environment(\.motionEnabled) private var motion
  @Environment(\.accessibilityReduceMotion) private var reduceMotion
  @Environment(\.actionTone) private var action
  let text: String
  var active: Bool

  public init(_ text: String, active: Bool = true) { self.text = text; self.active = active }

  public var body: some View {
    if !active {
      Text(web: text)
    } else if !motion || reduceMotion {
      // --fg-2, which an action row redefines as its own ink
      Text(web: text).foregroundStyle(action ? Palette.actionFg : Palette.fg2)
    } else {
      Text(web: text)
        .foregroundStyle(Palette.shimmerBase)
        .overlay {
          TimelineView(.animation) { context in
            let phase = context.date.timeIntervalSinceReferenceDate
              .truncatingRemainder(dividingBy: Motion.shimmerCycle) / Motion.shimmerCycle
            Text(web: text)
              .foregroundStyle(Palette.fgStrong)
              .mask {
                GeometryReader { geo in
                  LinearGradient(colors: [.clear, .black, .clear], startPoint: .leading, endPoint: .trailing)
                    .frame(width: geo.size.width)
                    .offset(x: (phase * 2 - 1) * geo.size.width)
                }
              }
          }
          .accessibilityHidden(true)
        }
    }
  }
}
