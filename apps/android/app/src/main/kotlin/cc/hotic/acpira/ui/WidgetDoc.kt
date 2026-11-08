package cc.hotic.acpira.ui

import androidx.compose.ui.graphics.Color
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Swatch
import cc.hotic.acpira.design.Swatches
import cc.hotic.acpira.design.Type
import org.json.JSONArray
import kotlin.math.roundToInt

// The document an HTML widget (Acpira's `show_widget`) runs as on the phone (apps/ios Web/WidgetDoc.swift, widgetDoc.ts's
// head for a WebView of its own instead of a srcdoc frame). A WebView has no page above it, so ours allows inline
// scripts outright and still leaves the widget no network (fetch, images, fonts from anywhere but data:). Tailwind's
// browser build and the bootstrap ride inline in the head (Android's WebView has no user scripts); a theme switch reaches
// the page as a script (`theme`), so the widget keeps its state as the desktop's does. Pure string work.

object WidgetDoc {
  /** The name the bootstrap reports the page's height through (WidgetView's JavaScript interface) */
  const val BRIDGE = "acpiraWidget"

  /** The page's `dark` swatches when [dark], else the light ones, as CSS */
  private fun css(s: Swatch, dark: Boolean): String {
    val c: Color = if (dark) s.dark else s.light
    fun ch(v: Float) = (v * 255).roundToInt()
    return "rgba(${ch(c.red)}, ${ch(c.green)}, ${ch(c.blue)}, ${c.alpha})"
  }

  /** chipHover and focus (Tokens.swift): only widgets read them on the phone */
  private val chipHover = Swatch(Color(1f, 1f, 1f, 0.1f), Color(0xFFF1F1F1))
  private val focus = Swatch(Color(0xFF0078D4), Color(0xFF005FB8))

  /** widgetDoc.ts WIDGET_VARS: the variable names models already write (shadcn-style), over the phone's tokens */
  fun vars(dark: Boolean): List<Pair<String, String>> {
    val colors = listOf(
      "background" to Swatches.bg0, "foreground" to Swatches.fg1, "card" to Swatches.bg1, "card-foreground" to Swatches.fg1,
      "popover" to Swatches.bg1, "popover-foreground" to Swatches.fg1, "muted" to Swatches.chip, "muted-foreground" to Swatches.fg2,
      "secondary" to Swatches.chip, "secondary-foreground" to Swatches.fg1, "accent" to chipHover,
      "accent-foreground" to Swatches.fg1, "primary" to Swatches.btn1Bg, "primary-foreground" to Swatches.btn1Fg,
      "border" to Swatches.line, "input" to Swatches.lineStrong, "ring" to focus, "success" to Swatches.ok,
      "warning" to Swatches.warn, "danger" to Swatches.danger, "destructive" to Swatches.danger,
    )
    return colors.map { (name, s) -> name to css(s, dark) } + listOf(
      "font-sans" to "Inter, system-ui, sans-serif", "font-mono" to "ui-monospace, monospace",
      "font-size" to "${Type.body.value.roundToInt()}px", "radius" to "${Metrics.rMd.value.roundToInt()}px",
    )
  }

  /** widgetDoc.ts TAILWIND_THEME: colour utilities for each colour name, `dark:` following the app's theme */
  val tailwindTheme: String
    get() {
      val names = vars(false).map { it.first }.filter { !it.startsWith("font-") && it != "radius" }
      return "@custom-variant dark (&:where(.dark, .dark *));\n@theme inline {\n" +
        names.joinToString("\n") { "  --color-$it: var(--$it);" } + "\n}"
    }

  /** Runs first: reports the content height to the app, keeps links from navigating (the WebView refuses them too) */
  val bootstrap = """
  (function () {
    var last = 0;
    function report() {
      var h = Math.ceil(document.documentElement.getBoundingClientRect().height);
      if (h !== last) { last = h; window.$BRIDGE.height(h); }
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
  """.trimIndent()

  /** Swaps the page's theme in place: the `dark` class Tailwind's variant reads, the colour scheme and every variable */
  fun theme(dark: Boolean): String {
    val pairs = JSONArray(vars(dark).map { JSONArray(listOf(it.first, it.second)) }).toString()
    return """
    (function () {
      var r = document.documentElement;
      r.classList.toggle('dark', $dark);
      r.style.colorScheme = '${if (dark) "dark" else "light"}';
      $pairs.forEach(function (p) { r.style.setProperty('--' + p[0], p[1]); });
    })();
    """.trimIndent()
  }

  private val cspMeta = Regex("""<meta\b[^>]*\bhttp-equiv\s*=\s*["']?\s*content-security-policy\b[^>]*>""", RegexOption.IGNORE_CASE)
  private val baseTag = Regex("""<base\b[^>]*>""", RegexOption.IGNORE_CASE)

  /**
   * The widget's page behind our head; the widget's own CSP (it could only narrow ours) and `<base>` are dropped.
   * [tailwind]: the browser build's source, inlined after the theme it reads (null: utilities stay unstyled)
   */
  fun build(source: String, dark: Boolean, tailwind: String?): String {
    val body = source.replace(cspMeta, "").replace(baseTag, "")
    val csp = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; media-src data: blob:"
    val rootVars = vars(dark).joinToString(" ") { "--${it.first}: ${it.second};" }
    val base = """
    :root { $rootVars color-scheme: ${if (dark) "dark" else "light"}; }
    html, body { margin: 0; background: transparent; }
    body { color: var(--foreground); font-family: var(--font-sans); font-size: var(--font-size); line-height: 1.5; overflow-wrap: anywhere; -webkit-text-size-adjust: 100%; }
    """.trimIndent()
    // `</script` inside the inlined runtime would end its tag early
    val runtime = tailwind?.replace("</script", "<\\/script")?.let { "<script>$it</script>" }.orEmpty()
    return """
    <!doctype html><html class="${if (dark) "dark" else ""}"><head>
    <meta charset="utf-8">
    <meta http-equiv="Content-Security-Policy" content="$csp">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <script>$bootstrap</script>
    <style>$base</style>
    <style type="text/tailwindcss">$tailwindTheme</style>
    $runtime
    </head>
    """.trimIndent() + "\n" + body
  }
}
