import AcpiraDesign
import SwiftUI

/// Fades scrolling content into the page before it reaches the bar's glass buttons (`Metrics.topFadeTail`).
/// Only the page colour is overlaid: materials tint even an empty background and leave a lighter band at the top.
/// The system's own edge effect is hidden so it cannot add another tinted layer.
struct TopFade: ViewModifier {
  /// How far past the bar the fade reaches into the scrolling content
  var tail: CGFloat = Metrics.topFadeTail

  func body(content: Content) -> some View {
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
  }
}

private struct TopFadeLayers: View {
  var body: some View {
    LinearGradient(stops: [.init(color: Palette.bg0, location: 0), .init(color: Palette.bg0, location: 0.5),
                           .init(color: Palette.bg0.opacity(0.5), location: 0.8), .init(color: Palette.bg0.opacity(0), location: 1)],
                   startPoint: .top, endPoint: .bottom)
  }
}

extension View {
  /// The page-colour top fade (TopFade)
  func topFade(tail: CGFloat = Metrics.topFadeTail) -> some View { modifier(TopFade(tail: tail)) }
}
