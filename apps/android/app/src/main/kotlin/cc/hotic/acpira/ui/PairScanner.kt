package cc.hotic.acpira.ui

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.net.Uri
import android.provider.Settings
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.BlendMode
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalLifecycleOwner
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.i18n.t
import com.google.zxing.BarcodeFormat
import com.google.zxing.BinaryBitmap
import com.google.zxing.DecodeHintType
import com.google.zxing.LuminanceSource
import com.google.zxing.qrcode.decoder.Decoder
import com.google.zxing.qrcode.decoder.QRCodeDecoderMetaData
import com.google.zxing.qrcode.detector.Detector
import cc.hotic.acpira.logic.PairCodeRows
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.common.HybridBinarizer
import java.util.concurrent.Executors

// The camera card that leads the add-computer sheet (apps/ios Pairing/PairScanner.swift): CameraX frames read by ZXing
// for QR codes only, under a viewfinder. Where the phone has no camera the card is a quiet stand-in; where access was
// refused it says so and opens the app's settings. Any QR code that is not an `acpira://pair/…` link is ignored.

private enum class CameraSource { Live, Unavailable, Denied }

@Composable
fun PairCameraCard(onLink: (String, List<String>?) -> Unit) {
  val context = LocalContext.current
  val lifecycle = LocalLifecycleOwner.current
  fun granted() = ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED
  val hasCamera = remember { context.packageManager.hasSystemFeature(PackageManager.FEATURE_CAMERA_ANY) }
  var source by remember { mutableStateOf<CameraSource?>(if (!hasCamera) CameraSource.Unavailable else if (granted()) CameraSource.Live else null) }
  val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { ok ->
    source = if (ok) CameraSource.Live else CameraSource.Denied
  }
  // The system asks on first use; a refusal shows the way to the app's settings
  LaunchedEffect(Unit) { if (source == null) ask.launch(Manifest.permission.CAMERA) }
  // Access granted in the settings meanwhile: the card comes back to life on return
  DisposableEffect(lifecycle) {
    val observer = LifecycleEventObserver { _, event ->
      if (event == Lifecycle.Event.ON_RESUME && source == CameraSource.Denied && granted()) source = CameraSource.Live
    }
    lifecycle.lifecycle.addObserver(observer)
    onDispose { lifecycle.lifecycle.removeObserver(observer) }
  }

  Box(Modifier.fillMaxSize().background(Palette.formRow).testTag("pair.camera"), contentAlignment = Alignment.Center) {
    when (source) {
      CameraSource.Live -> {
        CameraFeed(onLink)
        Finder(Color.White, dim = true)
      }
      CameraSource.Denied -> Column(
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(Metrics.gap),
        modifier = Modifier.padding(horizontal = Metrics.page),
      ) {
        Lucide("camera", Palette.fg3, size = Metrics.pairGlyph)
        Text(t("mobile.pair.camera.denied"), color = Palette.fg2, fontSize = Type.text2, textAlign = TextAlign.Center)
        OutlinedButton(onClick = {
          context.startActivity(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.fromParts("package", context.packageName, null)))
        }, modifier = Modifier.testTag("pair.camera.settings")) { Text(t("mobile.pair.camera.settings"), color = Palette.fg1) }
      }
      CameraSource.Unavailable -> {
        // The viewfinder alone, faint, so the card keeps its shape and says where a camera would be
        Finder(Palette.line, dim = false)
        Text(t("mobile.camera.unavailable"), color = Palette.fg3, fontSize = Type.text3)
      }
      null -> {}
    }
  }
}

/** The back camera's preview with a QR reader on its frames; a code read once is not handed over again */
@Composable
private fun CameraFeed(onLink: (String, List<String>?) -> Unit) {
  val context = LocalContext.current
  val lifecycle = LocalLifecycleOwner.current
  val latest by rememberUpdatedState(onLink)
  val executor = remember { Executors.newSingleThreadExecutor() }
  val preview = remember { PreviewView(context).apply { scaleType = PreviewView.ScaleType.FILL_CENTER } }
  DisposableEffect(lifecycle) {
    val future = ProcessCameraProvider.getInstance(context)
    var provider: ProcessCameraProvider? = null
    var last: String? = null
    future.addListener({
      val p = future.get()
      provider = p
      val shown = Preview.Builder().build().also { it.surfaceProvider = preview.surfaceProvider }
      val analysis = ImageAnalysis.Builder().setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST).build()
      val reader = QrReader()
      analysis.setAnalyzer(executor) { image ->
        val read = reader.read(image)
        image.close()
        if (read != null && read.first != last) {
          last = read.first
          ContextCompat.getMainExecutor(context).execute { latest(read.first, read.second) }
        }
      }
      runCatching {
        p.unbindAll()
        p.bindToLifecycle(lifecycle, CameraSelector.DEFAULT_BACK_CAMERA, shown, analysis)
      }
    }, ContextCompat.getMainExecutor(context))
    onDispose {
      provider?.unbindAll()
      executor.shutdown()
    }
  }
  AndroidView(factory = { preview }, modifier = Modifier.fillMaxSize())
}

