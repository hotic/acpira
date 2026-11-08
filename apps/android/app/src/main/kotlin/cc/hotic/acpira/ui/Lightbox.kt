package cc.hotic.acpira.ui

import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.os.Environment
import android.provider.MediaStore
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.animate
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.spring
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.calculateCentroid
import androidx.compose.foundation.gestures.calculatePan
import androidx.compose.foundation.gestures.calculateZoom
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.PathParser
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.positionChange
import androidx.compose.ui.input.pointer.util.VelocityTracker
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.compose.ui.window.DialogWindowProvider
import androidx.core.content.FileProvider
import androidx.core.view.WindowCompat
import cc.hotic.acpira.design.LocalDark
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.RowMotion
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.model.SessionView
import cc.hotic.acpira.store.DecodedImage
import cc.hotic.acpira.store.MediaLoader
import cc.hotic.acpira.store.MediaSource
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File
import kotlin.math.abs
import kotlin.math.max
import kotlin.math.min

// apps/ios Media/Lightbox.swift (chat/Lightbox.tsx on the phone): the picture over black, full screen. Pinch or a double
// tap zooms it, a zoomed picture pans, and a swipe down at rest closes it, as ✕ does; share hands it to the system share
// sheet and save puts it in the phone's pictures. A tap hides the bars. A sideways swipe turns to the session's previous
// or next image, as Photos does; a zoomed picture pans instead, so it is zoomed out before the page turns.

/** The Lightbox's arithmetic (LightboxGeometry), apart from the views */
object LightboxGeometry {
  /** Lightbox.tsx MAX_SCALE: the least a picture zooms to; a larger one goes on to its own pixels */
  const val MIN_MAX_ZOOM = 4f
  /** How far a double tap zooms in (a second one zooms out) */
  const val DOUBLE_TAP_ZOOM = 2.5f
  /** How much the picture shrinks at a full pull */
  const val PULL_SHRINK = 0.25f
  /** A pull past this share of the screen's height closes on release, as does a downward flick faster than [CLOSE_SPEED] */
  const val CLOSE_SHARE = 0.15f
  /** dp a second */
  const val CLOSE_SPEED = 800f

  /** The picture fitted into the screen, aspect kept (enlarged when smaller), clamped to the screen */
  fun fit(w: Float, h: Float, boundW: Float, boundH: Float): Pair<Float, Float> {
    if (w <= 0 || h <= 0) return 0f to 0f
    val scale = min(boundW / w, boundH / h)
    return min(boundW, w * scale) to min(boundH, h * scale)
  }

  /** The deepest zoom: [MIN_MAX_ZOOM], or further while a screen pixel still meets a picture pixel */
  fun maxZoom(pixelW: Float, fittedW: Float): Float = if (fittedW <= 0) MIN_MAX_ZOOM else max(MIN_MAX_ZOOM, pixelW / fittedW)

  /** How far a downward pull has gone, 0 at rest to 1 at half the screen's height */
  fun pull(dy: Float, height: Float): Float = if (height <= 0) 0f else (dy / (height / 2)).coerceIn(0f, 1f)

  /** Whether releasing a pull closes the Lightbox */
  fun closes(dy: Float, velocity: Float, height: Float, closeSpeed: Float): Boolean =
    dy > height * CLOSE_SHARE || (dy > 0 && velocity > closeSpeed)
}

/** One page of the Lightbox: its pixels when already at hand (the tapped picture), else where to load them */
class LightboxPage(val source: MediaSource?, val name: String?, val image: DecodedImage?)

/**
 * The pages the Lightbox swipes through: the session's images in reading order, opened at this one, which keeps the
 * pixels and name its view gave it. One not among them (no source or session, a photo not yet sent) stands alone
 */
fun gallery(item: LightboxItem, view: SessionView?): Pair<List<LightboxPage>, Int> {
  val tapped = LightboxPage(item.source, item.name, item.image)
  val source = item.source ?: return listOf(tapped) to 0
  view ?: return listOf(tapped) to 0
  val pages = MediaLoader.entries(view).map { LightboxPage(it.source, it.name, null) }.toMutableList()
  val start = pages.indexOfFirst { it.source == source }
  if (start < 0) return listOf(tapped) to 0
  pages[start] = tapped
  return pages to start
}

