import AcpiraFixtures
import AcpiraMarkdown
import AcpiraModels
import AcpiraSync
@testable import AcpiraUI
import CoreGraphics
import Foundation
import ImageIO
import Synchronization
import Testing

/// Counts what a fake store was asked for, across the loads the tests run concurrently
actor Calls {
  private(set) var list: [String] = []
  func add(_ call: String) { list.append(call) }
}

/// A 1×1 PNG (the fake agent's red dot)
let dotPNG = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg=="

// The media cache's bounds and sharing, the HTTP blob route, where an image's pixels come from, and the loader over a
// store the test plays
@MainActor
struct MediaCacheTests {
  func cache(budget: Int = 100, limit: Int = 10) -> MediaCache<String, Int> {
    MediaCache(budget: budget, limit: limit, cost: { $0 })
  }

  @Test func concurrentRequestsShareOneLoad() async throws {
    let cache = cache()
    let calls = Calls()
    let make: @Sendable () async throws -> Int = {
      await calls.add("load")
      try await Task.sleep(for: .milliseconds(20))
      return 7
    }
    async let a = cache.load("k", make)
    async let b = cache.load("k", make)
    #expect(try await [a, b] == [7, 7])
    #expect(await calls.list == ["load"])
    // Kept: a later request does not load again
    #expect(try await cache.load("k", make) == 7)
    #expect(await calls.list == ["load"])
  }

  @Test func aFailedLoadIsNotKept() async throws {
    let cache = cache()
    struct Boom: Error {}
    await #expect(throws: Boom.self) { try await cache.load("k") { throw Boom() } }
    #expect(cache.cached("k") == nil)
    #expect(try await cache.load("k") { 3 } == 3)
  }

  @Test func theLeastRecentlyUsedGoesFirst() {
    let cache = cache(limit: 2)
    cache.keep("a", 1)
    cache.keep("b", 1)
    _ = cache.cached("a")
    cache.keep("c", 1)
    #expect(cache.cached("b") == nil)
    #expect(cache.cached("a") == 1 && cache.cached("c") == 1)
    #expect(cache.count == 2)
  }

  @Test func theBudgetBoundsTheTotalCost() {
    let cache = cache(budget: 10)
    cache.keep("a", 4)
    cache.keep("b", 4)
    cache.keep("c", 4)
    #expect(cache.cached("a") == nil)
    #expect(cache.total == 8)
    // More than the whole budget: handed out, never kept
    cache.keep("huge", 11)
    #expect(cache.cached("huge") == nil)
    #expect(cache.total == 8)
  }

  @Test func blobURL() {
    #expect(HTTPBlobs.url(base: "http://127.0.0.1:7400/blobs", session: "s-1", name: "ab12.png")?.absoluteString
            == "http://127.0.0.1:7400/blobs/s-1/ab12.png")
    #expect(HTTPBlobs.url(base: "not a url", session: "s", name: "n.png") == nil)
  }
}

@MainActor
struct MediaSourceTests {
  @Test func storedNamesAndInlinePixels() {
    #expect(MediaSource.stored("ab12cd34.png", session: "s") == .blob(session: "s", name: "ab12cd34.png"))
    #expect(MediaSource.stored(dotPNG, session: "s") == .inline(dotPNG))
    // A blob name needs the session it belongs to; inline pixels do not
    #expect(MediaSource.stored("ab12cd34.png", session: nil) == nil)
    #expect(MediaSource.stored(dotPNG, session: nil) == .inline(dotPNG))
    #expect(MediaSource.stored(nil, session: "s") == nil)
  }

  @Test func agentImages() {
    #expect(MediaSource.agentImage(blob: nil, uri: "data:image/png;base64,\(dotPNG)", session: "s") == .inline(dotPNG))
    #expect(MediaSource.agentImage(blob: nil, uri: "/repo/red.png", session: "s") == nil)
    #expect(MediaSource.agentImage(blob: "a.png", uri: "/repo/red.png", session: "s") == .blob(session: "s", name: "a.png"))
    #expect(fileOfURI("/repo/red.png") == "/repo/red.png")
    #expect(fileOfURI("file:///repo/my%20shot.png") == "/repo/my shot.png")
    #expect(fileOfURI("https://example.com/a.png") == nil)
    #expect(fileOfURI("C:\\shots\\a.png") == "C:\\shots\\a.png")
  }

