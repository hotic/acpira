import AcpiraDesign
import Foundation
import SwiftUI
import Testing
@testable import AcpiraUI
#if os(iOS)
import UIKit
#endif

// Layouts past the portrait phone: landscape, iPhone Duo and iPad windows (docs/dev/ios.md)
@Suite struct WideLayoutTests {
  @Test func roomCapsDockedCardsByWindowHeight() {
    // Before the first measure the portrait phone's cap stands
    #expect(LayoutRoom().height(200) == 200)
    // A tall window keeps the cap; a landscape phone (about 380 pt inside its safe area) holds it to 35 %
    #expect(LayoutRoom(size: CGSize(width: 402, height: 800), visible: 800).height(200) == 200)
    #expect(LayoutRoom(size: CGSize(width: 874, height: 380), visible: 380).height(200) == 133)
  }

  @Test func roomTrimsComposerLinesOverTheKeyboard() {
    // Unmeasured, or a portrait phone over its keyboard: all eight lines
    #expect(LayoutRoom().lines(8, lineHeight: 22) == 8)
    #expect(LayoutRoom(size: CGSize(width: 402, height: 820), visible: 480).lines(8, lineHeight: 22) == 8)
    // A landscape phone over its keyboard keeps a few, never fewer than two
    #expect(LayoutRoom(size: CGSize(width: 874, height: 380), visible: 160).lines(8, lineHeight: 22) == 3)
    #expect(LayoutRoom(size: CGSize(width: 874, height: 380), visible: 40).lines(8, lineHeight: 22) == 2)
  }

  #if os(iOS)
  @MainActor @Test func barHandoffOnlyOverABottomBar() {
    // A regular width moves the list's search field to the top bar
    #expect(!BarHandoffController.barsAtBottom(UITraitCollection(horizontalSizeClass: .regular)))
    // No vertical bar on a plain compact width (the iOS 27.0 runtime has no verticalBarEdge at all)
    #expect(BarHandoffController.barsAtBottom(UITraitCollection(horizontalSizeClass: .compact)))
  }
  #endif
}
