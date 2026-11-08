package cc.hotic.acpira.ui

import android.content.Context
import android.graphics.Bitmap
import android.media.MediaMetadataRetriever
import android.net.Uri
import android.os.Handler
import android.os.Looper
import android.text.format.Formatter
import android.widget.FrameLayout
import android.widget.MediaController
import android.widget.VideoView
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.IconButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.compose.ui.window.DialogWindowProvider
import androidx.core.view.WindowCompat
import cc.hotic.acpira.design.LocalDark
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.design.WebMetrics
import cc.hotic.acpira.design.WebType
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.model.ImageRef
import cc.hotic.acpira.store.MediaLoader
import cc.hotic.acpira.store.MediaSource
import cc.hotic.acpira.store.isVideoName
import cc.hotic.acpira.sync.BlobNames
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File
import java.io.IOException

// apps/ios Media/VideoAttachment.swift and AgentVideoView.swift, DownloadProgress.swift: a video attached to a prompt
// is a tile showing its first frame, a `show_video` recording a card that stays dormant until tapped; either fetches
// the file once into the phone's video cache and plays it in the phone's own player (VideoView, as iOS uses Quick Look).
// A video the phone sent is parked by the engine in the session's blob store and linked by path, so the path's last
// component is a blob name the phone fetches like a recording; the phone keeps its own copy under that name at send time.

/** Where a video's bytes are */
sealed interface VideoSource {
  /** The phone's own copy of a picked video (staged, or drawn before the engine has the prompt) */
  data class Local(val file: File) : VideoSource
  /** A file in the session's blob store */
  data class Blob(val session: String, val name: String) : VideoSource
  /** Any other file on the computer, staged by the engine on demand */
  data class Staged(val session: String, val path: String) : VideoSource

  companion object {
    /** The source for a `file` attachment, null when it is no video or nothing here can read it (it stays a pill) */
    fun of(context: Context, uri: String, name: String, session: String?, loader: MediaLoader): VideoSource? {
      if (!isVideoName(name)) return null
      val parsed = runCatching { Uri.parse(uri) }.getOrNull() ?: return null
      if (parsed.scheme != "file") return null
      val path = parsed.path ?: return null
      if (AttachmentFiles.isCopy(context, path)) return File(path).takeIf { it.exists() }?.let { Local(it) }
      session ?: return null
      val last = path.substringAfterLast('/')
      if (BlobNames.isBlobName(last)) return Blob(session, last)
      return if (loader.canStage) Staged(session, path) else null
    }
  }
}

/** The phone's copies of picked files: the composer stages them here, the send reads their bytes */
object AttachmentFiles {
  fun folder(context: Context) = File(context.cacheDir, "attachments")

  fun isCopy(context: Context, path: String) = File(path).canonicalPath.startsWith(folder(context).canonicalPath + File.separator)
}

/**
 * Downloaded and sent videos on disk by blob name (and files opened from the download page), and their first frames in
 * memory (VideoCache.swift). Blob names are content hashes, so one file serves every session holding the same bytes.
 * The system may clear the cache folder; a miss downloads again. A download goes to `<name>.partial` first, so one that
 * stopped resumes where it was on the next tap
 */
object VideoCache {
  /** How far a download is: bytes on the phone and the file's size (0 until the computer said) */
  data class Transfer(val have: Long = 0, val total: Long = 0) {
    /** 0…1, null while the size is unknown */
    val fraction: Float? get() = if (total > 0) (have.toFloat() / total).coerceAtMost(1f) else null
  }

  /** Why a file did not arrive, as the cards word it */
  sealed class Failure(message: String) : IOException(message) {
    /** The computer could not read the file: gone, unreadable, or over what the link carries */
    class Unreadable : Failure("unreadable")
    /** The transfer stopped (a stall, a lost link); the next try resumes */
    class Interrupted : Failure("interrupted")
  }

  /** The line a card shows for a failed open */
  fun message(error: Throwable): String =
    if (error is Failure.Unreadable || (error is MediaLoader.Failure && error.message == "not staged")) t("mobile.download.unreadable")
    else t("mobile.download.interrupted")

  /** Kept videos together; the oldest go first past it */
  private const val BUDGET = 512L shl 20

  fun folder(context: Context) = File(context.cacheDir, "videos")

  private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
  private val main = Handler(Looper.getMainLooper())
  private val files = HashMap<VideoSource, Deferred<File>>()
  private val posters = HashMap<VideoSource, ImageBitmap>()

  /**
   * Downloads under way, by source: the cards draw their progress from it, so a card scrolled away and back still shows a
   * download it started
   */
  val transfers = mutableStateMapOf<VideoSource, Transfer>()

