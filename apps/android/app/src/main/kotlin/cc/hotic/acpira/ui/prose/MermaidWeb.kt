package cc.hotic.acpira.ui.prose

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.BitmapFactory
import android.util.Base64
import android.util.Log
import android.view.View
import android.webkit.JavascriptInterface
import android.webkit.WebResourceRequest
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.widthIn
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.unit.dp
import cc.hotic.acpira.design.LocalDark
import cc.hotic.acpira.design.MermaidWebMetrics
import java.util.concurrent.ConcurrentHashMap
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.delay
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import org.json.JSONObject

/*
 * The mermaid diagrams the native flowchart port does not read (sequence, pie, state, class, gantt …): mermaid's own
 * browser build (asset web/mermaid.min.js, synced from the iOS bundle: the version the webview renders with) renders
 * them in one shared, unparented WebView, and the SVG comes back as a PNG drawn through a canvas, so the transcript
 * holds a plain image instead of a live web view per diagram. Labels are SVG text (`htmlLabels: false`): a
 * foreignObject label would taint the canvas. A source mermaid cannot parse stays the code card, as a failed render
 * does in the webview.
 *
 * Ported from the iOS app's `AcpiraUI/Web/MermaidWeb.swift`.
 */

/** A rendered diagram: its pixels and the SVG's own size in dp */
class MermaidPicture(val image: ImageBitmap, val width: Float, val height: Float)

/** The one web view every diagram renders in, one at a time; pictures are cached by source, theme and scale */
object MermaidRenderer {
  private const val TAG = "AcpiraWeb"

  /** Renders outlive the row that asked (as the iOS renderer's unstructured tasks do), so the page is never mid-render
   * when the next one starts, and a picture that lands after its row scrolled away is cached for its return */
  private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
  private val lock = Mutex()
  private var web: WebView? = null
  private var loaded: CompletableDeferred<Unit>? = null
  private val pending = ConcurrentHashMap<String, CompletableDeferred<String>>()
  /** Touched on the main thread only */
  private val cache = HashMap<String, MermaidPicture>()
  private var serial = 0

  /** streamdown's mermaid defaults (the webview's config) plus SVG labels; mermaid comes from the app's assets */
  private const val PAGE = """<!doctype html><html><head><meta charset="utf-8"><script src="web/mermaid.min.js"></script></head><body><script>
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
function acpiraRun(id, source, dark, scale) {
  acpiraRender(id, source, dark, scale).then(
    (r) => AcpiraMermaid.done(id, JSON.stringify(r)),
    (e) => AcpiraMermaid.fail(id, String((e && e.message) || e)));
}
</script></body></html>"""

  private fun key(source: String, dark: Boolean, scale: Float) = "${if (dark) 1 else 0}\u0001$scale\u0001$source"

  /** A picture already made, for the first frame of a row that comes back */
  fun cached(source: String, dark: Boolean, scale: Float): MermaidPicture? = cache[key(source, dark, scale)]

  suspend fun picture(context: Context, source: String, dark: Boolean, scale: Float): MermaidPicture {
    val key = key(source, dark, scale)
    val job = scope.async {
      cache[key] ?: lock.withLock {
        // One render at a time: mermaid's render keeps state on the page between its awaits
        cache[key] ?: render(context.applicationContext, source, dark, scale).also {
          if (cache.size > MermaidWebMetrics.cacheLimit) cache.clear()
          cache[key] = it
        }
      }
    }
    return job.await()
  }

  private suspend fun render(context: Context, source: String, dark: Boolean, scale: Float): MermaidPicture {
    serial += 1
    val id = "acpira-mermaid-$serial"
    val done = CompletableDeferred<String>()
    pending[id] = done
    // The page load and the render together: a page that never finishes or never calls back must not hold the lock
    val json = try {
      withTimeoutOrNull(MermaidWebMetrics.renderTimeoutMs) {
        ready(context).evaluateJavascript(
          "acpiraRun(${JSONObject.quote(id)}, ${JSONObject.quote(source)}, $dark, $scale)", null,
        )
        done.await()
      } ?: error("mermaid: render timed out")
    } finally {
      pending.remove(id)
    }
    return withContext(Dispatchers.Default) {
      val result = JSONObject(json)
      val bytes = Base64.decode(result.getString("png"), Base64.DEFAULT)
      val bitmap = BitmapFactory.decodeByteArray(bytes, 0, bytes.size) ?: error("mermaid: undecodable picture")
      MermaidPicture(bitmap.asImageBitmap(), result.getDouble("width").toFloat(), result.getDouble("height").toFloat())
    }
  }