  @Test func aWidgetsHTMLIsItsPageNotAnImage() throws {
    let json = #"{"id":"w","kind":"other","verb":"Widget","verbKey":"verb.showWidget","status":"completed","contents":["#
      + #"{"type":"image","blob":"a.html","mimeType":"text/html"},{"type":"image","blob":"b.png","mimeType":"image/png"}]}"#
    let block = try JSONDecoder().decode(ToolCallBlock.self, from: Data(json.utf8))
    #expect(toolImages(block).map(\.blob) == ["b.png"])
    #expect(toolWidget(block)?.blob == "a.html")
    // A result for the reader, as imageTools.ts isImageResultBlock: under its row, after a closed fold
    #expect(isImageResult(block))
  }

  @Test func markdownImages() throws {
    let images = try JSONDecoder().decode([ImageRef].self, from: Data(#"""
    [{"uri":"docs/my%20arch.png","blob":"b1.png","mimeType":"image/png"},{"uri":"./gone.png","mimeType":"image/png"}]
    """#.utf8))
    #expect(MediaSource.markdown("https://example.com/a.png", images: nil, session: "s") == .web(URL(string: "https://example.com/a.png")!))
    #expect(MediaSource.markdown("data:image/png;base64,\(dotPNG)", images: nil, session: "s") == .inline(dotPNG))
    // Read by the engine with the text: matched on the source as written, percent-decoded on both sides
    #expect(MediaSource.markdown("docs/my arch.png", images: images, session: "s") == .blob(session: "s", name: "b1.png"))
    // The engine could not read it: nothing to stage again
    #expect(MediaSource.markdown("./gone.png", images: images, session: "s") == nil)
    // Never seen: staged on demand, the path decoded
    #expect(MediaSource.markdown("/tmp/new%20one.png", images: images, session: "s") == .file(session: "s", path: "/tmp/new one.png"))
    #expect(MediaSource.markdown("blob:abc", images: nil, session: "s") == nil)
  }

  @Test func paragraphsSplitAtTheirImages() {
    guard case .paragraph(let inlines) = Markdown.parse("Before:\n![a](x.png)\nafter **this**")[0] else {
      Issue.record("not a paragraph"); return
    }
    #expect(paragraphParts(inlines) == [.text([.text("Before:")]), .image(src: "x.png", alt: "a"),
                                        .text([.text("after "), .strong([.text("this")])])])
    #expect(holdsImage(inlines))
    guard case .paragraph(let badge) = Markdown.parse("[![CI](b.svg)](https://ci)")[0] else { return }
    #expect(!holdsImage(badge))
    guard case .paragraph(let two) = Markdown.parse("![a](1.png) ![b](2.png)")[0] else { return }
    #expect(paragraphParts(two) == [.image(src: "1.png", alt: "a"), .image(src: "2.png", alt: "b")])
  }

  @Test func theMediaFixtureHoldsEveryKind() throws {
    let view = try #require(Fixtures.session("media"))
    let sources = MediaLoader.sources(in: view)
    #expect(sources.contains { if case .inline = $0 { true } else { false } })
    #expect(sources.filter { if case .blob = $0 { true } else { false } }.count == 6)
    // Every stored blob ships with the fixtures
    for case .blob(_, let name) in sources { #expect(Fixtures.blob(name) != nil, "\(name)") }
  }
}

struct MarkdownImageTests {
  @Test func images() {
    #expect(Markdown.parse("![alt *text*](a.png \"title\")") == [.paragraph([.image(url: "a.png", alt: "alt text")])])
    #expect(Markdown.parse("![shot](<my shot.png>)") == [.paragraph([.image(url: "my shot.png", alt: "shot")])])
    #expect(Markdown.parse("[![CI](b.svg)](https://ci)") == [.paragraph([.link(url: "https://ci", [.image(url: "b.svg", alt: "CI")])])])
    // A bang with no link after it stays text
    #expect(Markdown.parse("Wow! [x](y)") == [.paragraph([.text("Wow! "), .link(url: "y", [.text("x")])])])
    #expect(Markdown.parse("![half](") == [.paragraph([.text("![half](")])])
    #expect(Markdown.plain([.image(url: "a.png", alt: "Chart")]) == "Chart")
  }
}

@MainActor
struct MediaLoaderTests {
  @Test func blobsFetchOnceAndFilesAreStagedFirst() async throws {
    let calls = Calls()
    let loader = MediaLoader(fetch: { session, name in
      await calls.add("fetch \(session)/\(name)")
      try await Task.sleep(for: .milliseconds(10))
      return Data(base64Encoded: dotPNG)!
    }, stage: { session, path in
      await calls.add("stage \(session) \(path)")
      return path.hasSuffix("missing.png") ? nil : "staged.png"
    })
    let source = MediaSource.blob(session: "s", name: "a.png")
    async let first = loader.image(source)
    async let second = loader.image(source)
    let (a, b) = try await (first, second)
    #expect(a == b && a.width == 1 && a.height == 1)
    #expect(loader.peek(source) == a)
    _ = try await loader.image(.file(session: "s", path: "/w/shot.png"))
    await #expect(throws: (any Error).self) { try await loader.image(.file(session: "s", path: "/w/missing.png")) }
    #expect(await calls.list == ["fetch s/a.png", "stage s /w/shot.png", "fetch s/staged.png", "stage s /w/missing.png"])
  }

  @Test func inlinePixelsDecodeInPlace() {
    let image = MediaLoader.demo.peek(.inline(dotPNG))
    #expect(image?.width == 1)
    #expect(MediaLoader.demo.peek(.inline("not base64 at all")) == nil)
  }

  /// An animated GIF plays: every frame decoded, on an even beat where a longer frame repeats, and a zero delay plays
  /// at the browsers' 100 ms
  @Test func animatedImagesKeepTheirFrames() throws {
    let gif = try Self.gif(delays: [0.1, 0.2, 0])
    let image = try #require(MediaLoader.decodeNow(gif))
    #expect(image.animated)
    #expect(abs(image.frameDuration - 0.1) < 0.001)
    // 0.1 once, 0.2 twice, the zero delay as 0.1 once
    #expect(image.frames.count == 4)
    #expect(image.frames[1] === image.frames[2])
    #expect(image.frames[0] !== image.frames[1])
    #expect(image.cgImage === image.frames[0])
    #expect(image.frame(at: Date(timeIntervalSinceReferenceDate: 0.15)) === image.frames[1])
    // A single frame stays a still picture
    let still = try #require(MediaLoader.decodeNow(try Self.gif(delays: [0.1])))
    #expect(!still.animated)
  }

  /// A GIF of solid 4×4 frames, one colour each, with these delays
  static func gif(delays: [Double]) throws -> Data {
    let data = NSMutableData()
    let out = try #require(CGImageDestinationCreateWithData(data, "com.compuserve.gif" as CFString, delays.count, nil))
    for (i, delay) in delays.enumerated() {
      let context = try #require(CGContext(data: nil, width: 4, height: 4, bitsPerComponent: 8, bytesPerRow: 16,
                                           space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
      context.setFillColor(CGColor(red: Double(i % 2), green: Double(i / 2 % 2), blue: 0.5, alpha: 1))
      context.fill(CGRect(x: 0, y: 0, width: 4, height: 4))
      let frame = try #require(context.makeImage())
      let props = [kCGImagePropertyGIFDictionary: [kCGImagePropertyGIFDelayTime: delay, kCGImagePropertyGIFUnclampedDelayTime: delay]]
      CGImageDestinationAddImage(out, frame, props as CFDictionary)
    }
    #expect(CGImageDestinationFinalize(out))
    return data as Data
  }

  @Test func theDemoReadsFixtureBlobs() async throws {
    let view = try #require(Fixtures.session("media"))
    await MediaLoader.demo.preload(view)
    for source in MediaLoader.sources(in: view) { #expect(MediaLoader.demo.peek(source) != nil, "\(source)") }
  }
}

struct LightboxGeometryTests {
  @Test func fitting() {
    #expect(LightboxGeometry.fit(CGSize(width: 800, height: 400), in: CGSize(width: 400, height: 800)) == CGSize(width: 400, height: 200))
    #expect(LightboxGeometry.fit(CGSize(width: 10, height: 20), in: CGSize(width: 400, height: 800)) == CGSize(width: 400, height: 800))
    #expect(LightboxGeometry.centering(CGSize(width: 400, height: 200), in: CGSize(width: 400, height: 800)) == CGSize(width: 0, height: 300))
    #expect(LightboxGeometry.centering(CGSize(width: 900, height: 900), in: CGSize(width: 400, height: 800)) == .zero)
  }

  @Test func zooming() {
    // At least 4×; further while a screen pixel still meets an image pixel
    #expect(LightboxGeometry.maxZoom(pixels: CGSize(width: 800, height: 400), fitted: CGSize(width: 400, height: 200), screenScale: 3) == 4)
    #expect(LightboxGeometry.maxZoom(pixels: CGSize(width: 6000, height: 3000), fitted: CGSize(width: 400, height: 200), screenScale: 3) == 5)
    #expect(LightboxGeometry.zoomRect(around: CGPoint(x: 100, y: 100), scale: 2, in: CGSize(width: 400, height: 800))
            == CGRect(x: 0, y: -100, width: 200, height: 400))
  }

  @Test func pulling() {
    #expect(LightboxGeometry.pull(-50, height: 800) == 0)
    #expect(LightboxGeometry.pull(200, height: 800) == 0.5)
    #expect(LightboxGeometry.pull(900, height: 800) == 1)
    #expect(LightboxGeometry.closes(dy: 130, velocity: 0, height: 800))
    #expect(!LightboxGeometry.closes(dy: 60, velocity: 100, height: 800))
    // A quick flick closes from a short pull, but not one going up
    #expect(LightboxGeometry.closes(dy: 30, velocity: 1200, height: 800))
    #expect(!LightboxGeometry.closes(dy: -30, velocity: 1200, height: 800))
  }
}

/// The store's side: imageBlob requests answered by seq, the blob base InitState names
@MainActor
struct EngineMediaTests {
  @Test func imageBlobRequestsAreAnswered() async throws {
    let engine = EngineModelTests()
    let (model, wire) = try await engine.connected()
    let link = try #require(model.link)
    let staged = Task { await link.stage("s1", "/w/a.png") }
    try await engine.until { wire.types.last == "webviewMessage:imageBlob" }
    #expect(wire.sent.last?.contains("a.png") == true)
    engine.push(wire, #"{"type":"imageBlob","seq":1,"blob":"ab12.png"}"#)
    #expect(await staged.value == "ab12.png")
  }

  /// A paired wire brings the session's blobs over its own link: the computer's HTTP route is loopback only
  @Test func aPairedWireFetchesBlobsOverItsLink() async throws {
    let wire = BlobWire()
    let link = EngineLink(transport: wire, language: "en", blobBase: "http://127.0.0.1:1/blobs")
    let image = try await link.media.image(.blob(session: "s1", name: "dot.png"))
    #expect(image.width == 1)
    #expect(wire.asked.withLock { $0 } == ["s1/dot.png"])
  }

  @Test func initNamesTheBlobBase() async throws {
    let engine = EngineModelTests()
    let (model, wire) = try await engine.connected()
    #expect(model.link?.blobBase == nil)
    engine.push(wire, #"{"type":"init","state":{"host":"sidebar","agents":[],"accounts":[],"hidden":{},"sessions":[],"settings":{},"locale":"en","home":"/h","cwd":"/w","blobBase":"http://127.0.0.1:7400/blobs"}}"#)
    try await engine.until { model.link?.blobBase != nil }
    #expect(model.link?.blobBase == "http://127.0.0.1:7400/blobs")
  }
}

/// A paired wire that answers blobs only: the 1×1 dot for any name
final class BlobWire: EngineTransport, PairedBlobs {
  let asked = Mutex<[String]>([])
  func frames() -> AsyncThrowingStream<String, Error> { AsyncThrowingStream { _ in } }
  func send(_ text: String) async throws {}
  func close() {}
  func blob(session: String, name: String) async throws -> Data {
    asked.withLock { $0.append("\(session)/\(name)") }
    return Data(base64Encoded: dotPNG)!
  }
}
