import AcpiraDesign
import AcpiraModels
import Foundation
import SwiftUI
import Testing
@testable import AcpiraUI
#if canImport(UIKit)
import UIKit
#endif

@MainActor struct DiffRenderingTests {
  private var largeDiff: [DiffLine] {
    [DiffLine(kind: .hunk, text: "@@ -1,1321 +1,6 @@")]
      + (1...1321).map { (i: Int) in DiffLine(kind: .del, text: "-Removed line \(i)", oldLine: i) }
      + (1...6).map { (i: Int) in DiffLine(kind: .add, text: "+Replacement \(i)", newLine: i) }
  }

  @Test func cardRendersOnlyThePreviewWithoutLosingTheCopy() throws {
    let lines = largeDiff
    let preview = diffPreview(lines)
    #expect(preview.count < lines.count)
    #expect(preview.first == lines.first)
    #expect(diffCopyText(lines, nil) == (1...6).map { "Replacement \($0)" }.joined(separator: "\n"))

    // Render the actual card, not just its prefix helper: the hidden tail must not affect layout or pixels.
    func render(_ rows: [DiffLine]) throws -> CGImage {
      let view = DiffBlockView(lines: rows, title: "notes.txt")
        .environment(\.snapshot, true)
        .frame(width: 402)
      return try #require(ImageRenderer(content: view).cgImage)
    }
    let full = try render(lines), cut = try render(preview)
    #expect(full.height == cut.height)
    #expect(full.height <= Int(WebMetrics.codeOutputMax + Metrics.hairline * 2))
    #expect(full.dataProvider?.data as Data? == cut.dataProvider?.data as Data?)
  }

  @Test(arguments: [false, true])
  func editRailReachesTheExpandedCard(grouped: Bool) throws {
    let block = try JSONDecoder().decode(ToolCallBlock.self, from: Data(#"""
      {"type":"tool_call","id":"edit-rail","title":"Edit notes.txt","kind":"edit","status":"completed","verb":"Edited",
       "target":"notes.txt","content":{"type":"diff","lines":[
         {"kind":"add","text":"+first","newLine":1},
         {"kind":"add","text":"+second","newLine":2},
         {"kind":"add","text":"+third","newLine":3},
         {"kind":"add","text":"+fourth","newLine":4}]}}
      """#.utf8))
    let view = Group {
      if grouped { ToolGroupView(kind: .edit, blocks: [block]) }
      else { ToolCallView(block: block) }
    }
      .environment(\.preOpen, [.results, .edit])
      .environment(\.motionEnabled, false)
      .environment(\.snapshot, true)
      .frame(width: 402)
    let image = try #require(ImageRenderer(content: view).cgImage)
    // Below every heading, the icon column is transparent except for the card's rail.
    let strip = try #require(image.cropping(to: CGRect(x: 0, y: image.height - Int(Metrics.codeCardGap) - 12,
                                                     width: Int(WebMetrics.icon), height: 1)))
    var rgba = [UInt8](repeating: 0, count: strip.width * 4)
    let hasRail = rgba.withUnsafeMutableBytes { bytes -> Bool in
      guard let context = CGContext(data: bytes.baseAddress, width: strip.width, height: 1, bitsPerComponent: 8,
                                    bytesPerRow: strip.width * 4, space: CGColorSpaceCreateDeviceRGB(),
                                    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return false }
      context.draw(strip, in: CGRect(x: 0, y: 0, width: strip.width, height: 1))
      return stride(from: 3, to: bytes.count, by: 4).contains { bytes[$0] > 0 }
    }
    #expect(hasRail, "The expanded diff must keep its rail below the file heading")
  }

  #if canImport(UIKit)
  @Test(.serialized, arguments: [false, true])
  func fullDiffHasOneBoundedScrollableViewport(short: Bool) async throws {
    var lines = short ? Array(largeDiff.prefix(4)) : largeDiff
    // A wide tail must become horizontally reachable even when it was not in the initial lazy viewport.
    lines[lines.count - 1].text += String(repeating: " wide", count: 60)
    let sheet = CodeSheet(title: "notes.txt", copy: diffCopyText(lines, nil), copyLabel: "Copy",
                          wrapInScrollView: false, edgeToEdge: true) {
      DiffBlockView(lines: lines, title: "notes.txt")
    }
    let host = UIHostingController(rootView: sheet)
    let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 402, height: 874))
    window.rootViewController = host
    window.makeKeyAndVisible()
    defer { window.isHidden = true; window.rootViewController = nil }
    host.view.layoutIfNeeded()
    try await Task.sleep(for: .milliseconds(150))
    host.view.layoutIfNeeded()

    func scrollers(_ view: UIView) -> [UIScrollView] {
      (view as? UIScrollView).map { [$0] } ?? view.subviews.flatMap(scrollers)
    }
    let scrolls = scrollers(host.view)
    #expect(scrolls.count == 1)
    let scroll = try #require(scrolls.first)
    #expect(scroll.bounds.height > 0 && scroll.bounds.height < window.bounds.height)
    // Short diffs keep the same full-height reading surface instead of shrinking into a card.
    #expect(scroll.bounds.height > window.bounds.height / 2)
    if !short {
      #expect(scroll.contentSize.height > scroll.bounds.height * 10)
      // The tail is reachable in the same viewport, rather than trapped under an outer scroller.
      scroll.setContentOffset(CGPoint(x: 0, y: scroll.contentSize.height - scroll.bounds.height), animated: false)
      try await Task.sleep(for: .milliseconds(100))
      #expect(scroll.contentOffset.y > scroll.bounds.height)
    }
    #expect(scroll.contentSize.width > scroll.bounds.width * 2)
  }
  #endif
}
