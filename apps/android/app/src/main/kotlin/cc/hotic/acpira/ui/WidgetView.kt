package cc.hotic.acpira.ui

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Color as AndroidColor
import android.os.Handler
import android.os.Looper
import android.view.MotionEvent
import android.webkit.JavascriptInterface
import android.webkit.WebResourceRequest
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import cc.hotic.acpira.design.LocalDark
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.i18n.t
import java.util.UUID
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext

// An HTML widget an agent showed with `show_widget` (apps/ios Web/WidgetView.swift, AgentWidget.tsx): the page runs in a
// WebView of its own, with no network, no storage that outlives it and no way to navigate, sized to the height the page
// reports. A theme switch is applied by script, so the widget keeps its state.

/**
 * A show_widget widget's `acpira.sendPrompt` (WidgetPromptContext): sends as the next message in the main thread; null
 * in an inspector or a session that cannot take messages, where widgets stay one-way
 */
val LocalWidgetPrompt = compositionLocalOf<((String) -> Unit)?> { null }

/** The widget of a completed show_widget row: its HTML read through [load] (the session's blob store) */
@Composable
fun WidgetBlock(load: suspend () -> String) {
  val dark = LocalDark.current
  val onPrompt = LocalWidgetPrompt.current
  val context = androidx.compose.ui.platform.LocalContext.current
  val token = remember { UUID.randomUUID().toString() }
  /** The page as built, in the scheme it first loaded in */
  var page by remember { mutableStateOf<String?>(null) }
  var failed by remember { mutableStateOf(false) }
  /** The page's own height in CSS px (dp); the room held until it reports one, so the row does not open from nothing */
  var height by remember { mutableStateOf(Metrics.widgetPlaceholder.value) }
  LaunchedEffect(Unit) {
    if (page != null) return@LaunchedEffect
    // The blob is written while the row completes, so the first read can miss it (AgentWidget.tsx fetchText)
    for (attempt in 0 until LOAD_TRIES) {
      val text = runCatching { load() }.getOrNull()
      if (text != null) {
        val tailwind = withContext(Dispatchers.IO) { tailwindRuntime(context) }
        page = WidgetDoc.build(text, dark, tailwind, token)
        return@LaunchedEffect
      }
      delay(LOAD_BACKOFF_MS * (attempt + 1))
    }
    failed = true
  }
  val shown = page
  when {
    shown != null -> {
      // Capped like the desktop's frame: a taller page scrolls inside itself
      val capped = minOf(height, Metrics.widgetMax.value)
      WidgetWebView(
        shown, dark, token, scrolls = height > Metrics.widgetMax.value,
        onHeight = { height = it }, onPrompt = onPrompt,
        modifier = Modifier.fillMaxWidth().height(capped.dp).testTag("widget"),
      )
    }
    failed -> Text(t("widget.failed"), fontSize = Type.text2, color = Palette.fg3, modifier = Modifier.fillMaxWidth())
    else -> Box(Modifier.fillMaxWidth().height(height.dp).background(Palette.chip, RoundedCornerShape(Metrics.rMd)))
  }
}

/** Tailwind's browser build, synced from the iOS app's runtimes into assets/web by the build; read once */
@Volatile private var tailwindSource: String? = null

private fun tailwindRuntime(context: Context): String? = tailwindSource ?: runCatching {
  context.assets.open("web/tailwind.js").bufferedReader().use { it.readText() }
}.getOrNull()?.also { tailwindSource = it }

/**
 * The WebView: scripts on, the network blocked, nothing stored, every navigation but the first load and in-page anchors
 * refused, the height through a JavaScript interface
 */
@SuppressLint("SetJavaScriptEnabled", "ClickableViewAccessibility")
@Composable
private fun WidgetWebView(
  html: String,
  dark: Boolean,
  token: String,
  scrolls: Boolean,
  onHeight: (Float) -> Unit,
  onPrompt: ((String) -> Unit)?,
  modifier: Modifier,
) {
  val holder = remember { WidgetState() }
  holder.token = token
  holder.onHeight = onHeight
  holder.onPrompt = onPrompt
  holder.dark = dark
  AndroidView(
    factory = { ctx ->
      WebView(ctx).apply {
        setBackgroundColor(AndroidColor.TRANSPARENT)
        settings.javaScriptEnabled = true
        settings.blockNetworkLoads = true
        settings.allowFileAccess = false
        settings.allowContentAccess = false
        settings.domStorageEnabled = false
        settings.cacheMode = WebSettings.LOAD_NO_CACHE
        settings.textZoom = TEXT_ZOOM
        isVerticalScrollBarEnabled = false
        isHorizontalScrollBarEnabled = false
        addJavascriptInterface(holder.bridge, WidgetDoc.BRIDGE)
        webViewClient = object : WebViewClient() {
          // Links, forms and scripts stay on the page; an in-page anchor moves within it (WidgetView.swift)
          override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean =
            !request.url.toString().startsWith(IN_PAGE)
          override fun onPageFinished(view: WebView, url: String?) {
            // The theme the page should show, in case the switch came first
            view.evaluateJavascript(WidgetDoc.theme(holder.dark), null)
          }
        }
        // The transcript scrolls; a page within its cap is as tall as itself and hands drags to the thread
        setOnTouchListener { v, event ->
          if (holder.scrolls && event.action == MotionEvent.ACTION_DOWN) v.parent?.requestDisallowInterceptTouchEvent(true)
          false
        }
        holder.loaded = html
        holder.applied = dark
        loadDataWithBaseURL(null, html, "text/html", "utf-8", null)
      }
    },
    update = { web ->
      holder.scrolls = scrolls
      if (holder.applied != dark) {
        holder.applied = dark
        web.evaluateJavascript(WidgetDoc.theme(dark), null)
      }
      if (holder.loaded != html) {
        holder.loaded = html
        web.loadDataWithBaseURL(null, html, "text/html", "utf-8", null)
      }
    },
    onRelease = { web ->
      web.removeJavascriptInterface(WidgetDoc.BRIDGE)
      web.destroy()
    },
    modifier = modifier,
  )
}

/** What the WebView's callbacks read: the latest height and prompt handlers, the theme, whether the page scrolls itself */
private class WidgetState {
  @Volatile var token = ""
  var onHeight: (Float) -> Unit = {}
  var onPrompt: ((String) -> Unit)? = null
  @Volatile var dark = false
  var applied = false
  var loaded: String? = null
  var scrolls = false
  private val main = Handler(Looper.getMainLooper())
  private val gate = WidgetPromptGate()

  /** Called by the page's bootstrap on the WebView's own thread */
  val bridge = object {
    @JavascriptInterface
    fun height(t: String, h: Double) {
      if (t == token && h > 0) main.post { onHeight(h.toFloat()) }
    }

    @JavascriptInterface
    fun prompt(t: String, raw: String) {
      if (t != token) return
      main.post {
        val send = onPrompt ?: return@post
        val text = gate.accept(raw) ?: return@post
        send(text)
      }
    }
  }
}

/** The page loads from a string with no base URL, so an `#anchor` link resolves against about:blank */
private const val IN_PAGE = "about:blank#"
private const val LOAD_TRIES = 5
private const val LOAD_BACKOFF_MS = 250L
/** CSS px stay dp whatever the system font scale (the page reads the app's own type size) */
private const val TEXT_ZOOM = 100
