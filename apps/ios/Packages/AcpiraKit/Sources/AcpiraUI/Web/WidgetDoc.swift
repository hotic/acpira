import AcpiraDesign
import Foundation

// The document an HTML widget (Acpira's `show_widget`) runs as on the phone: widgetDoc.ts's head, for a WKWebView of its
// own instead of a srcdoc frame. The frame there inherits the page's CSP and so needs nonces and a handler rebinding
// bootstrap; a web view has no page above it, so ours allows inline scripts outright and still leaves the widget no
// network (fetch, images, fonts from anywhere but data:). Tailwind's browser build and the bootstrap arrive as user
// scripts (WidgetView), which no page CSP blocks; a theme switch reaches the page as a script too (`theme`), so the
// widget keeps its state as the desktop's does. Pure string work, tested on macOS.

enum WidgetDoc {
  /// widgetDoc.ts WIDGET_VARS: the variable names models already write (shadcn-style), over the phone's tokens
  static func vars(dark: Bool) -> [(String, String)] {
    func css(_ s: Swatch) -> String {
      let c = dark ? s.dark : s.light
      return "rgba(\(Int((c.r * 255).rounded())), \(Int((c.g * 255).rounded())), \(Int((c.b * 255).rounded())), \(c.a))"
    }
    let colors: [(String, Swatch)] = [
      ("background", Swatches.bg0), ("foreground", Swatches.fg1), ("card", Swatches.bg1), ("card-foreground", Swatches.fg1),
      ("popover", Swatches.bg1), ("popover-foreground", Swatches.fg1), ("muted", Swatches.chip), ("muted-foreground", Swatches.fg2),
      ("secondary", Swatches.chip), ("secondary-foreground", Swatches.fg1), ("accent", Swatches.chipHover),
      ("accent-foreground", Swatches.fg1), ("primary", Swatches.btn1Bg), ("primary-foreground", Swatches.btn1Fg),
      ("border", Swatches.line), ("input", Swatches.lineStrong), ("ring", Swatches.focus), ("success", Swatches.ok),
      ("warning", Swatches.warn), ("danger", Swatches.danger), ("destructive", Swatches.danger),
    ]
    return colors.map { ($0.0, css($0.1)) } + [
      ("font-sans", "-apple-system, system-ui, sans-serif"), ("font-mono", "ui-monospace, Menlo, monospace"),
      ("font-size", "\(Int(TypeScale.body.size))px"), ("radius", "\(Int(Metrics.rMd))px"),
    ]
  }

  /// widgetDoc.ts TAILWIND_THEME: colour utilities for each colour name, `dark:` following the app's theme
  static var tailwindTheme: String {
    let names = vars(dark: false).map(\.0).filter { !$0.hasPrefix("font-") && $0 != "radius" }
    return "@custom-variant dark (&:where(.dark, .dark *));\n@theme inline {\n"
      + names.map { "  --color-\($0): var(--\($0));" }.joined(separator: "\n") + "\n}"
  }

  /// widgetDoc.ts WIDGET_PROMPT_MAX: the longest text `acpira.sendPrompt` accepts; longer throws in JS and drops here
  static let promptMax = 20_000
  /// AgentWidget.tsx PROMPT_GAP_MS: a double tap or a handler bound twice sends once
  static let promptGap = Duration.seconds(1)

  /// Runs first: exposes `window.acpira.sendPrompt(text)` (gated on user activation and a per-view token no page script
  /// can read), reports the content height to the app, and keeps links from navigating
  static func bootstrap(token: String = "") -> String {
    let secret = jsonString(token)
    return """
    (function () {
      var TOKEN = \(secret), post = window.webkit.messageHandlers.acpiraWidget.postMessage.bind(window.webkit.messageHandlers.acpiraWidget);
      window.acpira = Object.freeze({
        sendPrompt: function (text) {
          if (typeof text !== 'string' || !text.trim()) throw new TypeError('acpira.sendPrompt: text must be a non-empty string');
          if (text.length > \(promptMax)) throw new RangeError('acpira.sendPrompt: text is longer than \(promptMax) characters');
          if (navigator.userActivation && !navigator.userActivation.isActive) throw new Error('acpira.sendPrompt: call it from a click or key handler');
          post({ token: TOKEN, prompt: text });
        },
      });
      var last = 0;
      function report() {
        var h = Math.ceil(document.documentElement.getBoundingClientRect().height);
        if (h !== last) { last = h; post({ token: TOKEN, height: h }); }
      }
      addEventListener('click', function (e) {
        var a = e.target && e.target.closest && e.target.closest('a[href]');
        if (a && !/^#/.test(a.getAttribute('href') || '')) e.preventDefault();
      }, true);
      document.addEventListener('DOMContentLoaded', function () {
        var ro = new ResizeObserver(report);
        ro.observe(document.documentElement);
        if (document.body) ro.observe(document.body);
        report();
      });
      addEventListener('load', report);
    })();
    """
  }

  private static func jsonString(_ value: String) -> String {
    (try? JSONSerialization.data(withJSONObject: [value])).flatMap {
      let array = String(decoding: $0, as: UTF8.self)
      return array.count >= 2 ? String(array.dropFirst().dropLast()) : nil
    } ?? "\"\""
  }

  /// Swaps the page's theme in place: the `dark` class Tailwind's variant reads, the colour scheme and every variable
  static func theme(dark: Bool) -> String {
    let pairs = vars(dark: dark).map { [$0.0, $0.1] }
    let json = (try? JSONSerialization.data(withJSONObject: pairs)).map { String(decoding: $0, as: UTF8.self) } ?? "[]"
    return """
    (function () {
      var r = document.documentElement;
      r.classList.toggle('dark', \(dark));
      r.style.colorScheme = '\(dark ? "dark" : "light")';
      \(json).forEach(function (p) { r.style.setProperty('--' + p[0], p[1]); });
    })();
    """
  }

  private static let cspMeta = /<meta\b[^>]*\bhttp-equiv\s*=\s*["']?\s*content-security-policy\b[^>]*>/.ignoresCase()
  private static let baseTag = /<base\b[^>]*>/.ignoresCase()

  /// The widget's page behind our head; the widget's own CSP (it could only narrow ours) and `<base>` are dropped
  static func build(_ source: String, dark: Bool) -> String {
    let body = source.replacing(cspMeta, with: "").replacing(baseTag, with: "")
    let csp = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; media-src data: blob:"
    let rootVars = vars(dark: dark).map { "--\($0.0): \($0.1);" }.joined(separator: " ")
    let base = """
    :root { \(rootVars) color-scheme: \(dark ? "dark" : "light"); }
    html, body { margin: 0; background: transparent; -webkit-tap-highlight-color: transparent; }
    body { color: var(--foreground); font-family: var(--font-sans); font-size: var(--font-size); line-height: 1.5; overflow-wrap: anywhere; -webkit-text-size-adjust: 100%; }
    :where(button, [role="button"], label, summary, a) { touch-action: manipulation; }
    :where(button, [role="button"], summary):not(:disabled):active { opacity: 0.85; }
    """
    return """
    <!doctype html><html class="\(dark ? "dark" : "")"><head>
    <meta charset="utf-8">
    <meta http-equiv="Content-Security-Policy" content="\(csp)">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <style>\(base)</style>
    <style type="text/tailwindcss">\(tailwindTheme)</style>
    </head>
    \(body)
    """
  }
}
