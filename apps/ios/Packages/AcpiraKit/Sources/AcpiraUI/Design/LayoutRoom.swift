import AcpiraDesign
import SwiftUI

/// The room the screens lay out in, measured over the whole stack: the window inside its safe area, which a rotation,
/// an iPad window and the iPhone Duo's fold all change. Sizes that the portrait phone took as given read it instead
nonisolated struct LayoutRoom: Equatable {
  /// The window inside its safe area, the keyboard left out of account (the webview's dvh)
  var size: CGSize = .zero
  /// The height the keyboard leaves of it
  var visible: CGFloat = 0

  /// `cap`, held to `share` of the window's height once measured
  func height(_ cap: CGFloat, share: CGFloat = Metrics.dockShare) -> CGFloat {
    size.height > 0 ? min(cap, (size.height * share).rounded(.down)) : cap
  }

  /// The lines a growing field may take: `most`, or fewer where the keyboard leaves little (never under two)
  func lines(_ most: Int, lineHeight: CGFloat) -> Int {
    guard visible > 0, lineHeight > 0 else { return most }
    return min(most, max(2, Int(visible * Metrics.composerShare / lineHeight)))
  }
}

extension EnvironmentValues {
  @Entry var room = LayoutRoom()
}

extension View {
  /// Holds the view to the reading column (`Metrics.contentW`), centred in whatever is wider
  func readableWidth() -> some View {
    frame(maxWidth: Metrics.contentW).frame(maxWidth: .infinity)
  }

  /// Measures the room this view fills and hands it to everything inside (`LayoutRoom`)
  func measuresRoom() -> some View { modifier(RoomReader()) }
}

private struct RoomReader: ViewModifier {
  @State private var room = LayoutRoom()

  func body(content: Content) -> some View {
    content
      .environment(\.room, room)
      .background {
        Color.clear
          .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { room.visible = $0 }
      }
      .background {
        Color.clear
          .ignoresSafeArea(.keyboard)
          .onGeometryChange(for: CGSize.self) { $0.size } action: { room.size = $0 }
      }
  }
}
