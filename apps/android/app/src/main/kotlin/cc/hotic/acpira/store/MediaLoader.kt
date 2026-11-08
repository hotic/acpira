package cc.hotic.acpira.store

import android.content.Context
import android.graphics.Bitmap
import android.graphics.ImageDecoder
import android.util.Base64
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.asImageBitmap
import cc.hotic.acpira.model.AgentTurn
import cc.hotic.acpira.model.Attachment
import cc.hotic.acpira.model.ImageBlock
import cc.hotic.acpira.model.ImageRef
import cc.hotic.acpira.model.SessionView
import cc.hotic.acpira.model.SteerBlock
import cc.hotic.acpira.model.TextBlock
import cc.hotic.acpira.model.ToolCallBlock
import cc.hotic.acpira.model.ToolContent
import cc.hotic.acpira.model.UserTurn
import cc.hotic.acpira.sync.BlobFetch
import cc.hotic.acpira.sync.BlobNames
import cc.hotic.acpira.sync.HttpBlobs
import cc.hotic.acpira.sync.MediaCache
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File
import java.io.IOException
import java.net.URI
import java.net.URLDecoder
import java.nio.ByteBuffer
import kotlin.math.max

// Where a transcript's images come from and the loader the image views ask (apps/ios Media/MediaLoader.swift; the
// webview's AgentImage.tsx, Attachments.tsx and Prose.tsx load blob URLs themselves, the phone fetches through the store).

/** Where an image's pixels are */
sealed interface MediaSource {
  /** A file in the session's blob store, fetched through the store's connection */
  data class Blob(val session: String, val name: String) : MediaSource
  /** A file on the engine's machine that a Markdown image or a file row names: the engine stages it first (`imageBlob`) */
  data class HostFile(val session: String, val path: String) : MediaSource
  /** A web image, loaded straight from the network */
  data class Web(val url: String) : MediaSource
  /** Base64 pixels the transcript carries itself: photos the phone staged, fixtures */
  data class Inline(val base64: String) : MediaSource

  companion object {
    /**
     * An attachment's or an agent image's `blob`: a blob-store name, or the base64 of a photo the phone staged before
     * the engine stored it. A blob name always has an extension (transcript_store.rs blob_name); base64 never holds a dot
     */
    fun stored(blob: String?, session: String?): MediaSource? {
      if (blob.isNullOrEmpty()) return null
      if ('.' !in blob) return Inline(blob)
      return session?.let { Blob(it, blob) }
    }

    /**
     * An agent image: its stored pixels, else a `data:` URI it carries, else the file its `uri` names, staged on demand
     * (a row of a session on an SSH host keeps the host path alone). null: nothing loadable, which the card names instead
     */
    fun agentImage(blob: String?, uri: String?, session: String?): MediaSource? =
      stored(blob, session) ?: uri?.let(::dataUri) ?: uri?.let { hostFile(it, session) }

    /** A file the engine can read for the page: a file: URL or an absolute path (stagedBlob.ts isFileSource) */
    fun isFilePath(uri: String): Boolean {
      val lower = uri.lowercase()
      // `//host/…` is a protocol-relative web URL, not a path
      return lower.startsWith("file:") || (uri.startsWith("/") && !uri.startsWith("//")) || uri.startsWith("\\\\") ||
        (uri.length > 2 && uri[1] == ':' && (uri[2] == '/' || uri[2] == '\\'))
    }

    fun hostFile(uri: String, session: String?): MediaSource? =
      if (isFilePath(uri)) session?.let { HostFile(it, uri) } else null

    /** The base64 of a `data:…;base64,` URI */
    fun dataUri(uri: String): MediaSource? {
      if (!uri.lowercase().startsWith("data:")) return null
      val comma = uri.indexOf(',')
      if (comma < 0 || !uri.substring(0, comma).lowercase().endsWith(";base64")) return null
      return Inline(uri.substring(comma + 1))
    }

    /**
     * A Markdown image's source (Prose.tsx MarkdownImage): web URLs load directly and `data:` URIs decode in place;
     * anything else is a file on the engine's machine, the blob the engine read with the text (`TextBlock.images`,
     * matched on the source as written, both sides percent-decoded), else staged on demand. null: nothing loadable
     */
    fun markdown(src: String, images: List<ImageRef>?, session: String?): MediaSource? {
      val lower = src.lowercase()
      if (lower.startsWith("https:") || lower.startsWith("http:")) return Web(src)
      if (lower.startsWith("data:")) return dataUri(src)
      if (lower.startsWith("blob:")) return null
      val written = decoded(src)
      val read = images?.firstOrNull { it.uri?.let(::decoded) == written }
      if (read != null) return stored(read.blob, session) ?: session?.let { HostFile(it, written) }
      return session?.let { HostFile(it, written) }
    }

    private fun decoded(s: String) = runCatching { URLDecoder.decode(s.replace("+", "%2B"), "UTF-8") }.getOrDefault(s)
  }
}