  /** Callbacks from the page, on the WebView's JavaBridge thread */
  private class Bridge {
    @JavascriptInterface
    fun done(id: String, json: String) {
      pending.remove(id)?.complete(json)
    }

    @JavascriptInterface
    fun fail(id: String, message: String) {
      pending.remove(id)?.completeExceptionally(IllegalStateException(message))
    }
  }

  /** The page with mermaid loaded, made on the first diagram (main thread) */
  @SuppressLint("SetJavaScriptEnabled")
  private suspend fun ready(context: Context): WebView {
    val existing = web
    val wait = loaded
    if (existing != null && wait != null) {
      wait.await()
      return existing
    }
    val signal = CompletableDeferred<Unit>()
    val view = WebView(context).apply {
      settings.javaScriptEnabled = true
      // No network: mermaid and the page come from the app's assets
      settings.blockNetworkLoads = true
      settings.allowFileAccess = false
      settings.allowContentAccess = false
      addJavascriptInterface(Bridge(), "AcpiraMermaid")
      webViewClient = object : WebViewClient() {
        override fun onPageFinished(view: WebView, url: String?) {
          signal.complete(Unit)
        }

        override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest) = true
      }
      // Never on screen: a viewport of its own so mermaid's measurements see a page, as the iOS view's frame
      val w = View.MeasureSpec.makeMeasureSpec(MermaidWebMetrics.viewportWidth, View.MeasureSpec.EXACTLY)
      val h = View.MeasureSpec.makeMeasureSpec(MermaidWebMetrics.viewportHeight, View.MeasureSpec.EXACTLY)
      measure(w, h)
      layout(0, 0, MermaidWebMetrics.viewportWidth, MermaidWebMetrics.viewportHeight)
    }
    web = view
    loaded = signal
    // android_asset stays readable with file access off
    view.loadDataWithBaseURL("file:///android_asset/", PAGE, "text/html", "utf-8", null)
    signal.await()
    return view
  }

  internal fun log(error: Throwable) {
    Log.i(TAG, "mermaid: $error")
  }
}

/**
 * A mermaid block drawn by mermaid itself: [fallback] (the code card) while the picture is made and if mermaid cannot
 * read it, then the picture at the SVG's own size, scaled down to the column when wider
 */
@Composable
internal fun MermaidWebBlock(source: String, modifier: Modifier, fallback: @Composable () -> Unit) {
  val context = LocalContext.current
  val dark = LocalDark.current
  val scale = maxOf(LocalDensity.current.density, MermaidWebMetrics.minScale)
  // A row that comes back shows its picture at once; the iOS view waits its debounce even on a cache hit
  var picture by remember { mutableStateOf(MermaidRenderer.cached(source, dark, scale)) }
  LaunchedEffect(dark, source, scale) {
    MermaidRenderer.cached(source, dark, scale)?.let {
      picture = it
      return@LaunchedEffect
    }
    // MermaidBlock.tsx waits before its first try, so a fence still streaming is not rendered at every chunk
    delay(MermaidWebMetrics.renderDelayMs)
    // A source mermaid cannot read keeps what was there: the code, or the picture of the fence before its last chunk
    try {
      picture = MermaidRenderer.picture(context, source, dark, scale)
    } catch (e: CancellationException) {
      throw e
    } catch (e: Exception) {
      MermaidRenderer.log(e)
    }
  }
  val shown = picture
  if (shown != null) {
    Box(modifier.fillMaxWidth().clearAndSetSemantics {}, contentAlignment = Alignment.TopCenter) {
      Image(
        bitmap = shown.image,
        contentDescription = null,
        modifier = Modifier.widthIn(max = shown.width.dp).fillMaxWidth().aspectRatio(shown.width / maxOf(shown.height, 1f)),
        contentScale = ContentScale.Fit,
      )
    }
  } else {
    Box(modifier) { fallback() }
  }
}