  /** Keeps a video the phone is sending under the blob name the engine will give it; [read] runs off the main thread */
  fun adopt(context: Context, name: String, read: () -> ByteArray?) {
    val folder = folder(context)
    scope.launch {
      val bytes = runCatching(read).getOrNull() ?: return@launch
      val target = File(folder, BlobNames.ofFile(bytes, name))
      if (target.exists()) return@launch
      folder.mkdirs()
      val tmp = File(folder, target.name + ".tmp")
      tmp.writeBytes(bytes)
      tmp.renameTo(target)
      prune(folder)
    }
  }

  /** A playable file for the source: the phone's copy, or the blob downloaded once. Called on the main thread */
  suspend fun file(context: Context, source: VideoSource, loader: MediaLoader): File {
    if (source is VideoSource.Local) return source.file
    files[source]?.let { return it.await() }
    transfers[source] = Transfer()
    val folder = folder(context)
    val job = scope.async {
      val (session, name) = when (source) {
        is VideoSource.Blob -> source.session to source.name
        is VideoSource.Staged -> source.session to (loader.blobName(source.session, null, source.path) ?: throw Failure.Unreadable())
        is VideoSource.Local -> return@async source.file
      }
      val target = File(folder, name)
      if (target.exists()) return@async target
      folder.mkdirs()
      val partial = File(folder, "$name.partial")
      try {
        loader.download(session, name, partial) { have, total -> main.post { progressed(source, have, total) } }
      } catch (e: MediaLoader.Failure) {
        throw e
      } catch (e: Exception) {
        // A blob the computer no longer holds or will not send cannot come by trying again
        val text = e.message.orEmpty()
        throw if ("no such blob" in text || "too large" in text) Failure.Unreadable() else Failure.Interrupted()
      }
      target.delete()
      if (!partial.renameTo(target)) throw Failure.Interrupted()
      prune(folder)
      target
    }
    files[source] = job
    try {
      return job.await()
    } catch (e: Throwable) {
      files.remove(source)
      throw e
    } finally {
      transfers.remove(source)
    }
  }

  /** A download's progress, kept only while it is under way (a late report after its end is dropped) */
  private fun progressed(source: VideoSource, have: Long, total: Long) {
    val now = transfers[source] ?: return
    transfers[source] = Transfer(maxOf(now.have, have), total)
  }

  /** A poster already at hand, drawn without waiting */
  fun cached(source: VideoSource): ImageBitmap? = posters[source]

  /** Whether the source's file is on the phone already, so its poster costs no download */
  fun kept(context: Context, source: VideoSource): Boolean = when (source) {
    is VideoSource.Local -> source.file.exists()
    is VideoSource.Blob -> File(folder(context), source.name).exists()
    is VideoSource.Staged -> false
  }

  /** Keeps a poster taken elsewhere (a staged video's original while it compresses) */
  fun remember(image: ImageBitmap, source: VideoSource) {
    posters[source] = image
  }

  /** The video's first frame, at most [pixels] on its long side */
  suspend fun poster(context: Context, source: VideoSource, loader: MediaLoader, pixels: Int): ImageBitmap {
    posters[source]?.let { return it }
    val file = file(context, source, loader)
    val image = firstFrame(file, pixels)?.asImageBitmap() ?: throw Failure.Unreadable()
    posters[source] = image
    return image
  }

  /** A video file's first frame scaled to [pixels] on its long side; null when it cannot be read */
  suspend fun firstFrame(file: File, pixels: Int): Bitmap? = withContext(Dispatchers.IO) {
    val retriever = MediaMetadataRetriever()
    try {
      retriever.setDataSource(file.path)
      retriever.getScaledFrameAtTime(0, MediaMetadataRetriever.OPTION_CLOSEST_SYNC, pixels, pixels)
    } catch (_: Exception) {
      null
    } finally {
      retriever.release()
    }
  }

  /** Drops the least recently written videos past the budget */
  private fun prune(folder: File) {
    var total = 0L
    for (f in folder.listFiles().orEmpty().sortedByDescending { it.lastModified() }) {
      total += f.length()
      if (total > BUDGET) f.delete()
    }
  }
}

/**
 * A `show_video` recording (AgentVideoView.swift): a card that stays dormant until tapped (nothing is downloaded
 * before), then fetches the file once into the video cache and plays it. Once the file is on the phone the card shows its
 * first frame. While the file comes over, the card shows how far it is; a download that stopped says so and resumes on
 * the next tap
 */
