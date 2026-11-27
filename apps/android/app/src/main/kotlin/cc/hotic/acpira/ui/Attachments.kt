package cc.hotic.acpira.ui

import android.Manifest
import android.content.Context
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.graphics.ImageDecoder
import android.net.Uri
import android.provider.OpenableColumns
import android.util.Base64
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.text.style.TextOverflow
import androidx.core.content.ContextCompat
import androidx.core.content.FileProvider
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.model.Attachment
import cc.hotic.acpira.model.Draft
import cc.hotic.acpira.store.MediaSource
import java.io.ByteArrayOutputStream
import java.io.File
import kotlin.math.max
import kotlin.math.min
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** Where an attach bean sends the reader: the photo picker, the camera, the file picker */
private enum class AttachSource { Photos, Camera, Files, Paste }

/** The source a bean asked for, picked up by [AttachPickers] */
private val pendingSource = mutableStateOf<AttachSource?>(null)

/** Attach as a liquid chip: the photo, camera and file sources squeeze out of `+`, each named beside it */
@Composable
internal fun attachBeans(staged: List<Staged>, onStaged: (List<Staged>) -> Unit, goo: Goo, onToast: (String) -> Unit, paste: Boolean): List<Bean> {
  fun source(key: String, icon: String, label: String, s: AttachSource) = Bean(
    key, label, label,
    glyph = { Lucide(icon, Palette.fg1, size = Metrics.barIcon) },
    tap = { goo.close(); pendingSource.value = s },
  )
  return listOf(
    source("photos", "image", t("mobile.attach.photos"), AttachSource.Photos),
    source("camera", "camera", t("mobile.attach.camera"), AttachSource.Camera),
    source("files", "paperclip", t("mobile.attach.files"), AttachSource.Files),
  ) + listOfNotNull(
    // While the clipboard holds an image, `+` offers to paste it (Goo.swift sources)
    if (paste) source("paste", "clipboardPaste", t("desktop.menu.paste"), AttachSource.Paste) else null,
  )
}

/**
 * The system pickers behind the attach beans: photos through the photo picker (no permission), the camera into a
 * file this app shares with it, any file through the document picker. Images are cut to [IMAGE_SIDE] px as JPEG
 * (AttachmentImage.thumbnail); files travel whole, inside the one message, so the draft stays under [MESSAGE_BYTES]
 */
@Composable
internal fun AttachPickers(goo: Goo, onStaged: (List<Staged>) -> Unit, staged: List<Staged>, onToast: (String) -> Unit, context: Context) {
  val scope = rememberCoroutineScope()
  val current by androidx.compose.runtime.rememberUpdatedState(staged)
  val add: (List<Staged>) -> Unit = { more ->
    val next = current + more
    if (next.sumOf { draftBytes(it.draft) } > MESSAGE_BYTES) onToast(t("mobile.video.tooBig", "mb" to MESSAGE_BYTES / 4 * 3 / (1 shl 20)))
    else onStaged(next)
  }
  // A picked video swaps its pending tile for the ready one, or drops it when its copy failed or is too large to go
  val replace: (String, Staged?) -> Unit = { uri, ready ->
    val at = current.indexOfFirst { (it.draft as? Draft.File)?.uri == uri }
    if (at >= 0) {
      val without = current.filterIndexed { j, _ -> j != at }
      val next = if (ready == null) without else current.toMutableList().also { it[at] = ready }
      if (ready != null && next.sumOf { draftBytes(it.draft) } > MESSAGE_BYTES) {
        onToast(t("mobile.video.tooBig", "mb" to MESSAGE_BYTES / 4 * 3 / (1 shl 20)))
        onStaged(without)
      } else onStaged(next)
    }
  }
  // Videos take their tile at once and fill it as they compress; photos arrive as they load (VideoAttach.swift)
  val photos = rememberLauncherForActivityResult(ActivityResultContracts.PickMultipleVisualMedia(MAX_PHOTOS)) { uris ->
    val (videos, images) = uris.partition { context.contentResolver.getType(it)?.startsWith("video/") == true }
    val pending = ArrayList<Staged>()
    for (v in videos) VideoJobs.start(context, v, stage = { pending += it }, replace = replace, failed = onToast)
    if (pending.isNotEmpty()) onStaged(current + pending)
    if (images.isNotEmpty()) scope.launch { add(images.mapNotNull { stageImage(context, it) }) }
  }
  var shot by remember { mutableStateOf<Uri?>(null) }
  val camera = rememberLauncherForActivityResult(ActivityResultContracts.TakePicture()) { ok ->
    val uri = shot
    if (ok && uri != null) scope.launch { stageImage(context, uri)?.let { add(listOf(it)) } }
  }
  val takePicture = {
    val dir = File(context.cacheDir, "camera").apply { mkdirs() }
    val file = File(dir, "photo-${System.currentTimeMillis()}.jpg")
    val uri = FileProvider.getUriForFile(context, "${context.packageName}.files", file)
    shot = uri
    camera.launch(uri)
  }
  // The app declares CAMERA (pairing's scanner), so the system camera only opens once it is granted
  val cameraGrant = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
    if (granted) takePicture() else onToast(t("mobile.pair.camera.denied"))
  }
  val files = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { uris ->
    scope.launch { add(uris.mapNotNull { stageFile(context, it) }) }
  }
  val source by pendingSource
  LaunchedEffect(source) {
    when (source ?: return@LaunchedEffect) {
      AttachSource.Photos -> photos.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageAndVideo))
      AttachSource.Camera -> {
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) takePicture()
        else cameraGrant.launch(Manifest.permission.CAMERA)
      }
      AttachSource.Files -> files.launch(arrayOf("*/*"))
      // The clipboard's image, staged as a picked photo is
      AttachSource.Paste -> clipboardImage(context)?.let { uri -> scope.launch { stageImage(context, uri)?.let { add(listOf(it)) } } }
    }
    pendingSource.value = null
  }
}

