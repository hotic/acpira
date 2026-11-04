#if os(iOS)
import AcpiraDesign
import AcpiraModels
import SwiftUI
import Testing
import UIKit
@testable import AcpiraUI

@MainActor @Suite(.serialized) struct FoldAnimationTests {
  /// Media stays at full size and opacity while only its surrounding rows fold, including an automatic close
  /// without a button transaction. A height-only assertion misses two clipped copies cross-fading to white.
  @Test(arguments: [false, true])
  func imageStaysWholeDuringFold(automatic: Bool) async throws {
    let block = try JSONDecoder().decode(AgentBlock.self, from: Data(#"""
      {"type":"tool_call","id":"shown","kind":"other","verb":"Show image","verbKey":"verb.showImage",
       "status":"completed","content":{"type":"image","blob":"81b549eb128f4aaa.png","mimeType":"image/png"}}
      """#.utf8))
    _ = try await MediaLoader.demo.image(.blob(session: "fold-test", name: "81b549eb128f4aaa.png"))
    let probe = FoldHeight()
    let host = UIHostingController(rootView: Sample(block: block, probe: probe))
    host.safeAreaRegions = []
    let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 402, height: 874))
    window.rootViewController = host
    // Measuring a fold does not need keyboard focus; leave other hosted tests' first responders alone.
    window.isHidden = false
    defer { window.isHidden = true; window.rootViewController = nil }
    host.view.layoutIfNeeded()
    try await Task.sleep(for: .milliseconds(300))
    let opened = probe.height
    #expect(opened > Metrics.agentImage)
    let pixels = coloredPixels(in: host.view)
    try #require(pixels > 1000)

    if automatic {
      probe.open = false
    } else {
      withAnimation(Motion.close) { probe.open = false }
    }
    var samples = [opened]
    var visiblePixels = [pixels]
    for _ in 0..<40 {
      try await Task.sleep(for: .milliseconds(15))
      samples.append(probe.height)
      visiblePixels.append(coloredPixels(in: host.view))
    }
    let closed = probe.height
    #expect(closed > Metrics.agentImage)
    #expect(opened > closed)
    #expect(samples.allSatisfy { $0 >= closed - 1 && $0 <= opened + 1 },
            "Closing must preserve the image's layout space: \(samples)")

    withAnimation(Motion.open) { probe.open = true }
    samples = [closed]
    for _ in 0..<40 {
      try await Task.sleep(for: .milliseconds(15))
      samples.append(probe.height)
      visiblePixels.append(coloredPixels(in: host.view))
    }
    #expect(abs(probe.height - opened) <= 1)
    #expect(samples.allSatisfy { $0 >= closed - 1 && $0 <= opened + 1 },
            "Opening must not change the result's layout space: \(samples)")
    #expect(visiblePixels.allSatisfy { abs($0 - pixels) <= pixels / 20 },
            "Media must not fade, clip or duplicate while rows fold: \(visiblePixels)")
  }

  /// Count the fixture's saturated magenta, independent of its position. A white wipe loses these pixels even if
  /// the total fold height remains correct. Read the live layer tree, not an ImageRenderer at the final state.
  private func coloredPixels(in view: UIView) -> Int {
    let format = UIGraphicsImageRendererFormat()
    format.scale = 1
    let image = UIGraphicsImageRenderer(bounds: view.bounds, format: format).image { context in
      view.layer.render(in: context.cgContext)
    }
    guard let image = image.cgImage else { return 0 }
    let width = image.width, height = image.height
    var bytes = [UInt8](repeating: 0, count: width * height * 4)
    return bytes.withUnsafeMutableBytes { buffer in
      guard let context = CGContext(data: buffer.baseAddress, width: width, height: height, bitsPerComponent: 8,
                                    bytesPerRow: width * 4, space: CGColorSpaceCreateDeviceRGB(),
                                    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return 0 }
      context.draw(image, in: CGRect(x: 0, y: 0, width: width, height: height))
      let pixels = buffer.bindMemory(to: UInt8.self)
      return stride(from: 0, to: pixels.count, by: 4).reduce(into: 0) { count, i in
        let red = Int(pixels[i]), green = Int(pixels[i + 1]), blue = Int(pixels[i + 2])
        if red > 150 && blue > 150 && red > green * 3 / 2 && blue > green * 3 / 2 { count += 1 }
      }
    }
  }

  @Observable final class FoldHeight {
    var open = true
    var height: CGFloat = 0
  }

  private struct Sample: View {
    let block: AgentBlock
    let probe: FoldHeight

    var body: some View {
      CodexFold(turn: AgentTurn(blocks: [block]), blocks: [block], running: false, hasTools: true)
        .environment(\.preOpen, probe.open ? [.fold] : [])
        .environment(\.hostedTranscript, true)
        .environment(\.media, MediaScope(session: "fold-test"))
        .environment(\.colorScheme, .light)
        .background(Color.white)
        .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { probe.height = $0 }
    }
  }
}
#endif