@Composable
fun Lightbox(item: LightboxItem, loader: MediaLoader, session: () -> SessionView?, motion: Boolean, onClose: () -> Unit) {
  val context = LocalContext.current
  val scope = rememberCoroutineScope()
  val (pages, start) = remember(item) { gallery(item, session()) }
  val pager = rememberPagerState(start) { pages.size }
  /** The pixels each built page has, for the share and save buttons */
  val images = remember(item) { mutableStateMapOf<Int, DecodedImage>().apply { item.image.let { put(start, it) } } }
  var shown by remember { mutableStateOf(false) }
  var closing by remember { mutableStateOf(false) }
  var pull by remember { mutableFloatStateOf(0f) }
  var bars by remember { mutableStateOf(true) }
  var zoomed by remember { mutableStateOf(false) }
  var saved by remember { mutableStateOf(false) }
  val fade by animateFloatAsState(
    if (shown && !closing) 1f else 0f,
    if (!motion) tween(0) else if (closing) tween(CLOSE_MS) else spring(stiffness = RowMotion.foldStiffness),
    label = "lightbox",
    finishedListener = { if (closing) onClose() },
  )
  LaunchedEffect(Unit) { shown = true }
  LaunchedEffect(saved) { if (saved) { delay(SAVED_MS); saved = false } }
  val close = { if (!closing) { closing = true; if (!motion) onClose() } }

  Dialog(
    onDismissRequest = close,
    properties = DialogProperties(usePlatformDefaultWidth = false, decorFitsSystemWindows = false),
  ) {
    // The dialog's own dim and the dark bars' light icons: the black is drawn here, faded in and pulled away
    val window = (LocalView.current.parent as? DialogWindowProvider)?.window
    val view = LocalView.current
    SideEffect {
      window?.setDimAmount(0f)
      window?.let { WindowCompat.getInsetsController(it, view).isAppearanceLightStatusBars = false }
    }
    CompositionLocalProvider(LocalDark provides true) {
      Box(Modifier.fillMaxSize().testTag("lightbox")) {
        Box(Modifier.fillMaxSize().background(Color.Black.copy(alpha = fade * (1 - pull))))
        HorizontalPager(
          pager,
          modifier = Modifier.fillMaxSize().alpha(fade),
          pageSpacing = Metrics.page,
          userScrollEnabled = !zoomed && pull == 0f,
          beyondViewportPageCount = 1,
        ) { i ->
          LightboxPageView(
            pages[i], loader, motion,
            onImage = { images[i] = it },
            onZoom = { if (i == pager.currentPage) zoomed = it },
            onPull = { pull = it },
            onClose = close,
            onTap = { bars = !bars },
          )
        }
        val current = images[pager.currentPage]
        AnimatedVisibility(
          visible = shown && !closing && bars && pull == 0f,
          enter = fadeIn(tween(if (motion) QUICK_MS else 0)),
          exit = fadeOut(tween(if (motion) QUICK_MS else 0)),
        ) {
          Row(
            Modifier
              .fillMaxWidth()
              // A scrim under the bar keeps its title and buttons legible over a light picture (the iOS bar is glass)
              .background(androidx.compose.ui.graphics.Brush.verticalGradient(listOf(Palette.scrim, Palette.scrim, Color.Transparent)))
              .statusBarsPadding()
              .padding(horizontal = Metrics.s1)
              .height(Metrics.barCtl + Metrics.s2),
            verticalAlignment = Alignment.CenterVertically,
          ) {
            IconButton(onClick = close, modifier = Modifier.testTag("lightbox.close")) {
              Lucide("x", Palette.fg1, size = Metrics.iconCtl, contentDescription = t("common.close"))
            }
            Text(
              pages.getOrNull(pager.currentPage)?.name.orEmpty(),
              color = Palette.fg1, fontSize = Type.text1, fontWeight = FontWeight.SemiBold, textAlign = TextAlign.Center,
              maxLines = 1, overflow = TextOverflow.MiddleEllipsis, modifier = Modifier.weight(1f),
            )
            if (current != null) {
              val name = pages.getOrNull(pager.currentPage)?.name
              IconButton(
                onClick = { scope.launch { if (saveImage(context, current, name)) saved = true } },
                modifier = Modifier.testTag("lightbox.save"),
              ) { Lucide(if (saved) "check" else "download", Palette.fg1, size = Metrics.iconCtl, contentDescription = t("mobile.save")) }
              IconButton(onClick = { scope.launch { shareImage(context, current, name) } }, modifier = Modifier.testTag("lightbox.share")) {
                Icon(shareGlyph, t("mobile.share"), Modifier.padding(Metrics.s0_5), tint = Palette.fg1)
              }
            }
          }
        }
      }
    }
  }
}