private fun draftBytes(d: Draft): Int = when (d) {
  is Draft.Image -> d.data.length
  is Draft.Text -> d.text.length
  is Draft.File -> d.data?.length ?: 0
  is Draft.Quote -> d.text.length
}

private fun displayName(context: Context, uri: Uri): String? =
  context.contentResolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { c ->
    if (c.moveToFirst()) c.getString(0) else null
  }

/** A picked photo as the composer sends it: the longest side cut to [IMAGE_SIDE] px, JPEG, base64 */
private suspend fun stageImage(context: Context, uri: Uri): Staged? = withContext(Dispatchers.IO) {
  runCatching {
    val bitmap = ImageDecoder.decodeBitmap(ImageDecoder.createSource(context.contentResolver, uri)) { decoder, info, _ ->
      val scale = min(1f, IMAGE_SIDE.toFloat() / max(info.size.width, info.size.height))
      decoder.setTargetSize(max(1, (info.size.width * scale).toInt()), max(1, (info.size.height * scale).toInt()))
      decoder.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
    }
    val bytes = ByteArrayOutputStream().also { bitmap.compress(Bitmap.CompressFormat.JPEG, JPEG_QUALITY, it) }.toByteArray()
    val name = displayName(context, uri) ?: "image.jpg"
    Staged(Draft.Image("image/jpeg", Base64.encodeToString(bytes, Base64.NO_WRAP), name), name, bitmap.asImageBitmap())
  }.getOrNull()
}

/** A picked file with its bytes: the engine stages it on the computer and links the staged path */
private suspend fun stageFile(context: Context, uri: Uri): Staged? = withContext(Dispatchers.IO) {
  runCatching {
    val bytes = context.contentResolver.openInputStream(uri)?.use { it.readBytes() } ?: return@runCatching null
    val name = displayName(context, uri) ?: uri.lastPathSegment ?: "file"
    Staged(Draft.File(uri.toString(), name, Base64.encodeToString(bytes, Base64.NO_WRAP)), name)
  }.getOrNull()
}

/**
 * The staged attachments inside the card over the text (Composer.swift StagedAttachments): image tiles, video tiles and
 * file chips, each with its remove mark. A tile opens its picture in the Lightbox, a video plays, a pasted text shows
 * whole, a file picked on the phone opens in the phone's viewer and a file on the computer in the chat's file page.
 * [onNavigate] runs before a picture or a file opens (the whole-screen editor gets out of the way); a video and a
 * pasted text open in dialogs over it
 */
