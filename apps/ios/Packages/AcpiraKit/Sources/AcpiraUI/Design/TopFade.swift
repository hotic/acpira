import AcpiraDesign
import SwiftUI

/// A progressive blur over the top of a scrolling page under system chrome, as Claude's app draws it: what scrolls up
/// under the bar's glass buttons blurs and dims more the nearer it gets to the screen's edge, and is gone a little past
/// the buttons (`Metrics.topFadeTail`). The system's own soft edge effect reached only the status bar's band, so text
/// ran sharp under the buttons; it is hidden here so the two never stack.
///
/// SwiftUI has no variable-radius blur, so the ramp is stacked materials, each masked to fade out a little higher than
/// the one under it (more layers, more blur, towards the top), with the page colour laid over them to dim
struct TopFade: ViewModifier {
  var on = true
  /// How far past the bar the band reaches. A list keeps none, so its first caption or title is never veiled at rest;
  /// the chat's thread scrolls up from the bottom and gets the full tail
  var tail: CGFloat = Metrics.topFadeTail

  func body(content: Content) -> some View {
    if on {
      content
        .scrollEdgeEffectHidden(true, for: .top)
        .overlay(alignment: .top) {
          // The page's content starts under the bar (or the status bar); the band reaches from the screen's top edge
          // down to there and on by the tail. A reader ignoring the safe area reports no insets, so the distance is
          // read from the content's own place on screen
          GeometryReader { g in
            let top = g.frame(in: .global).minY
            TopFadeLayers()
              .frame(height: top + tail)
              .offset(y: -top)
          }
          .allowsHitTesting(false)
        }
    } else {
      content
    }
  }
}

private struct TopFadeLayers: View {
  /// Where each material layer stops (fraction of the band): the first reaches all the way, the last only the top
  private static let reach: [Double] = [1, 0.7, 0.4]

  var body: some View {
    ZStack(alignment: .top) {
      ForEach(Self.reach, id: \.self) { end in
        Rectangle().fill(.ultraThinMaterial)
          .mask(LinearGradient(stops: [.init(color: .black, location: 0), .init(color: .black, location: end * 0.4),
                                       .init(color: .clear, location: end)],
                               startPoint: .top, endPoint: .bottom))
      }
      // The page colour over the blur: the materials carry a light tint of their own (over an empty dark page they read
      // as a grey band), so the page colour nearly covers them where they stack and lets go only towards the band's end
      LinearGradient(stops: [.init(color: Palette.bg0.opacity(0.95), location: 0), .init(color: Palette.bg0.opacity(0.85), location: 0.5),
                             .init(color: Palette.bg0.opacity(0.5), location: 0.8), .init(color: Palette.bg0.opacity(0), location: 1)],
                     startPoint: .top, endPoint: .bottom)
    }
  }
}

extension View {
  /// The progressive top blur (TopFade), when `on`
  func topFade(_ on: Bool = true, tail: CGFloat = Metrics.topFadeTail) -> some View { modifier(TopFade(on: on, tail: tail)) }
}
