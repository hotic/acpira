package cc.hotic.acpira.ui

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.webkit.MimeTypeMap
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.core.content.FileProvider
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.design.WebType
import cc.hotic.acpira.design.readableWidth
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.model.ProjectError
import cc.hotic.acpira.store.AppModel
import cc.hotic.acpira.store.MediaLoader
import cc.hotic.acpira.store.StagedFile
import cc.hotic.acpira.store.imageMimeOf
import cc.hotic.acpira.store.isVideoName
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File

// apps/ios Files/FileDownload.swift: a file the phone cannot show as text (a video, a picture, a binary, one over the
// text limit). The engine stages it into the session's blob store (`stageFile`), the phone downloads it with its progress
// (VideoCache, resumable over a paired link) and opens it: a picture in the Lightbox, a video in the phone's player,
// anything else in the app the system picks for it (iOS hands every kind to Quick Look).

/** The largest file the engine stages for the phone (limits.rs `MAX_STAGED_FILE_BYTES`, the link's blob limit), in MB */
const val STAGED_FILE_MAX_MB = 64

/** Opened by the page straight away, without first asking for the text: what the phone plays or zooms */
fun opensDirectly(path: String): Boolean {
  val name = path.substringAfterLast('/')
  return isVideoName(name) || imageMimeOf(name) != null
}

/**
 * A project file's download page: the file's name on the bar and [FileDownload] under it. A file page that shows text
 * embeds [FileDownload] itself for what it cannot show
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun DownloadScreen(model: AppModel, sessionId: String, path: String) {
  Scaffold(
    containerColor = Palette.bg0,
    topBar = {
      TopAppBar(
        title = {
          Text(path.substringAfterLast('/'), maxLines = 1, overflow = TextOverflow.MiddleEllipsis, fontSize = Type.text1, fontWeight = FontWeight.SemiBold)
        },
        navigationIcon = { LeadButton("download.back") { model.pop() } },
        colors = TopAppBarDefaults.topAppBarColors(containerColor = Palette.bg0, scrolledContainerColor = Palette.bg0),
      )
    },
  ) { padding ->
    MediaLayer(sessionId, model) {
      Box(Modifier.fillMaxSize().padding(padding).navigationBarsPadding().readableWidth()) {
        FileDownload(model, sessionId, path, reason = null)
      }
    }
  }
}

/**
 * The download itself: the file's glyph, why there is no text ([reason]; null for a video or picture, which downloads as
 * the page opens), and the button that downloads and opens it; while it comes over, a ring and the bytes so far. A
 * download that stopped part way resumes on the next try, and a file already on the phone opens again without one
 */