/** One image of a session (`MediaLoader.entries`): where its pixels are and the name its card gives it */
data class MediaEntry(val source: MediaSource, val name: String?)

/**
 * Decoded pixels, orientation applied, the long side capped at [MediaLoader.MAX_PIXEL]. An animation (GIF, animated
 * WebP) keeps its encoded [animation] bytes too, which the views play through the platform's AnimatedImageDrawable one
 * frame at a time; [bitmap] is its first frame. Equal by identity, as a fresh decode is a new picture
 */
class DecodedImage(val bitmap: Bitmap, val animation: ByteArray? = null) {
  val image: ImageBitmap = bitmap.asImageBitmap()
  val width: Int get() = bitmap.width
  val height: Int get() = bitmap.height
  /** Width over height */
  val aspect: Float get() = width.toFloat() / max(1, height)
  /** Bytes the bitmap (and an animation's encoded file) hold, the cache's measure */
  val cost: Long get() = bitmap.allocationByteCount.toLong() + (animation?.size ?: 0)
}

/**
 * The images a chat shows, fetched once per app run and kept in memory ([MediaCache]): blobs through the store's fetch,
 * staged files through its `imageBlob` request, web images straight from the network, inline pixels decoded in place.
 * Views ask by [MediaSource]; nothing here knows how the store reaches the engine
 */
