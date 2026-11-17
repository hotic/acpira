import AcpiraDesign
import AcpiraMarkdown
@testable import AcpiraUI
import SwiftUI
import Testing

/// Per laid-out line: its glyphs, the ones on a code chip, and the chip's width
private final class LineLog: @unchecked Sendable {
  var lines: [(glyphs: Int, chip: Int, chipWidth: CGFloat)] = []
}

@MainActor private struct LineRecorder: TextRenderer {
  let log: LineLog

  func draw(layout: Text.Layout, in ctx: inout GraphicsContext) {
    log.lines = layout.map { line in
      let chips = line.filter { $0[InlineCodeChip.self] != nil }
      return (line.reduce(0) { $0 + $1.count }, chips.reduce(0) { $0 + $1.count },
              chips.reduce(CGRect.null) { $0.union($1.typographicBounds.rect) }.width)
    }
    for line in layout { ctx.draw(line) }
  }
}

@MainActor struct InlineCodeWrapTests {
  private func lines(_ markdown: String, width: CGFloat) throws -> [(glyphs: Int, chip: Int, chipWidth: CGFloat)] {
    guard case .paragraph(let inlines) = Markdown.parse(markdown).first else { throw CancellationError() }
    let text = InlineBuilder(style: WebType.text1, ink: .white, strongInk: .white, accent: .blue).paragraph(inlines)
    let log = LineLog()
    let view = text.lineBox(WebType.text1).textRenderer(LineRecorder(log: log))
      .frame(width: width, alignment: .leading).fixedSize(horizontal: false, vertical: true)
    _ = try #require(ImageRenderer(content: view).cgImage)
    return log.lines
  }

  /// A code span wider than the line wraps inside itself: its leading pad never stays behind as an empty chip, alone
  /// on a line (a paragraph opening with the code) or after the text before it
  @Test(arguments: [
    "`apps/ios/Packages/AcpiraKit/Sources/AcpiraUI/Pairing/PairingModel.swift`：加了一行 `wiredAt = nil`，附注释。",
    "`PairingModelTests.swift`：新增一个测试 `aSwitchToAComputerThatDoesNotAnswerShowsOffline`，并让测试用的假服务支持第二台电脑。",
    "see `aSwitchToAComputerThatDoesNotAnswerShowsOfflineAtOnceWithoutRedials` for the case",
  ])
  func aLongCodeSpanLeavesNoEmptyChip(markdown: String) throws {
    let laid = try lines(markdown, width: 300)
    #expect(laid.count > 2)
    for line in laid where line.chip > 0 {
      #expect(line.chip > 1, "a line holds a chip of padding alone: \(laid)")
    }
  }

  /// The pads keep CSS's `.4em` on each side of the code (a mono glyph is `.6em`)
  @Test func aShortCodeSpanKeepsItsPadding() throws {
    let laid = try lines("run `abc` now", width: 300)
    let size = WebType.mono.size
    #expect(laid.count == 1)
    #expect(abs((laid.first?.chipWidth ?? 0) - size * (3 * 0.6 + 2 * 0.4)) < 0.5)
  }
}