@Composable
fun FileDownload(model: AppModel, sessionId: String, path: String, reason: String?) {
  val context = LocalContext.current
  val scope = rememberCoroutineScope()
  val media = LocalMedia.current
  val name = path.substringAfterLast('/')
  /** The staged blob once the engine answered */
  var source by remember(path) { mutableStateOf<VideoSource?>(null) }
  var staging by remember(path) { mutableStateOf(false) }
  var failure by remember(path) { mutableStateOf<String?>(null) }
  /** The download stopped part way, so the next try resumes it */
  var stopped by remember(path) { mutableStateOf(false) }
  /** The file on the phone once it came over: opening it again needs no download */
  var file by remember(path) { mutableStateOf<File?>(null) }
  var playing by remember { mutableStateOf<File?>(null) }
  val transfer = source?.let { VideoCache.transfers[it] }
  val icon = when {
    isVideoName(name) -> "fileVideo"
    imageMimeOf(name) != null -> "image"
    else -> "file"
  }

  fun open(f: File) {
    when {
      isVideoName(name) -> playing = f
      imageMimeOf(name) != null -> scope.launch {
        val image = withContext(Dispatchers.Default) { MediaLoader.decodeNow(f.readBytes()) }
        if (image != null && media.lightbox != null) media.lightbox.open(LightboxItem(image, name, null)) else viewExternally(context, f, name, model)
      }
      else -> viewExternally(context, f, name, model)
    }
  }

  fun start() {
    file?.let { open(it); return }
    if (staging || transfer != null) return
    failure = null
    stopped = false
    staging = true
    scope.launch {
      // Staged again on every try: the file may have changed, and the engine answers an unchanged one from its index
      when (val staged = model.media.stageFile(sessionId, path)) {
        is StagedFile.Failed -> {
          failure = message(staged.error)
          staging = false
        }
        is StagedFile.Staged -> {
          val s = VideoSource.Blob(sessionId, staged.blob)
          source = s
          staging = false
          try {
            val got = named(context, VideoCache.file(context, s, model.media.loader), name)
            file = got
            open(got)
          } catch (e: Exception) {
            failure = VideoCache.message(e)
            stopped = e is VideoCache.Failure.Interrupted
          }
        }
      }
    }
  }

  LaunchedEffect(path) { if (reason == null) start() }
  Column(
    Modifier.fillMaxSize().padding(Metrics.pad).testTag("download"),
    horizontalAlignment = Alignment.CenterHorizontally,
    verticalArrangement = Arrangement.spacedBy(Metrics.gap, Alignment.CenterVertically),
  ) {
    if (staging || transfer != null) {
      DownloadRing(transfer?.fraction)
      Text(transfer?.let { downloadLine(context, it) } ?: " ", style = WebType.text3.copy(fontFeatureSettings = TABULAR), color = Palette.fg3)
    } else {
      Lucide(icon, Palette.fg3, size = Metrics.iconCtl)
      (failure ?: reason)?.let { Text(it, style = WebType.text3, color = Palette.fg3, textAlign = TextAlign.Center) }
      Button(
        onClick = ::start,
        colors = ButtonDefaults.buttonColors(containerColor = Palette.btn1Bg, contentColor = Palette.btn1Fg),
        modifier = Modifier.padding(top = Metrics.gapHalf).testTag("download.start"),
      ) {
        Text(t(if (file != null) "common.open" else if (stopped) "mobile.download.resume" else "mobile.download.open"), fontSize = Type.text3)
      }
    }
  }
  playing?.let { VideoPlayer(it, name) { playing = null } }
}

private fun message(error: ProjectError): String = when (error) {
  ProjectError.TooLarge -> t("mobile.download.tooLarge", "mb" to STAGED_FILE_MAX_MB)
  ProjectError.Missing, ProjectError.Outside, ProjectError.Exists, ProjectError.Binary -> t("mobile.download.unreadable")
  ProjectError.NoProject -> t("panel.error.other", "error" to "no project")
  is ProjectError.Message -> error.text
}

/**
 * The downloaded blob under the file's own name, which the opening app shows as its title: a hard link (a copy where
 * that fails) in a folder named by the blob, whose name is a hash of the bytes, so one link serves every later open
 */
private suspend fun named(context: Context, file: File, name: String): File = withContext(Dispatchers.IO) {
  val dir = File(File(context.cacheDir, "open"), file.nameWithoutExtension)
  val target = File(dir, name)
  if (target.exists()) return@withContext target
  runCatching {
    dir.mkdirs()
    runCatching { android.system.Os.link(file.path, target.path) }.getOrElse { file.copyTo(target, overwrite = true) }
    target
  }.getOrDefault(file)
}

/** Hands the file to the app the system picks for its type; a toast when there is none */
private fun viewExternally(context: Context, file: File, name: String, model: AppModel) {
  val ext = name.substringAfterLast('.', "").lowercase()
  val mime = MimeTypeMap.getSingleton().getMimeTypeFromExtension(ext) ?: "application/octet-stream"
  val uri = FileProvider.getUriForFile(context, "${context.packageName}.files", file)
  val view = Intent(Intent.ACTION_VIEW).setDataAndType(uri, mime).addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
  try {
    context.startActivity(Intent.createChooser(view, name).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
  } catch (_: ActivityNotFoundException) {
    model.showToast(t("mobile.download.unreadable"))
  }
}