/** One page: its picture in a zooming frame, or a spinner while its pixels load, or the image glyph when they cannot */
@Composable
private fun LightboxPageView(
  page: LightboxPage,
  loader: MediaLoader,
  motion: Boolean,
  onImage: (DecodedImage) -> Unit,
  onZoom: (Boolean) -> Unit,
  onPull: (Float) -> Unit,
  onClose: () -> Unit,
  onTap: () -> Unit,
) {
  var image by remember(page) { mutableStateOf(page.image ?: page.source?.let(loader::peek)) }
  var failed by remember(page) { mutableStateOf(page.source == null && page.image == null) }
  LaunchedEffect(page) {
    if (image == null && page.source != null) {
      image = runCatching { loader.image(page.source) }.getOrNull()
      if (image == null) failed = true
    }
    image?.let(onImage)
  }
  val shown = image
  when {
    shown != null -> ZoomPage(shown, motion, onZoom, onPull, onClose, onTap)
    else -> Box(
      Modifier.fillMaxSize().pointerInput(Unit) { detectTapGestures(onTap = { onTap() }) }
        .semantics { if (failed) contentDescription = t("attach.loadFailed", "name" to (page.name ?: t("common.image"))) },
      contentAlignment = Alignment.Center,
    ) {
      if (failed) Lucide("image", Palette.fg3, size = Metrics.iconCtl)
      else CircularProgressIndicator(color = Palette.fg3, strokeWidth = Metrics.s0_5, modifier = Modifier.padding(Metrics.s1))
    }
  }
}

/**
 * The picture in a zooming frame: pinch and double tap zoom, a zoomed picture pans within its edges, and a one-finger
 * downward pan at rest pulls it away ([onPull] 0…1), closing past the threshold. A sideways drag at rest is left to the
 * pager
 */
