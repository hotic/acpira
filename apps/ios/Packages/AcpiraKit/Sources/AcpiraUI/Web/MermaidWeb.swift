import AcpiraDesign
import os
import SwiftUI
#if canImport(UIKit)
import UIKit
import WebKit
#endif

// The mermaid diagrams the native flowchart port does not read (sequence, pie, state, class, gantt …): mermaid's own
// browser build (Resources/web, the version the webview renders with, scripts/export/ios-web.ts) renders them in one
// shared, unparented WKWebView, and the SVG comes back as a PNG drawn through a canvas, so the transcript holds a plain
// image instead of a live web view per diagram. Labels are SVG text (`htmlLabels: false`): a foreignObject label would
// taint the canvas. A source mermaid cannot parse stays a code block, as a failed render does in the webview.

/// The bundled browser builds
enum WebRuntime {
  static func script(_ name: String) -> String? {
    guard let url = Bundle.module.url(forResource: name, withExtension: "js", subdirectory: "web") else { return nil }
    return try? String(contentsOf: url, encoding: .utf8)
  }
}

/// A rendered diagram: its pixels and the SVG's own size in points
struct MermaidPicture {
  #if canImport(UIKit)
  var image: UIImage
  #endif
  var size: CGSize
}

#if canImport(UIKit)
/// The one web view every diagram renders in, one at a time; pictures are cached by source, theme and scale
@MainActor
final class MermaidRenderer: NSObject, WKNavigationDelegate {
  static let shared = MermaidRenderer()
  static let log = Logger(subsystem: "cc.hotic.acpira", category: "web")

  private var web: WKWebView?
  private var loaded: CheckedContinuation<Void, Never>?
  private var loading: Task<Void, Never>?
  private var last: Task<MermaidPicture, Error>?
  private var cache: [String: MermaidPicture] = [:]
  private var serial = 0

  /// streamdown's mermaid defaults (the webview's config) plus SVG labels
  private static let page = """
  <!doctype html><html><head><meta charset="utf-8"></head><body><script>
  async function acpiraRender(id, source, dark, scale) {
    mermaid.initialize({ startOnLoad: false, securityLevel: 'strict', fontFamily: 'monospace', suppressErrorRendering: true,
      theme: dark ? 'dark' : 'default', htmlLabels: false, flowchart: { htmlLabels: false } });
    const { svg } = await mermaid.render(id, source);
    const root = new DOMParser().parseFromString(svg, 'image/svg+xml').documentElement;
    const box = root.viewBox.baseVal;
    const w = box && box.width ? box.width : parseFloat(root.getAttribute('width')) || 300;
    const h = box && box.height ? box.height : parseFloat(root.getAttribute('height')) || 150;
    root.setAttribute('width', w); root.setAttribute('height', h); root.removeAttribute('style');
    const img = new Image();
    await new Promise((ok, fail) => { img.onload = ok; img.onerror = () => fail(new Error('svg image')); img.src =
      'data:image/svg+xml;charset=utf-8,' + encodeURIComponent(new XMLSerializer().serializeToString(root)); });
    const canvas = document.createElement('canvas');
    canvas.width = Math.ceil(w * scale); canvas.height = Math.ceil(h * scale);
    const ctx = canvas.getContext('2d');
    ctx.scale(scale, scale);
    ctx.drawImage(img, 0, 0, w, h);
    return { png: canvas.toDataURL('image/png').slice(22), width: w, height: h };
  }
  </script></body></html>
  """

  func picture(_ source: String, dark: Bool, scale: CGFloat) async throws -> MermaidPicture {
    let key = "\(dark ? 1 : 0)\u{1}\(scale)\u{1}\(source)"
    if let hit = cache[key] { return hit }
    // One render at a time: mermaid's render keeps state on the page between its awaits
    let previous = last
    let task = Task { @MainActor [weak self] () throws -> MermaidPicture in
      _ = try? await previous?.value
      guard let self else { throw CancellationError() }
      return try await self.render(source, dark: dark, scale: scale)
    }
    last = task
    let made = try await task.value
    if cache.count > 64 { cache.removeAll() }
    cache[key] = made
    return made
  }

