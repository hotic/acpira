import AcpiraDesign
import SwiftUI
#if canImport(UIKit)
import UIKit
import WebKit
#endif

// An HTML widget an agent showed with `show_widget` (AgentWidget.tsx): the page runs in a web view of its own, with no
// network, no storage that outlives it and no way to navigate, sized to the height the page reports. The page loads
// once the row nears the screen; a theme switch is applied by script, so the widget keeps its state.

/// The widget of a completed show_widget row: its HTML read through `load` (the session's blob store)
struct WidgetBlock: View {
  /// The blob's HTML; the caller knows where blobs come from (a development engine's HTTP, a paired computer's wire)
  let load: () async throws -> String
  @Environment(\.colorScheme) private var scheme
  /// The page as built, in the scheme it first loaded in
  @State private var page: String?
  @State private var failed = false
  /// The page's own height; the room held until it reports one, so the row does not open from nothing
  @State private var height: CGFloat = Metrics.widgetPlaceholder

  var body: some View {
    Group {
      #if canImport(UIKit)
      if let page {
        // Capped like the desktop's frame: a taller page scrolls inside itself
        WidgetWebView(html: page, dark: scheme == .dark, height: $height, scrolls: height > Metrics.widgetMax)
          .frame(height: min(height, Metrics.widgetMax))
      } else if failed {
        Text(web: t("widget.failed"), paragraph: false).textStyle(TypeScale.text2).foregroundStyle(Palette.fg3)
          .frame(maxWidth: .infinity, alignment: .leading)
      } else {
        RoundedRectangle(cornerRadius: Metrics.rMd).fill(Palette.chip).frame(height: height)
      }
      #else
      EmptyView()
      #endif
    }
    .task {
      guard page == nil else { return }
      // The blob is written while the row completes, so the first read can miss it (AgentWidget.tsx fetchText)
      for attempt in 0..<5 {
        if let text = try? await load() {
          page = WidgetDoc.build(text, dark: scheme == .dark)
          return
        }
        try? await Task.sleep(for: .milliseconds(250 * (attempt + 1)))
        if Task.isCancelled { return }
      }
      failed = true
    }
  }
}

#if canImport(UIKit)
/// The web view: Tailwind and the bootstrap as user scripts, the height through a message handler, every navigation but
/// the first load refused
struct WidgetWebView: UIViewRepresentable {
  let html: String
  var dark: Bool
  @Binding var height: CGFloat
  var scrolls = false

  func makeCoordinator() -> Coordinator { Coordinator(height: $height) }

  func makeUIView(context: Context) -> WKWebView {
    let config = WKWebViewConfiguration()
    config.websiteDataStore = .nonPersistent()
    config.allowsInlineMediaPlayback = true
    let scripts = config.userContentController
    // At the document's end: the build appends its stylesheet to `document.head`, which does not exist at its start
    if let tailwind = WebRuntime.script("tailwind") {
      scripts.addUserScript(WKUserScript(source: tailwind, injectionTime: .atDocumentEnd, forMainFrameOnly: true))
    }
    scripts.addUserScript(WKUserScript(source: WidgetDoc.bootstrap, injectionTime: .atDocumentStart, forMainFrameOnly: true))
    scripts.add(context.coordinator, name: "acpiraWidget")
    let web = WKWebView(frame: .zero, configuration: config)
    web.navigationDelegate = context.coordinator
    web.isOpaque = false
    web.backgroundColor = .clear
    web.scrollView.backgroundColor = .clear
    // The transcript scrolls; the widget is as tall as its page
    web.scrollView.isScrollEnabled = false
    web.scrollView.contentInsetAdjustmentBehavior = .never
    #if DEBUG
    web.isInspectable = true
    #endif
    context.coordinator.loaded = html
    context.coordinator.dark = dark
    web.loadHTMLString(html, baseURL: nil)
    return web
  }

  func updateUIView(_ web: WKWebView, context: Context) {
    web.scrollView.isScrollEnabled = scrolls
    if context.coordinator.dark != dark {
      context.coordinator.dark = dark
      web.evaluateJavaScript(WidgetDoc.theme(dark: dark))
    }
    guard context.coordinator.loaded != html else { return }
    context.coordinator.loaded = html
    context.coordinator.started = false
    web.loadHTMLString(html, baseURL: nil)
  }

  static func dismantleUIView(_ web: WKWebView, coordinator: Coordinator) {
    web.configuration.userContentController.removeScriptMessageHandler(forName: "acpiraWidget")
  }

  final class Coordinator: NSObject, WKNavigationDelegate, WKScriptMessageHandler {
    var height: Binding<CGFloat>
    var loaded: String?
    /// The theme the page should show; applied again once it loads, in case the switch came first
    var dark = false
    /// The document's own load went through; anything after it is the page trying to leave
    var started = false

    init(height: Binding<CGFloat>) { self.height = height }

    func userContentController(_ controller: WKUserContentController, didReceive message: WKScriptMessage) {
      guard let h = message.body as? Double, h > 0 else { return }
      height.wrappedValue = CGFloat(h)
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
      webView.evaluateJavaScript(WidgetDoc.theme(dark: dark))
    }

    func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction) async -> WKNavigationActionPolicy {
      // The initial about:blank load of the string, then nothing: links, forms and scripts stay on the page
      if !started, action.navigationType == .other, action.request.url?.absoluteString == "about:blank" {
        started = true
        return .allow
      }
      // An in-page anchor moves within the page
      if action.navigationType == .linkActivated, action.request.url?.fragment != nil,
         action.request.url?.absoluteString.hasPrefix("about:blank#") == true { return .allow }
      return .cancel
    }
  }
}
#endif