class MediaLoader(
  private val fetch: BlobFetch,
  /** Stages a file on the engine's machine into the session's blob store, answering its blob name (null: unreadable) */
  private val stage: (suspend (session: String, path: String) -> String?)? = null,
  private val web: suspend (String) -> ByteArray = { HttpBlobs.get(it) },
  /**
   * Writes a session blob into a file, resuming after what the file holds, reporting the bytes held and the total;
   * answers the blob's size (`PairedBlobs.download`)
   */
  private val download: (suspend (session: String, name: String, to: File, progress: (Long, Long) -> Unit) -> Long)? = null,
) {
  class Failure(message: String) : IOException(message)

  private val cache = MediaCache<MediaSource, DecodedImage>(BUDGET, LIMIT, { it.cost })
  /** The blob each file source was last staged as: its image shows at once while the engine checks the file again */
  private val stagedAs = java.util.concurrent.ConcurrentHashMap<MediaSource, MediaSource>()

  /** Whether files on the engine's machine can be staged at all; no stager fails them at once */
  val canStage: Boolean get() = stage != null

  /**
   * The image already at hand: a kept one, or inline pixels decoded in place, so a view draws at its final size on its
   * first pass
   */
  fun peek(source: MediaSource): DecodedImage? {
    if (source is MediaSource.HostFile) return stagedAs[source]?.let { cache.cached(it) }
    cache.cached(source)?.let { return it }
    if (source !is MediaSource.Inline) return null
    val bytes = runCatching { Base64.decode(source.base64, Base64.DEFAULT) }.getOrNull() ?: return null
    val image = decodeNow(bytes) ?: return null
    cache.keep(source, image)
    return image
  }

  /**
   * Keeps the photos the phone sends under the blob names the engine will store them as. The engine's copy of the
   * prompt (queue entry, user turn) names that blob instead of carrying the pixels, and without this the thumbnail fell
   * back to its placeholder while the same bytes came back over the link. Blob names are content hashes, so a wrong
   * guess is never hit
   */
  fun adopt(images: List<Pair<String, String>>, session: String) {
    for ((base64, mimeType) in images) {
      val bytes = runCatching { Base64.decode(base64, Base64.DEFAULT) }.getOrNull() ?: continue
      val image = peek(MediaSource.Inline(base64)) ?: continue
      cache.keep(MediaSource.Blob(session, BlobNames.ofImage(bytes, mimeType)), image)
    }
  }

  /** Loads and keeps the image; joins a load already under way for the same source */
  suspend fun image(source: MediaSource): DecodedImage {
    // A file is staged every time: the engine answers the blob it already read while the file is unchanged (a stat, no
    // transfer), and a new one once it changed, so a rewritten file shown again is not the old picture
    if (source is MediaSource.HostFile) {
      val name = stage?.invoke(source.session, source.path) ?: throw Failure("not staged")
      val blob = MediaSource.Blob(source.session, name)
      stagedAs[source] = blob
      return image(blob)
    }
    peek(source)?.let { return it }
    return cache.load(source) {
      val data = when (source) {
        is MediaSource.Blob -> fetch(source.session, source.name)
        is MediaSource.Web -> web(source.url)
        is MediaSource.Inline -> Base64.decode(source.base64, Base64.DEFAULT)
        is MediaSource.HostFile -> throw Failure("not staged")
      }
      withContext(Dispatchers.Default) { decodeNow(data) } ?: throw Failure("undecodable")
    }
  }

  /** The blob a video or widget plays from: its saved copy, else its file on the engine's side staged now. null: nothing */
  suspend fun blobName(session: String, blob: String?, uri: String?): String? {
    if (blob != null) return blob
    if (uri == null || !MediaSource.isFilePath(uri)) return null
    return stage?.invoke(session, uri)
  }

  /**
   * A video or a downloaded file, fetched only after a tap into [to] (never decoded into the image cache): streamed and
   * resumable over a paired link, else fetched whole. Answers the blob's size
   */
  suspend fun download(session: String, blob: String, to: File, progress: (Long, Long) -> Unit): Long {
    download?.let { return it(session, blob, to, progress) }
    val bytes = fetch(session, blob)
    withContext(Dispatchers.IO) { to.writeBytes(bytes) }
    progress(bytes.size.toLong(), bytes.size.toLong())
    return bytes.size.toLong()
  }

  /** A stored blob's text (a widget's page), read fresh each time: pages are small and not decoded images */
  suspend fun text(session: String, blob: String): String = fetch(session, blob).decodeToString()

  /** Every image a session shows, loaded before a launch-time Lightbox; failures are left to the views */
  suspend fun preload(view: SessionView) {
    for (source in sources(view)) runCatching { image(source) }
  }

  companion object {
    /** The longest side a decoded image keeps: a Lightbox zoomed to its cap stays sharp, a 5K screenshot is not 60 MB */
    const val MAX_PIXEL = 3072
    /** Every kept bitmap together, and how many */
    const val BUDGET = 96L shl 20
    const val LIMIT = 64

    /** The demo's loader: blob names read the fixtures' bundled files, nothing is staged */
    fun demo(context: Context): MediaLoader {
      val assets = context.applicationContext.assets
      return MediaLoader(fetch = { _, name ->
        withContext(Dispatchers.IO) { runCatching { assets.open("fixtures/blobs/$name").use { it.readBytes() } }.getOrNull() }
          ?: throw Failure("no fixture blob $name")
      })
    }

    /**
     * ImageDecoder's path: EXIF orientation applied, the long side capped at [MAX_PIXEL] (never enlarged). An animated
     * file keeps its bytes for playback beside its first frame
     */
    fun decodeNow(data: ByteArray): DecodedImage? = runCatching {
      var animated = false
      val bitmap = ImageDecoder.decodeBitmap(ImageDecoder.createSource(ByteBuffer.wrap(data))) { decoder, info, _ ->
        animated = info.isAnimated
        val side = max(info.size.width, info.size.height)
        if (side > MAX_PIXEL) {
          val scale = MAX_PIXEL.toFloat() / side
          decoder.setTargetSize(max(1, (info.size.width * scale).toInt()), max(1, (info.size.height * scale).toInt()))
        }
      }
      DecodedImage(bitmap, if (animated) data else null)
    }.getOrNull()

    /** The image sources of a session in reading order: prompt attachments, agent images, tool images, Markdown images */
    fun sources(view: SessionView): List<MediaSource> = entries(view).map { it.source }

    /**
     * [sources] with the name each card gives its image (the Lightbox's title): an attachment's file name, the file an
     * agent or tool image's `uri` names, a Markdown image's alt text or file name. Each source once, where it first shows
     */
    fun entries(view: SessionView): List<MediaEntry> {
      val out = ArrayList<MediaEntry>()
      val seen = HashSet<MediaSource>()
      fun add(source: MediaSource?, name: String?) {
        if (source != null && seen.add(source)) out += MediaEntry(source, name)
      }
      fun fileName(uri: String?) = uri?.let(::fileOfUri)?.substringAfterLast('/')
      fun attachments(list: List<Attachment>?) {
        for (a in list.orEmpty()) if (a is Attachment.Image) add(MediaSource.stored(a.blob, view.id), a.name)
      }
      for (turn in view.turns) {
        when (turn) {
          is UserTurn -> attachments(turn.attachments)
          is AgentTurn -> for (block in turn.blocks) {
            when (block) {
              is ImageBlock -> add(MediaSource.agentImage(block.blob, block.uri, view.id), fileName(block.uri))
              is ToolCallBlock -> for (image in toolImages(block)) add(MediaSource.agentImage(image.blob, image.uri, view.id), fileName(image.uri))
              is TextBlock -> for ((src, alt) in markdownImages(block.markdown)) {
                // MarkdownImage's name: the alt text, else the file
                add(MediaSource.markdown(src, block.images, view.id), alt.ifEmpty { src.substringAfterLast('/') })
              }
              is SteerBlock -> attachments(block.attachments)
              else -> {}
            }
          }
          else -> {}
        }
      }
      return out
    }

    private val MD_IMAGE = Regex("""!\[([^\]]*)]\(\s*<?([^)\s>]+)>?(?:\s+"[^"]*")?\s*\)""")

    /** The `![alt](src)` images of a Markdown text, outside fenced code */
    fun markdownImages(markdown: String): List<Pair<String, String>> {
      if ("![" !in markdown) return emptyList()
      var fenced = false
      val out = ArrayList<Pair<String, String>>()
      for (line in markdown.lineSequence()) {
        if (line.trimStart().startsWith("```")) { fenced = !fenced; continue }
        if (!fenced) MD_IMAGE.findAll(line).forEach { out += it.groupValues[2] to it.groupValues[1] }
      }
      return out
    }
  }
}

