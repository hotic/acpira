import AcpiraDesign
import CoreGraphics
import CoreText
import Foundation
import SwiftUI
import Testing

@MainActor struct CJKLayoutTests {
  /// An unbroken CJK passage must not turn into one attributed run per ideograph and invisible break. SwiftUI
  /// resolves each run while a historical message's cell is measured on the scrolling frame.
  @Test func breakOpportunitiesDoNotSplitEveryCharacterIntoAnotherRun() {
    let source = String(repeating: "历史消息保持原来的字宽和换行", count: 20)
    let text = CJKWidth.attributed(source, size: WebType.text1.size)
    #expect(String(text.characters) == CJKWidth.breakOpportunities(source) + CJKWidth.paragraphEnd)
    #expect(text.runs.count == 2) // One kerned passage and the unchanged paragraph-end marker.
  }

  /// Folded one part at a time, every part nested the text a level deeper and resolving it recursed through each
  /// level: a command output of a few hundred lines overflowed the phone's 1 MB main-thread stack, and this many parts
  /// overflow a Mac's 8 MB one. `Text.joined` keeps the depth at log2 of the count
  @Test func joinedTextOfManyPartsRenders() throws {
    let text = Text.joined((0..<10_000).map { Text(web: "\($0)\n", paragraph: false) })
    let image = try #require(ImageRenderer(content: text.fixedSize()).cgImage)
    #expect(image.height > 0)
  }

  /// Coalescing a zero-width break is safe only when it leaves glyph positions, wrapping and Latin advances intact.
  /// Compare with the original per-character kern assignment, including punctuation, supplementary Han and kana.
  @Test(arguments: [
    "中文English 中\u{200B}文", "（中文），测试。Latin words", "\u{200B}中文\u{200B}",
    "かな한글中文abc", "𠮷野家和中文、emoji 👨‍👩‍👧‍👦", "短句", "",
  ])
  func coalescedRunsKeepCoreTextPositionsAndWrapping(source: String) throws {
    let size = WebType.text1.size
    let current = CJKWidth.attributed(source, size: size)
    var previous = AttributedString(String(current.characters))
    var index = previous.startIndex
    while index < previous.endIndex {
      let next = previous.characters.index(after: index)
      if let scalar = previous.characters[index].unicodeScalars.first, CJKWidth.widened(scalar) {
        previous[index..<next].kern = size * CJKWidth.extra
      }
      index = next
    }
    let old = native(previous, size: size), new = native(current, size: size)
    let oldLine = CTLineCreateWithAttributedString(old), newLine = CTLineCreateWithAttributedString(new)
    #expect(abs(CTLineGetTypographicBounds(oldLine, nil, nil, nil) - CTLineGetTypographicBounds(newLine, nil, nil, nil)) < 0.001)
    for offset in 0..<old.length {
      #expect(abs(CTLineGetOffsetForStringIndex(oldLine, offset, nil) - CTLineGetOffsetForStringIndex(newLine, offset, nil)) < 0.001)
    }
    let oldTypesetter = CTTypesetterCreateWithAttributedString(old), newTypesetter = CTTypesetterCreateWithAttributedString(new)
    for width in [80.0, 160, 360] {
      var start = 0
      while start < old.length {
        let count = CTTypesetterSuggestLineBreak(oldTypesetter, start, width)
        #expect(count == CTTypesetterSuggestLineBreak(newTypesetter, start, width))
        guard count > 0 else { break }
        start += count
      }
      guard !source.isEmpty else { continue }
      let oldImage = try render(previous, width: width), newImage = try render(current, width: width)
      #expect(oldImage.width == newImage.width && oldImage.height == newImage.height)
      let oldPixels = try #require(oldImage.dataProvider?.data) as Data
      let newPixels = try #require(newImage.dataProvider?.data) as Data
      let samePixels = oldPixels == newPixels
      #expect(samePixels)
    }
  }

  private func render(_ text: AttributedString, width: Double) throws -> CGImage {
    let renderer = ImageRenderer(content: Text(text).textStyle(WebType.text1)
      .foregroundStyle(.white).frame(width: width, alignment: .leading).fixedSize(horizontal: false, vertical: true))
    return try #require(renderer.cgImage)
  }

  private func native(_ text: AttributedString, size: CGFloat) -> NSAttributedString {
    let result = NSMutableAttributedString(string: String(text.characters), attributes: [
      NSAttributedString.Key(kCTFontAttributeName as String): InterFont.ctFont(size: size, weight: .regular),
    ])
    for run in text.runs {
      if let kern = run.kern {
        result.addAttribute(NSAttributedString.Key(kCTKernAttributeName as String), value: kern,
                            range: NSRange(run.range, in: text))
      }
    }
    return result
  }
}