@Composable
internal fun StagedStrip(staged: List<Staged>, onStaged: (List<Staged>) -> Unit, modifier: Modifier, onNavigate: (() -> Unit)? = null) {
  val context = LocalContext.current
  val media = LocalMedia.current
  var playing by remember { mutableStateOf<Pair<File, String>?>(null) }
  var reading by remember { mutableStateOf<Staged?>(null) }
  playing?.let { (file, name) -> VideoPlayer(file, name) { playing = null } }
  reading?.let { s -> StagedText(s) { reading = null } }
  val opener = rememberFileOpener(onNavigate)
  Row(
    modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(end = Metrics.s3),
    horizontalArrangement = Arrangement.spacedBy(Metrics.s2),
  ) {
    // Quotes from the transcript lead as one annotations chip (ui/Quotes.kt)
    QuoteChip(staged, onStaged)
    staged.forEachIndexed { i, s ->
      if (s.draft is Draft.Quote) return@forEachIndexed
      Box {
        val thumb = s.thumb
        val tile = Modifier.size(Metrics.stagedTile).clip(RoundedCornerShape(Metrics.rLg)).border(Metrics.hairline, Palette.line, RoundedCornerShape(Metrics.rLg))
        // A sent prompt's own photo, taken in for editing, still lives in the engine's store
        val stored = (s.source as? Attachment.Image)?.let { MediaSource.stored(it.blob, media.session) }
        // A video is a tile like a photo's from the moment it is picked: while it compresses, the original's first frame
        // waits under a spinner, one view throughout (Composer.swift staged strip)
        val file = s.draft as? Draft.File
        val progress = file?.let { VideoJobs.progress[it.uri] }
        val video = file?.takeIf { s.source !is Attachment.Selection }?.let {
          if (progress != null) Uri.parse(it.uri).path?.let { path -> VideoSource.Local(File(path)) }
          else VideoSource.of(context, it.uri, s.name, media.session, media.loader)
        }
        if (video != null) {
          // The player is a dialog of its own over the editor, so the editor stays (iOS leaves only for the file page):
          // collapsing it first took this strip, and the player with it, out of composition
          VideoAttachmentTile(video, s.name, side = Metrics.stagedTile, progress = progress, still = VideoJobs.stills[file.uri], corner = Metrics.rLg) {
            playing = it to s.name
          }
        } else if (thumb != null) {
          val image = s.draft as? Draft.Image
          Image(
            thumb, s.name, contentScale = ContentScale.Crop,
            modifier = tile.clickable {
              // The photo's own pixels, decoded once: the Lightbox shows it alone, as it is not in the session yet
              val source = image?.let { MediaSource.Inline(it.data) }
              val pixels = source?.let { media.loader.peek(it) } ?: return@clickable
              onNavigate?.invoke()
              media.lightbox?.open(LightboxItem(pixels, s.name, source))
            }.testTag("composer.staged.image"),
          )
        } else if (stored != null) {
          val glyph = @Composable {
            Box(Modifier.size(Metrics.stagedTile).background(Palette.chip, RoundedCornerShape(Metrics.rLg)), contentAlignment = Alignment.Center) {
              Lucide("image", Palette.fg3, size = Metrics.icon)
            }
          }
          MediaImage(
            stored,
            content = { pixels ->
              Picture(
                pixels,
                tile.clickable { onNavigate?.invoke(); media.lightbox?.open(LightboxItem(pixels, s.name, stored)) }.testTag("composer.staged.image"),
                contentScale = ContentScale.Crop,
                description = t("common.previewImage", "name" to s.name.ifEmpty { t("common.image") }),
              )
            },
            placeholder = glyph,
            failure = glyph,
          )
        } else {
          val open: (() -> Unit)? = when {
            s.draft is Draft.Text || s.source is Attachment.Text -> ({ reading = s })
            s.source is Attachment.Selection -> (s.source as Attachment.Selection).let { sel -> opener(sel.uri, sel.startLine) }
            file != null -> opener(file.uri, null)
            else -> null
          }
          Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(Metrics.s1_5),
            modifier = Modifier.height(Metrics.stagedTile).widthIn(max = Metrics.stagedFileW)
              .background(Palette.chip, RoundedCornerShape(Metrics.rLg)).clip(RoundedCornerShape(Metrics.rLg))
              .then(if (open != null) Modifier.clickable(onClick = open) else Modifier)
              .padding(horizontal = Metrics.s3).testTag("composer.staged.file"),
          ) {
            Lucide(if (s.source is Attachment.Selection) "textSelect" else "fileText", Palette.fg2, size = Metrics.icon)
            Text(stagedLabel(s), color = Palette.fg1, fontSize = Type.text3, maxLines = 1, overflow = TextOverflow.MiddleEllipsis)
          }
        }
        Box(
          Modifier.align(Alignment.TopEnd).padding(Metrics.s1).size(Metrics.stagedRemove)
            .background(Palette.scrim, CircleShape).clip(CircleShape).clickable {
              (s.draft as? Draft.File)?.let { VideoJobs.cancel(it.uri) }
              onStaged(staged.filterIndexed { j, _ -> j != i })
            },
          contentAlignment = Alignment.Center,
        ) { Lucide("x", Palette.fgStrong, size = Metrics.chevron, contentDescription = t("common.remove")) }
      }
    }
  }
}

/** A chip's name: a selection taken back from a sent prompt names its lines, as the transcript's pill does */
private fun stagedLabel(s: Staged): String = when (val a = s.source) {
  is Attachment.Selection -> "${a.name} ${if (a.startLine == a.endLine) "L${a.startLine}" else "L${a.startLine}-${a.endLine}"}"
  else -> s.name
}

/** A staged text's whole contents: the draft's own text, or a sent prompt's stored text read back */
@Composable
private fun StagedText(s: Staged, onDismiss: () -> Unit) {
  val draft = s.draft as? Draft.Text
  val source = s.source as? Attachment.Text
  TextAttachmentSheet(s.name, text = draft?.takeIf { source == null }?.text, blob = source?.blob, onDismiss = onDismiss)
}

private const val MAX_PHOTOS = 4
/** AttachmentImage.thumbnail: the longest side and the JPEG quality */
private const val IMAGE_SIDE = 1024
private const val JPEG_QUALITY = 80
/** AttachmentFile.messageBytes: one message on the paired link holds at most this much base64 */
private const val MESSAGE_BYTES = 60 * (1 shl 20)