/** A row item's identity: its blob, or the path it is kept as */
val ImageRef.uriOrBlob: String get() = blob ?: uri ?: ""

/**
 * The file an agent image's `uri` names (fileLinks.ts parseFileLink, the cases an agent writes): a path or a file URL;
 * web and data URIs name none
 */
fun fileOfUri(uri: String): String? {
  val lower = uri.lowercase()
  if (lower.startsWith("file:")) return runCatching { URI(uri).path }.getOrNull()
  if (Regex("^[a-z][a-z0-9+.-]*:").containsMatchIn(lower) && !Regex("""^[a-z]:[\\/]""").containsMatchIn(lower)) return null
  return uri.ifEmpty { null }
}

private val SHOW_VERBS = setOf("verb.showImage", "verb.showVideo", "verb.showWidget")

/**
 * imageTools.ts savedContents: a saved item, or a show tool's file kept as its path (a session on an SSH host), which
 * the engine stages on demand; anywhere else an unsaved item is a lost one
 */
private fun kept(ref: ToolContent.Image, block: ToolCallBlock): Boolean =
  ref.blob != null || (block.verbKey in SHOW_VERBS && ref.uri?.let(MediaSource::isFilePath) == true)

private fun toolRefs(block: ToolCallBlock, mime: (String) -> Boolean): List<ImageRef> =
  (block.contents ?: listOfNotNull(block.content)).mapNotNull {
    if (it is ToolContent.Image && kept(it, block) && mime(it.mimeType)) ImageRef(it.blob, it.mimeType, it.uri) else null
  }

/**
 * The saved images of a tool call in wire order (imageTools.ts toolImages): `contents` holds every item when there is
 * more than one. A widget's HTML rides as an image of type text/html and is not one
 */
fun toolImages(block: ToolCallBlock): List<ImageRef> = toolRefs(block) { it.startsWith("image/") }

fun toolVideos(block: ToolCallBlock): List<ImageRef> = toolRefs(block) { it.startsWith("video/") }

/** imageTools.ts WIDGET_MIME: show_widget stores its HTML as an image content (ui/WidgetView.kt draws it) */
fun isWidget(mimeType: String): Boolean = mimeType == "text/html"

/** imageTools.ts toolWidget: the saved page of a show_widget call */
fun toolWidget(block: ToolCallBlock): ImageRef? = toolRefs(block, ::isWidget).firstOrNull()

/** imageTools.ts isImageResultBlock: generated and shown images, and widgets, are results for the reader */
fun isImageResult(block: ToolCallBlock): Boolean =
  block.verbKey == "verb.imagegen" || block.verbKey in SHOW_VERBS

/** attachments.ts imageMimeOf: the image types the engine stages (`image_mime_of`), by extension */
fun imageMimeOf(name: String): String? {
  val tail = name.substringAfterLast('/').substringAfterLast('\\')
  val dot = tail.lastIndexOf('.')
  if (dot < 0) return null
  return when (tail.substring(dot + 1).lowercase()) {
    "png" -> "image/png"
    "jpg", "jpeg" -> "image/jpeg"
    "gif" -> "image/gif"
    "webp" -> "image/webp"
    else -> null
  }
}