  private func render(_ source: String, dark: Bool, scale: CGFloat) async throws -> MermaidPicture {
    let web = try await ready()
    serial += 1
    let result = try await web.callAsyncJavaScript(
      "return await acpiraRender(id, source, dark, scale)",
      arguments: ["id": "acpira-mermaid-\(serial)", "source": source, "dark": dark, "scale": Double(scale)],
      in: nil, contentWorld: .page)
    guard let dict = result as? [String: Any], let png = dict["png"] as? String,
          let data = Data(base64Encoded: png), let image = UIImage(data: data, scale: scale),
          let w = dict["width"] as? Double, let h = dict["height"] as? Double else { throw CocoaError(.coderReadCorrupt) }
    return MermaidPicture(image: image, size: CGSize(width: w, height: h))
  }

  /// The page with mermaid loaded, made on the first diagram
  private func ready() async throws -> WKWebView {
    if let web, loading == nil { return web }
    if loading == nil {
      guard let script = WebRuntime.script("mermaid.min") else { throw CocoaError(.fileNoSuchFile) }
      let config = WKWebViewConfiguration()
      config.websiteDataStore = .nonPersistent()
      config.userContentController.addUserScript(WKUserScript(source: script, injectionTime: .atDocumentStart, forMainFrameOnly: true))
      let web = WKWebView(frame: CGRect(x: 0, y: 0, width: 800, height: 600), configuration: config)
      web.navigationDelegate = self
      self.web = web
      loading = Task { @MainActor in
        await withCheckedContinuation { self.loaded = $0; web.loadHTMLString(Self.page, baseURL: nil) }
      }
    }
    await loading?.value
    loading = nil
    guard let web else { throw CancellationError() }
    return web
  }

  nonisolated func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
    MainActor.assumeIsolated { finishLoad() }
  }

  nonisolated func webView(_ webView: WKWebView, didFail navigation: WKNavigation!, withError error: Error) {
    MainActor.assumeIsolated { finishLoad() }
  }

  private func finishLoad() {
    loaded?.resume()
    loaded = nil
  }
}
#endif

/// A mermaid block drawn by mermaid itself: the source while the picture is made (and if mermaid cannot read it),
/// then the picture, scaled down to the column when wider
struct MermaidWebBlock: View {
  let source: String
  @Environment(\.colorScheme) private var scheme
  @Environment(\.displayScale) private var displayScale
  @State private var picture: MermaidPicture?

  var body: some View {
    Group {
      #if canImport(UIKit)
      if let picture {
        Image(uiImage: picture.image).resizable()
          .aspectRatio(picture.size.width / max(picture.size.height, 1), contentMode: .fit)
          .frame(maxWidth: picture.size.width)
          .frame(maxWidth: .infinity)
          .accessibilityHidden(true)
      } else {
        CodeSurface(text: source, lang: "mermaid")
      }
      #else
      CodeSurface(text: source, lang: "mermaid")
      #endif
    }
    .task(id: "\(scheme == .dark)\u{1}\(source)") {
      #if canImport(UIKit)
      // MermaidBlock.tsx waits before its first try, so a fence still streaming is not rendered at every chunk
      try? await Task.sleep(for: .milliseconds(200))
      guard !Task.isCancelled else { return }
      // A source mermaid cannot read keeps what was there: the code, or the picture of the fence before its last chunk
      do {
        picture = try await MermaidRenderer.shared.picture(source, dark: scheme == .dark, scale: max(displayScale, 2))
      } catch {
        MermaidRenderer.log.notice("mermaid: \(String(describing: error), privacy: .public)")
      }
      #endif
    }
  }
}