/**
 * ZXing on a frame's luminance plane: QR only, light-on-dark too (a dark-theme pairing card). Detected and decoded in
 * two steps rather than through a reader, so the sampled module grid comes along with the text: the sheet draws the
 * card's very modules (logic/PairCode.kt)
 */
private class QrReader {
  private val hints = mapOf(DecodeHintType.POSSIBLE_FORMATS to listOf(BarcodeFormat.QR_CODE))
  private val decoder = Decoder()

  /** The text and the code's rows, or null when no code is read in the frame */
  fun read(image: ImageProxy): Pair<String, List<String>?>? {
    val plane = image.planes.firstOrNull() ?: return null
    val buffer = plane.buffer
    val stride = plane.rowStride
    val data = ByteArray(buffer.remaining()).also { buffer.get(it) }
    val source = PlanarYUVLuminanceSource(data, stride, image.height, 0, 0, image.width, image.height, false)
    return read(source) ?: read(source.invert())
  }

  private fun read(source: LuminanceSource): Pair<String, List<String>?>? = try {
    val found = Detector(BinaryBitmap(HybridBinarizer(source)).blackMatrix).detect(hints)
    // Decoding unmasks the grid in place: the rows are taken from a copy first
    val grid = found.bits.clone()
    val result = decoder.decode(found.bits, hints)
    // Read mirrored (a front camera): the grid as the card shows it is the transpose
    val mirrored = (result.other as? QRCodeDecoderMetaData)?.isMirrored == true
    val rows = PairCodeRows.of(grid)?.let { r -> if (mirrored) r.indices.map { x -> String(CharArray(r.size) { y -> r[y][x] }) } else r }
    result.text to rows
  } catch (_: Exception) {
    null
  }
}

/** The viewfinder: four rounded corners, the picture dimmed outside it */
@Composable
private fun Finder(ink: Color, dim: Boolean) {
  Canvas(Modifier.fillMaxSize().graphicsLayer { compositingStrategy = CompositingStrategy.Offscreen }) {
    val side = Metrics.pairFinder.toPx()
    val corner = Metrics.pairFinderCorner.toPx()
    val radius = Metrics.pairFinderRadius.toPx()
    val stroke = Metrics.pairFinderStroke.toPx()
    val topLeft = Offset((size.width - side) / 2, (size.height - side) / 2)
    if (dim) {
      // The picture dimmed outside the finder, over a camera picture in either theme
      drawRect(Color.Black.copy(alpha = FINDER_DIM))
      drawRoundRect(Color.Transparent, topLeft, Size(side, side), CornerRadius(radius), blendMode = BlendMode.Clear)
    }
    val style = Stroke(width = stroke, cap = StrokeCap.Round)
    val l = topLeft.x
    val t = topLeft.y
    val r = l + side
    val b = t + side
    // Each corner: a short run along both edges, joined by the finder's own rounding
    for ((cx, cy, sx, sy) in listOf(Quad(l, t, 1f, 1f), Quad(r, t, -1f, 1f), Quad(l, b, 1f, -1f), Quad(r, b, -1f, -1f))) {
      val path = androidx.compose.ui.graphics.Path().apply {
        moveTo(cx, cy + sy * corner)
        lineTo(cx, cy + sy * radius)
        quadraticTo(cx, cy, cx + sx * radius, cy)
        lineTo(cx + sx * corner, cy)
      }
      drawPath(path, ink, style = style)
    }
  }
}

private data class Quad(val x: Float, val y: Float, val sx: Float, val sy: Float)

/** The camera dimmed outside the viewfinder */
private const val FINDER_DIM = 0.22f