@Composable
private fun ZoomPage(image: DecodedImage, motion: Boolean, onZoom: (Boolean) -> Unit, onPull: (Float) -> Unit, onClose: () -> Unit, onTap: () -> Unit) {
  val scope = rememberCoroutineScope()
  val density = LocalDensity.current
  BoxWithConstraints(Modifier.fillMaxSize()) {
    val w = constraints.maxWidth.toFloat()
    val h = constraints.maxHeight.toFloat()
    val (fw, fh) = LightboxGeometry.fit(image.width.toFloat(), image.height.toFloat(), w, h)
    val maxZoom = LightboxGeometry.maxZoom(image.width.toFloat(), fw)
    val closeSpeed = with(density) { LightboxGeometry.CLOSE_SPEED.dp.toPx() }
    var scale by remember(image) { mutableFloatStateOf(1f) }
    var offset by remember(image) { mutableStateOf(Offset.Zero) }
    var pullAt by remember(image) { mutableStateOf(Offset.Zero) }

    fun clamp(o: Offset, s: Float): Offset {
      val mx = max(0f, (fw * s - w) / 2)
      val my = max(0f, (fh * s - h) / 2)
      return Offset(o.x.coerceIn(-mx, mx), o.y.coerceIn(-my, my))
    }
    fun zoomTo(target: Float, focus: Offset) {
      val from = scale
      val fromOffset = offset
      // The tapped point comes to the middle, as UIScrollView.zoom(to:) shows the rect around it
      val content = (focus - Offset(w / 2, h / 2) - fromOffset) / from
      val toOffset = if (target <= 1f) Offset.Zero else clamp(-content * target, target)
      scope.launch {
        animate(0f, 1f, animationSpec = if (motion) spring(stiffness = RowMotion.foldStiffness) else tween(0)) { p, _ ->
          scale = from + (target - from) * p
          offset = fromOffset + (toOffset - fromOffset) * p
        }
        onZoom(scale > 1f)
      }
    }

    Picture(
      image,
      Modifier
        .fillMaxSize()
        .graphicsLayer {
          val shrink = 1 - LightboxGeometry.pull(pullAt.y, h) * LightboxGeometry.PULL_SHRINK
          scaleX = scale * shrink
          scaleY = scale * shrink
          translationX = offset.x + pullAt.x
          translationY = offset.y + pullAt.y
        }
        .pointerInput(image, w, h) {
          detectTapGestures(
            onDoubleTap = { p -> zoomTo(if (scale > 1f) 1f else min(maxZoom, LightboxGeometry.DOUBLE_TAP_ZOOM), p) },
            onTap = { onTap() },
          )
        }
        .pointerInput(image, w, h) {
          awaitEachGesture {
            awaitFirstDown(requireUnconsumed = false)
            // undecided, a pinch or pan of the picture, the pull, or a drag left to the pager
            var mode = Gesture.Undecided
            var drag = Offset.Zero
            val tracker = VelocityTracker()
            do {
              val event = awaitPointerEvent()
              val pressed = event.changes.filter { it.pressed }
              if (pressed.size >= 2 && mode != Gesture.Pull && mode != Gesture.Pass) {
                mode = Gesture.Transform
                val zoom = event.calculateZoom()
                val pan = event.calculatePan()
                val focus = event.calculateCentroid(useCurrent = true) - Offset(w / 2, h / 2)
                val next = (scale * zoom).coerceIn(1f, maxZoom)
                offset = clamp(focus - (focus - offset) * (next / scale) + pan, next)
                scale = next
                event.changes.forEach { it.consume() }
              } else if (pressed.size == 1) {
                val change = pressed[0]
                val delta = change.positionChange()
                when {
                  mode == Gesture.Pull -> {
                    pullAt += delta
                    tracker.addPosition(change.uptimeMillis, change.position)
                    onPull(LightboxGeometry.pull(pullAt.y, h))
                    change.consume()
                  }
                  mode == Gesture.Pass -> {}
                  scale > 1f -> {
                    mode = Gesture.Transform
                    offset = clamp(offset + delta, scale)
                    change.consume()
                  }
                  mode == Gesture.Undecided -> {
                    drag += delta
                    if (drag.getDistance() > viewConfiguration.touchSlop) {
                      // The pull only starts at rest and downward, so a sideways drag turns the page
                      mode = if (drag.y > abs(drag.x)) Gesture.Pull else Gesture.Pass
                      if (mode == Gesture.Pull) {
                        tracker.addPosition(change.uptimeMillis, change.position)
                        change.consume()
                      }
                    }
                  }
                }
              }
            } while (event.changes.any { it.pressed })
            if (mode == Gesture.Pull) {
              val velocity = tracker.calculateVelocity().y
              if (LightboxGeometry.closes(pullAt.y, velocity, h, closeSpeed)) {
                onClose()
                // Keep going the way it was released while the Lightbox fades out
                val from = pullAt
                val to = Offset(from.x, from.y + h)
                if (motion) scope.launch { animate(0f, 1f, animationSpec = tween(CLOSE_MS)) { p, _ -> pullAt = from + (to - from) * p } }
              } else {
                val from = pullAt
                scope.launch {
                  animate(0f, 1f, animationSpec = if (motion) spring(stiffness = RowMotion.foldStiffness) else tween(0)) { p, _ ->
                    pullAt = from * (1 - p)
                    onPull(LightboxGeometry.pull(pullAt.y, h))
                  }
                  onPull(0f)
                }
              }
            }
            onZoom(scale > 1f)
          }
        }
        .testTag("lightbox.page"),
    )
  }
}

private enum class Gesture { Undecided, Transform, Pull, Pass }

