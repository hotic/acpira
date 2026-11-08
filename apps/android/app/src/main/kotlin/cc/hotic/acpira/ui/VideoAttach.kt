package cc.hotic.acpira.ui

import android.content.ClipboardManager
import android.content.Context
import android.media.MediaMetadataRetriever
import android.net.Uri
import android.provider.OpenableColumns
import android.util.Base64
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.platform.LocalContext
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.compose.ui.platform.LocalLifecycleOwner
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.model.Draft
import cc.hotic.acpira.store.VideoCompressor
import java.io.File
import java.util.UUID
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

// Videos picked from the photo picker (apps/ios Chat/VideoAttach.swift). The tile is staged at once under the path its
// compressed copy will have, and the copy is made in the background (VideoCompressor); until it is on disk the tile
// spins and send waits. Once it is, the staged draft carries the copy's bytes, as a picked file's does.

/** The videos still being prepared, by the staged copy's file URI */
object VideoJobs {
  /** 0…1 per video still being prepared */
  val progress = mutableStateMapOf<String, Float>()
  /** The original's first frame, for the tile to show while the copy is made */
  val stills = mutableStateMapOf<String, ImageBitmap>()
  private val jobs = HashMap<String, Job>()
  private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main)

  /** A staged video is still compressing: the draft cannot go yet */
  fun pending(staged: List<Staged>): Boolean = staged.any { (it.draft as? Draft.File)?.uri in progress }

  /**
   * Stages a picked video at once and prepares its copy: [stage] adds the pending tile, [replace] swaps it for the
   * ready one (null drops it, when the copy failed; then [failed] says why)
   */
  fun start(
    context: Context,
    source: Uri,
    stage: (Staged) -> Unit,
    replace: (uri: String, Staged?) -> Unit,
    failed: (String) -> Unit,
  ) {
    val app = context.applicationContext
    val out = File(File(AttachmentFiles.folder(app), UUID.randomUUID().toString()), "video.mp4")
    val uri = Uri.fromFile(out).toString()
    stage(Staged(Draft.File(uri, out.name, null), out.name))
    progress[uri] = 0f
    jobs[uri] = scope.launch {
      try {
        val name = withContext(Dispatchers.IO) { displayName(app, source) }?.substringBeforeLast('.')?.plus(".mp4") ?: out.name
        val still = withContext(Dispatchers.IO) { firstFrame(app, source) }
        if (still != null && uri in progress) stills[uri] = still
        VideoCompressor.compress(app, source, out, VideoCompressor.MAX_BYTES) { fraction ->
          scope.launch { if (uri in progress) progress[uri] = fraction }
        }
        val data = withContext(Dispatchers.IO) { Base64.encodeToString(out.readBytes(), Base64.NO_WRAP) }
        // The ready tile reads its poster from the cache: the same frame, without decoding the copy again
        still?.let { VideoCache.remember(it, VideoSource.Local(out)) }
        replace(uri, Staged(Draft.File(uri, name, data), name))
      } catch (e: CancellationException) {
        out.parentFile?.deleteRecursively()
      } catch (e: Exception) {
        out.parentFile?.deleteRecursively()
        replace(uri, null)
        failed(
          if (e is VideoCompressor.Failure.TooBig) t("mobile.video.tooBig", "mb" to (VideoCompressor.MAX_BYTES shr 20))
          else t("mobile.video.failed"),
        )
      } finally {
        progress.remove(uri)
        stills.remove(uri)
        jobs.remove(uri)
      }
    }
  }

  /** The tile was removed: its preparation stops and its copy goes */
  fun cancel(uri: String) {
    jobs[uri]?.cancel()
  }

  private fun displayName(context: Context, uri: Uri): String? =
    context.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { c ->
      if (c.moveToFirst()) c.getString(0) else null
    }

  /** The original's first frame at a staged tile's size on a dense screen, null when it cannot be read */
  private fun firstFrame(context: Context, uri: Uri): ImageBitmap? {
    val retriever = MediaMetadataRetriever()
    return try {
      retriever.setDataSource(context, uri)
      val side = (context.resources.displayMetrics.density * Metrics.stagedTile.value * STILL_SCALE).toInt()
      retriever.getScaledFrameAtTime(0, MediaMetadataRetriever.OPTION_CLOSEST_SYNC, side, side)?.asImageBitmap()
    } catch (_: Exception) {
      null
    } finally {
      retriever.release()
    }
  }

  /** The still is cut a little over the staged tile, which crops it */
  private const val STILL_SCALE = 1.5f
}

/** The clipboard's image, if it holds one: the first item's content URI */
fun clipboardImage(context: Context): Uri? {
  val clipboard = context.getSystemService(ClipboardManager::class.java) ?: return null
  if (clipboard.primaryClipDescription?.hasMimeType("image/*") != true) return null
  return clipboard.primaryClip?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.uri
}

/**
 * Whether the clipboard holds an image, checked from its description without reading it (PasteWatch): on entering, on
 * a change made in the app, and on coming back from another app. [forced] (`popup attach.paste`) offers it regardless
 */
@Composable
fun rememberPasteReady(forced: Boolean = false): Boolean {
  val context = LocalContext.current
  val lifecycle = LocalLifecycleOwner.current.lifecycle
  var ready by remember { mutableStateOf(false) }
  DisposableEffect(context, lifecycle) {
    val clipboard = context.getSystemService(ClipboardManager::class.java)
    fun check() {
      ready = forced || clipboard?.primaryClipDescription?.hasMimeType("image/*") == true
    }
    val listener = ClipboardManager.OnPrimaryClipChangedListener { check() }
    val observer = LifecycleEventObserver { _, event -> if (event == Lifecycle.Event.ON_RESUME) check() }
    clipboard?.addPrimaryClipChangedListener(listener)
    lifecycle.addObserver(observer)
    check()
    onDispose {
      clipboard?.removePrimaryClipChangedListener(listener)
      lifecycle.removeObserver(observer)
    }
  }
  return ready
}