@Composable
fun AgentVideoView(video: ImageRef) {
  val media = LocalMedia.current
  val context = LocalContext.current
  val scope = rememberCoroutineScope()
  val source = remember(video, media.session) {
    val session = media.session
    when {
      session == null -> null
      video.blob != null -> VideoSource.Blob(session, video.blob!!)
      video.uri != null && MediaSource.isFilePath(video.uri!!) && media.loader.canStage -> VideoSource.Staged(session, video.uri!!)
      else -> null
    }
  }
  val name = video.uri?.substringAfterLast('/') ?: t("verb.showVideo")
  val pixels = with(LocalDensity.current) { (Metrics.agentImage * 2).roundToPx() }
  var opening by remember { mutableStateOf(false) }
  var failure by remember { mutableStateOf<String?>(null) }
  var poster by remember(source) { mutableStateOf(source?.let(VideoCache::cached)) }
  var playing by remember { mutableStateOf<File?>(null) }
  val transfer = source?.let { VideoCache.transfers[it] }
  val busy = opening || transfer != null
  // A recording opened before (here or as the same bytes elsewhere) shows its first frame without a download
  LaunchedEffect(source) {
    if (poster != null || source == null || !VideoCache.kept(context, source)) return@LaunchedEffect
    poster = runCatching { VideoCache.poster(context, source, media.loader, pixels) }.getOrNull()
  }
  val open = open@{
    if (source == null) { failure = t("mobile.download.unreadable"); return@open }
    opening = true
    failure = null
    scope.launch {
      try {
        playing = VideoCache.file(context, source, media.loader)
        if (poster == null) poster = runCatching { VideoCache.poster(context, source, media.loader, pixels) }.getOrNull()
      } catch (e: Exception) {
        failure = VideoCache.message(e)
      } finally {
        opening = false
      }
    }
  }
  val shape = RoundedCornerShape(Metrics.rMd)
  Box(
    Modifier
      .fillMaxWidth()
      .height(Metrics.agentImage)
      .clip(shape)
      .background(Palette.chip)
      .border(Metrics.hairline, Palette.line, shape)
      .clickable(enabled = !busy) { open() }
      .semantics { contentDescription = name }
      .testTag("media.video"),
    contentAlignment = Alignment.Center,
  ) {
    val frame = poster
    if (frame != null) {
      Image(frame, null, Modifier.fillMaxSize(), contentScale = ContentScale.Fit)
      PlayDisc(busy)
    } else {
      Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(Metrics.gap)) {
        if (busy) DownloadRing(transfer?.fraction) else Lucide("play", Palette.fg3, size = Metrics.iconCtl)
        Text(
          transfer?.let { downloadLine(context, it) } ?: failure ?: name,
          style = WebType.text3.copy(fontFeatureSettings = TABULAR), color = Palette.fg3, maxLines = 2, textAlign = TextAlign.Center,
          overflow = TextOverflow.Ellipsis, modifier = Modifier.padding(horizontal = Metrics.pad),
        )
      }
    }
  }
  playing?.let { VideoPlayer(it, name) { playing = null } }
}

/** The play mark on its own disc, which reads on any frame; a spinner in its place while busy */
@Composable
private fun PlayDisc(busy: Boolean) {
  Box(Modifier.size(WebMetrics.ctlSm).background(Palette.scrim, CircleShape), contentAlignment = Alignment.Center) {
    if (busy) CircularProgressIndicator(color = Color.White, strokeWidth = Metrics.s0_5, modifier = Modifier.size(WebMetrics.icon))
    else Lucide("play", Color.White, size = WebMetrics.icon)
  }
}

/**
 * A video attachment (VideoAttachmentTile): its first frame under a play mark, the size of a photo's tile. A tap opens
 * the video, fetched first if this phone has no copy yet. A staged video still compressing spins in place of the mark
 * and opens nothing
 */
@Composable
fun VideoAttachmentTile(
  source: VideoSource,
  name: String,
  side: Dp = Metrics.thumb,
  /** A staged video's compression, 0…1 (VideoJobs); null once the file is ready */
  progress: Float? = null,
  /** The frame to show before the file itself can be read (the original's, while it compresses) */
  still: ImageBitmap? = null,
  /** The tile's corner: a sent one's, or the composer strip's */
  corner: Dp = Metrics.rSm,
  onOpen: (File) -> Unit,
) {
  val media = LocalMedia.current
  val context = LocalContext.current
  val scope = rememberCoroutineScope()
  val pixels = with(LocalDensity.current) { side.roundToPx() }
  var poster by remember(source) { mutableStateOf<ImageBitmap?>(null) }
  var opening by remember { mutableStateOf(false) }
  val transfer = VideoCache.transfers[source]
  val busy = opening || progress != null || transfer != null
  // A staged copy is read only once it is complete
  LaunchedEffect(source, progress == null) {
    if (progress != null) return@LaunchedEffect
    poster = runCatching { VideoCache.poster(context, source, media.loader, pixels) }.getOrNull()
  }
  val shape = RoundedCornerShape(corner)
  Box(
    Modifier
      .size(side)
      .clip(shape)
      .background(Palette.chip)
      .clickable(enabled = !busy) {
        opening = true
        scope.launch {
          try {
            onOpen(VideoCache.file(context, source, media.loader))
          } catch (e: Exception) {
            media.model?.showToast(VideoCache.message(e))
          } finally {
            opening = false
          }
        }
      }
      .semantics {
        contentDescription = name
        progress?.let { stateDescription = t("mobile.video.compressing", "percent" to (it * 100).toInt()) }
      }
      .testTag("media.videoTile"),
    contentAlignment = Alignment.Center,
  ) {
    val frame = poster ?: still ?: VideoCache.cached(source)
    when {
      frame != null -> {
        Image(frame, null, Modifier.fillMaxSize(), contentScale = ContentScale.Crop)
        PlayDisc(busy)
      }
      busy -> DownloadRing(transfer?.fraction, Metrics.icon)
      else -> Lucide("fileVideo", Palette.fg3, size = Metrics.icon)
    }
  }
}