/** The system share sheet's glyph (lucide share-2; the webview imports no share icon, so it is not in icons.json) */
internal val shareGlyph: ImageVector by lazy {
  ImageVector.Builder("share2", 24.dp, 24.dp, 24f, 24f).apply {
    for (d in SHARE_PATHS) {
      addPath(
        PathParser().parsePathString(d).toNodes(), stroke = SolidColor(Color.Black), strokeLineWidth = 2f,
        strokeLineCap = StrokeCap.Round, strokeLineJoin = StrokeJoin.Round,
      )
    }
  }.build()
}

private val SHARE_PATHS = listOf(
  "M15 5a3 3 0 1 0 6 0a3 3 0 1 0 -6 0z", "M3 12a3 3 0 1 0 6 0a3 3 0 1 0 -6 0z", "M15 19a3 3 0 1 0 6 0a3 3 0 1 0 -6 0z",
  "M8.59 13.51L15.42 17.49", "M15.41 6.51L8.59 10.49",
)

/** The picture as a file to hand out: an animation's own bytes, else the pixels as PNG */
private suspend fun imageFile(context: Context, image: DecodedImage, name: String?): Pair<File, String> = withContext(Dispatchers.IO) {
  val dir = File(context.cacheDir, "share").apply { mkdirs() }
  val animation = image.animation
  val gif = animation != null && animation.size > 3 && animation[0] == 'G'.code.toByte() && animation[1] == 'I'.code.toByte()
  val (ext, mime) = when {
    animation == null -> "png" to "image/png"
    gif -> "gif" to "image/gif"
    else -> "webp" to "image/webp"
  }
  val stem = name?.substringBeforeLast('.')?.takeIf { it.isNotBlank() }?.replace(Regex("[/\\\\:]"), "_") ?: "image"
  val file = File(dir, "$stem.$ext")
  if (animation != null) file.writeBytes(animation)
  else {
    // A hardware bitmap cannot be compressed: a software copy first
    val bitmap = if (image.bitmap.config == Bitmap.Config.HARDWARE) image.bitmap.copy(Bitmap.Config.ARGB_8888, false) else image.bitmap
    file.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, PNG_QUALITY, it) }
  }
  file to mime
}

/** The system share sheet with the picture (Copy, Nearby Share, apps) */
private suspend fun shareImage(context: Context, image: DecodedImage, name: String?) {
  val (file, mime) = imageFile(context, image, name)
  val uri = FileProvider.getUriForFile(context, "${context.packageName}.files", file)
  val send = Intent(Intent.ACTION_SEND).setType(mime).putExtra(Intent.EXTRA_STREAM, uri).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
  // The clip lets the share sheet preview the picture and offer Copy
  send.clipData = android.content.ClipData.newUri(context.contentResolver, file.name, uri)
  context.startActivity(Intent.createChooser(send, name ?: t("common.image")).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
}

/** The picture into the phone's Pictures/Acpira (MediaStore, no permission from Android 10 on); false when it failed */
private suspend fun saveImage(context: Context, image: DecodedImage, name: String?): Boolean = withContext(Dispatchers.IO) {
  runCatching {
    val (file, mime) = imageFile(context, image, name)
    val resolver = context.contentResolver
    val values = ContentValues().apply {
      put(MediaStore.Images.Media.DISPLAY_NAME, file.name)
      put(MediaStore.Images.Media.MIME_TYPE, mime)
      put(MediaStore.Images.Media.RELATIVE_PATH, "${Environment.DIRECTORY_PICTURES}/Acpira")
      put(MediaStore.Images.Media.IS_PENDING, 1)
    }
    val uri = resolver.insert(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, values) ?: return@runCatching false
    resolver.openOutputStream(uri)?.use { out -> file.inputStream().use { it.copyTo(out) } }
    resolver.update(uri, ContentValues().apply { put(MediaStore.Images.Media.IS_PENDING, 0) }, null, null)
    true
  }.getOrDefault(false)
}

/** Motion.durClose: the Lightbox fading out */
private const val CLOSE_MS = 320
/** How long save's tick stands before the glyph returns */
private const val SAVED_MS = 1_500L
private const val PNG_QUALITY = 100
