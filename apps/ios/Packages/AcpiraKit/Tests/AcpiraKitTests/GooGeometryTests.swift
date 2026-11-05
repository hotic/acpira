import AcpiraDesign
import SwiftUI
import Testing
@testable import AcpiraUI

@MainActor struct GooGeometryTests {
  private let chip = CGSize(width: Metrics.barCtl, height: Metrics.barCtl)

  private func pieces() -> [GooPiece] {
    let offsets = GooColumn.offsets(chip: chip, widths: [chip.width, chip.width])
    return offsets.enumerated().map { i, offset in
      GooPiece(id: "piece-\(i)", name: "Choice", offset: offset, glyph: AnyView(EmptyView()),
               panel: GooPanel(size: CGSize(width: Metrics.gooPanelW, height: Metrics.gooRow * 6),
                               content: AnyView(EmptyView())), action: {})
    }
  }

  @Test func aPanelNeverBecomesAnEmptyLensBetweenItsRowsAndGlyph() {
    // Both directions use these same intermediate frames. Previously rows ended at 0.55 and the glyph began
    // below 0.3, leaving a large, blank glass rectangle in between on every model or effort pick.
    for step in 1..<100 {
      let progress = GooProgress(values: [1, 0, CGFloat(step) / 100])
      let frame = GooGeometry.frames(pieces(), progress, chip: chip, seed: chip.height / 2)[1]
      #expect(frame.glyph + frame.rows > 0.1)
    }
  }

  @Test func aClosingBeanShrinksWhileItTravelsHome() {
    let half = GooGeometry.frames(pieces(), GooProgress(values: [0.5, 0, 0]), chip: chip, seed: chip.height / 2)
    #expect(half[0].size.height < chip.height)
    #expect(half[0].glyph > 0)
    // At rest none of the glass escapes the chip, including a fully swallowed panel.
    let closed = GooGeometry.frames(pieces(), GooProgress(values: [0, 0, 0]), chip: chip, seed: chip.height / 2)
    #expect(closed.allSatisfy { $0.size == .zero })
  }
}