/**
 * A download's progress (DownloadProgress.swift DownloadRing): an arc on a faint track that fills as the bytes arrive,
 * a spinner until the computer has said how large the file is
 */
@Composable
fun DownloadRing(fraction: Float?, side: Dp = Metrics.iconCtl, tint: Color = Palette.fg3) {
  if (fraction == null) {
    CircularProgressIndicator(color = tint, strokeWidth = side / RING_PARTS, modifier = Modifier.size(side))
    return
  }
  Canvas(Modifier.size(side).semantics { contentDescription = t("mobile.download.progress", "percent" to (fraction * 100).toInt()) }) {
    val stroke = size.minDimension / RING_PARTS
    val inset = stroke / 2
    val arc = Size(size.width - stroke, size.height - stroke)
    drawArc(tint, 0f, FULL_TURN, false, Offset(inset, inset), arc, alpha = RING_TRACK, style = Stroke(stroke))
    drawArc(tint, -QUARTER_TURN, FULL_TURN * fraction, false, Offset(inset, inset), arc, style = Stroke(stroke, cap = StrokeCap.Round))
  }
}

/** The bytes so far against the file's size, in the system's file units; null until the size is known */
fun downloadLine(context: Context, transfer: VideoCache.Transfer): String? {
  if (transfer.total <= 0) return null
  return "${Formatter.formatShortFileSize(context, transfer.have)} / ${Formatter.formatShortFileSize(context, transfer.total)}"
}

/**
 * The phone's player over everything (iOS opens Quick Look): the platform's VideoView with its own controls, over black,
 * playing at once; ✕ or back closes it
 */
@Composable
fun VideoPlayer(file: File, name: String, onClose: () -> Unit) {
  Dialog(onDismissRequest = onClose, properties = DialogProperties(usePlatformDefaultWidth = false, decorFitsSystemWindows = false)) {
    val window = (LocalView.current.parent as? DialogWindowProvider)?.window
    val view = LocalView.current
    SideEffect { window?.let { WindowCompat.getInsetsController(it, view).isAppearanceLightStatusBars = false } }
    CompositionLocalProvider(LocalDark provides true) {
      Box(Modifier.fillMaxSize().background(Color.Black).testTag("media.player")) {
        AndroidView(
          factory = { ctx ->
            FrameLayout(ctx).apply {
              val video = VideoView(ctx)
              addView(video, FrameLayout.LayoutParams(FrameLayout.LayoutParams.WRAP_CONTENT, FrameLayout.LayoutParams.WRAP_CONTENT, android.view.Gravity.CENTER))
              val controls = MediaController(ctx)
              controls.setAnchorView(video)
              video.setMediaController(controls)
              video.setVideoPath(file.path)
              video.setOnPreparedListener { video.start(); controls.show() }
            }
          },
          onRelease = { frame -> (frame.getChildAt(0) as? VideoView)?.stopPlayback() },
          modifier = Modifier.fillMaxSize(),
        )
        Row(Modifier.fillMaxWidth().statusBarsPadding().padding(horizontal = Metrics.s1).height(Metrics.barCtl + Metrics.s2), verticalAlignment = Alignment.CenterVertically) {
          IconButton(onClick = onClose, modifier = Modifier.testTag("media.player.close")) {
            Lucide("x", Palette.fg1, size = Metrics.iconCtl, contentDescription = t("common.close"))
          }
          Text(
            name, color = Palette.fg1, fontSize = Type.text1, fontWeight = FontWeight.SemiBold, maxLines = 1,
            overflow = TextOverflow.MiddleEllipsis, textAlign = TextAlign.Center, modifier = Modifier.weight(1f).padding(end = Metrics.barCtl),
          )
        }
      }
    }
  }
}

/** The ring's stroke and inset are an eighth of its box, the track a quarter of the ink (ContextUsage.swift) */
private const val RING_PARTS = 8
private const val RING_TRACK = 0.25f
private const val FULL_TURN = 360f
private const val QUARTER_TURN = 90f
