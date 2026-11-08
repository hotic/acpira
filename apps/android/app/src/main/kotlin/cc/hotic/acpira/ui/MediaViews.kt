package cc.hotic.acpira.ui

import android.graphics.ImageDecoder
import android.graphics.drawable.AnimatedImageDrawable
import android.graphics.drawable.Drawable
import android.os.Handler
import android.os.Looper
import androidx.compose.animation.Crossfade
import androidx.compose.animation.core.tween
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.RememberObserver
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.runtime.compositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.blur
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.drawIntoCanvas
import androidx.compose.ui.graphics.nativeCanvas
import androidx.compose.ui.graphics.painter.Painter
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import cc.hotic.acpira.design.LocalMotion
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.WebMetrics
import cc.hotic.acpira.design.WebType
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.model.Attachment
import cc.hotic.acpira.model.ImageRef
import cc.hotic.acpira.model.ToolCallBlock
import cc.hotic.acpira.model.ToolStatus
import cc.hotic.acpira.store.AppModel
import cc.hotic.acpira.store.DecodedImage
import cc.hotic.acpira.store.MediaLoader
import cc.hotic.acpira.store.MediaSource
import cc.hotic.acpira.store.Screen
import cc.hotic.acpira.store.fileOfUri
import cc.hotic.acpira.store.isVideoName
import cc.hotic.acpira.store.toolImages
import cc.hotic.acpira.store.toolVideos
import cc.hotic.acpira.store.toolWidget
import cc.hotic.acpira.store.uriOrBlob
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.nio.ByteBuffer

// apps/ios Media/MediaViews.swift (chat/AgentImage.tsx, GeneratedImage.tsx and the image parts of Attachments.tsx and
// Prose.tsx at the phone's caps). A transcript image takes a card `agentImage` tall from its first pass, loading or
// loaded, so nothing below moves when its pixels land (only the card's width follows the picture); a tap opens it in
// the Lightbox (ui/Lightbox.kt).

/** The image the Lightbox shows, its name when it has a real one, and its source among the session's images */
class LightboxItem(val image: DecodedImage, val name: String?, val source: MediaSource?)

/** Opens and closes a chat's Lightbox: a holder in the scope rather than a callback, so image views do not redraw with it */
@Stable
class LightboxPresenter {
  var item by mutableStateOf<LightboxItem?>(null)
    private set

  fun open(item: LightboxItem) {
    this.item = item
  }

  fun close() {
    item = null
  }
}

/**
 * What the image views of one chat read: the session their blobs belong to, the loader, the Lightbox and the app model
 * (downloads, file pages). The default is no session (only inline pixels show) and no Lightbox
 */
@Stable
class MediaScope(val session: String?, val loader: MediaLoader, val lightbox: LightboxPresenter?, val model: AppModel?)

private val inlineOnly by lazy { MediaLoader(fetch = { _, _ -> throw MediaLoader.Failure("no session") }) }

val LocalMedia = staticCompositionLocalOf { MediaScope(null, inlineOnly, null, null) }

/** Whether a tool row owns its media; a process fold renders media as separate, always-visible parts */
val LocalImageInFold = compositionLocalOf { true }

/**
 * The chat's media (MediaViews.swift MediaLayer): the scope its image views read and the Lightbox they open over the
 * whole screen. The `popup` launch extra `lightbox` opens the session's first image on launch (screenshots)
 */
@Composable
fun MediaLayer(session: String, model: AppModel, content: @Composable () -> Unit) {
  val lightbox = remember(session) { LightboxPresenter() }
  val loader = model.media.loader
  val scope = remember(session, loader) { MediaScope(session, loader, lightbox, model) }
  CompositionLocalProvider(LocalMedia provides scope) { content() }
  LaunchedEffect(session) {
    if (model.options.popup != "lightbox" || LightboxProbe.claimed) return@LaunchedEffect
    LightboxProbe.claimed = true
    val view = model.views[session] ?: return@LaunchedEffect
    loader.preload(view)
    MediaLoader.sources(view).firstNotNullOfOrNull { s -> loader.peek(s)?.let { s to it } }
      ?.let { (s, image) -> lightbox.open(LightboxItem(image, null, s)) }
  }
  lightbox.item?.let { item ->
    // The view is read once the Lightbox opens, so a streamed chunk does not rebuild it
    Lightbox(item, loader, session = { model.views[session] }, motion = model.motion) { lightbox.close() }
  }
}

/** `popup lightbox` opens once per launch, not on every return to the chat */
private object LightboxProbe {
  var claimed = false
}

// Loading

/**
 * One image source through the chat's loader: [content] with the pixels, [placeholder] while they load, [failure] when
 * there are none (no source, a failed fetch, bytes that do not decode). Pixels already at hand draw on the first pass,
 * so a row scrolled back into the thread shows no placeholder
 */
@Composable
fun MediaImage(
  source: MediaSource?,
  content: @Composable (DecodedImage) -> Unit,
  placeholder: @Composable () -> Unit,
  failure: @Composable () -> Unit,
) {
  if (source == null) {
    failure()
    return
  }
  val loader = LocalMedia.current.loader
  val motion = LocalMotion.current
  // The finished load of `source`: null while it runs, an empty holder when it failed
  var loaded by remember(source, loader) { mutableStateOf<Loaded?>(null) }
  val peeked = remember(source, loader, loaded) { loaded?.image ?: loader.peek(source) }
  LaunchedEffect(source, loader) {
    if (peeked != null) {
      // A file shown from the copy an earlier view loaded: checked again, swapped only when it changed
      if (source !is MediaSource.HostFile) return@LaunchedEffect
      val fresh = runCatching { loader.image(source) }.getOrNull()
      if (fresh != null && fresh !== peeked) loaded = Loaded(fresh)
      return@LaunchedEffect
    }
    loaded = Loaded(runCatching { loader.image(source) }.getOrNull())
  }
  val state = when {
    peeked != null -> Phase.Shown
    loaded != null -> Phase.Failed
    else -> Phase.Loading
  }
  Crossfade(state, animationSpec = tween(if (motion) QUICK_MS else 0), label = "media") { phase ->
    when (phase) {
      Phase.Shown -> peeked?.let { content(it) } ?: placeholder()
      Phase.Failed -> failure()
      Phase.Loading -> placeholder()
    }
  }
}

private class Loaded(val image: DecodedImage?)

private enum class Phase { Loading, Shown, Failed }

/**
 * A decoded image as a picture; an animation plays through the platform's AnimatedImageDrawable while on screen, as an
 * `<img>` plays a GIF on the desktop
 */
@Composable
fun Picture(image: DecodedImage, modifier: Modifier = Modifier, contentScale: ContentScale = ContentScale.Fit, description: String? = null) {
  val bytes = image.animation
  if (bytes == null) {
    Image(image.image, description, modifier, contentScale = contentScale)
    return
  }
  var painter by remember(image) { mutableStateOf<AnimatedPainter?>(null) }
  LaunchedEffect(image) {
    painter = withContext(Dispatchers.Default) {
      runCatching { ImageDecoder.decodeDrawable(ImageDecoder.createSource(ByteBuffer.wrap(bytes))) }.getOrNull()
    }?.let { (it as? AnimatedImageDrawable)?.let(::AnimatedPainter) }
  }
  val p = painter
  if (p != null) Image(p, description, modifier, contentScale = contentScale)
  else Image(image.image, description, modifier, contentScale = contentScale)
}

/**
 * An AnimatedImageDrawable as a painter: the drawable decodes its frames on its own thread and asks for a redraw, which
 * bumps [tick]; it runs while remembered (on screen) and stops when it leaves
 */
private class AnimatedPainter(private val drawable: AnimatedImageDrawable) : Painter(), RememberObserver {
  private var tick by mutableIntStateOf(0)
  private val handler = Handler(Looper.getMainLooper())
  private val callback = object : Drawable.Callback {
    override fun invalidateDrawable(who: Drawable) {
      tick++
    }
    override fun scheduleDrawable(who: Drawable, what: Runnable, `when`: Long) {
      handler.postAtTime(what, `when`)
    }
    override fun unscheduleDrawable(who: Drawable, what: Runnable) {
      handler.removeCallbacks(what)
    }
  }

  override val intrinsicSize: Size
    get() = Size(drawable.intrinsicWidth.toFloat(), drawable.intrinsicHeight.toFloat())

  override fun DrawScope.onDraw() {
    tick
    drawIntoCanvas { canvas ->
      drawable.setBounds(0, 0, size.width.toInt(), size.height.toInt())
      drawable.draw(canvas.nativeCanvas)
    }
  }

  override fun onRemembered() {
    drawable.callback = callback
    drawable.repeatCount = AnimatedImageDrawable.REPEAT_INFINITE
    drawable.start()
  }

  override fun onForgotten() {
    drawable.stop()
    drawable.callback = null
    handler.removeCallbacksAndMessages(null)
  }

  override fun onAbandoned() = onForgotten()
}

// Agent images

/**
 * AgentImage.tsx: an image the agent put in the transcript (an image block, a tool's image content). Without pixels (a
 * refused payload, a URI-only reference, a failed fetch) the card names it instead. The agent's `uri`, where it saved
 * its copy, rides below as a caption unless the row already names the file
 */
@Composable
fun AgentImageView(blob: String?, uri: String?, caption: Boolean = true) {
  val media = LocalMedia.current
  val file = uri?.let(::fileOfUri)
  Column(Modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(Metrics.s1)) {
    ImageSlot(MediaSource.agentImage(blob, uri, media.session), file?.substringAfterLast('/'))
    if (caption && uri != null && file != null) {
      Text(uri, style = WebType.text3, color = Palette.fg3, maxLines = 1, overflow = TextOverflow.MiddleEllipsis)
    }
  }
}

/** The card in its slot; one that cannot load drops the slot for the named chip */
@Composable
fun ImageSlot(source: MediaSource?, name: String?) {
  val media = LocalMedia.current
  val label = name ?: t("common.image")
  MediaImage(
    source,
    content = { image ->
      ImageCard(
        Metrics.agentImage * image.aspect,
        Modifier
          .clickable { media.lightbox?.open(LightboxItem(image, name, source)) }
          .semantics { contentDescription = t("common.previewImage", "name" to label) }
          .testTag("media.image"),
        // The bands beside a wide picture: the picture itself, blurred, so the card reads as one piece
        fill = { Image(image.image, null, Modifier.fillMaxSize().blur(Metrics.page), contentScale = ContentScale.Crop) },
      ) { Picture(image, Modifier.fillMaxSize()) }
    },
    placeholder = { ImagePlaceholder() },
    failure = {
      AttachmentPill(label, "image", Modifier.semantics { contentDescription = t("attach.loadFailed", "name" to label) })
    },
  )
}

/**
 * The rounded card every transcript image takes: `agentImage` tall, as wide as the picture is at that height or the
 * column at most, the picture fitted inside over [fill] with the line border. A wide picture sits between bands of the
 * fill rather than shrinking the card, so the height never depends on the pixels
 */
@Composable
fun ImageCard(width: Dp, modifier: Modifier = Modifier, fill: @Composable () -> Unit, content: @Composable () -> Unit) {
  val shape = RoundedCornerShape(Metrics.rMd)
  Box(Modifier.fillMaxWidth()) {
    Box(
      Modifier
        .widthIn(max = width)
        .fillMaxWidth()
        .height(Metrics.agentImage)
        .clip(shape)
        .border(Metrics.hairline, Palette.line, shape)
        .then(modifier),
      contentAlignment = Alignment.Center,
    ) {
      fill()
      content()
    }
  }
}

/** The card's stand-in while the pixels load, and a running image generation's: a square card with the image glyph */
@Composable
fun ImagePlaceholder() {
  ImageCard(Metrics.agentImage, Modifier.semantics { contentDescription = t("common.image") }, fill = {
    Box(Modifier.fillMaxSize().background(Palette.chip))
  }) { Lucide("image", Palette.fg3, size = Metrics.iconCtl) }
}

/**
 * GeneratedImage.tsx GeneratedImages: image, video and widget results. A process fold keeps these outside the collapsing
 * rows (`grouped`: the caller is a tool row in that fold). A generation still running with nothing saved holds the
 * card's place; shown images drop their path captions, as their row names the files
 */
@Composable
fun ImageResults(block: ToolCallBlock, grouped: Boolean = false) {
  if (grouped && !LocalImageInFold.current) return
  val media = LocalMedia.current
  val shown = block.verbKey == "verb.showImage"
  val images = remember(block) { toolImages(block) }
  when {
    block.verbKey == "verb.showVideo" -> Column(verticalArrangement = Arrangement.spacedBy(Metrics.gap)) {
      for (video in remember(block) { toolVideos(block) }) androidx.compose.runtime.key(video.uriOrBlob) { AgentVideoView(video) }
    }
    block.verbKey == "verb.showWidget" -> {
      // Nothing while the call runs or when it failed: the row says so (AgentWidget.tsx ShownWidget). One kept as a path
      // (a session on an SSH host) is staged by the engine first
      val widget = remember(block) { toolWidget(block) }
      val session = media.session
      if (widget != null && session != null) {
        val loader = media.loader
        androidx.compose.runtime.key(widget.uriOrBlob) {
          WidgetBlock {
            val blob = loader.blobName(session, widget.blob, widget.uri) ?: throw MediaLoader.Failure("not staged")
            loader.text(session, blob)
          }
        }
      }
    }
    images.isEmpty() && !shown && (block.status == ToolStatus.pending || block.status == ToolStatus.in_progress) -> ImagePlaceholder()
    images.isNotEmpty() -> Column(verticalArrangement = Arrangement.spacedBy(Metrics.gap)) {
      for (image in images) AgentImageView(image.blob, image.uri, caption = !shown)
    }
  }
}

/** A tool row with image results: the row, then its results (ToolViews.swift `isImageResult`) */
@Composable
fun ImageResultRow(block: ToolCallBlock, grouped: Boolean, row: @Composable () -> Unit) {
  Column(verticalArrangement = Arrangement.spacedBy(Metrics.gap)) {
    row()
    ImageResults(block, grouped)
  }
}

// Attachments

/**
 * Attachments.tsx AttachmentTag as a thumbnail: a [side] square on the chip fill, the picture cropped to fill it; a tap
 * opens the whole picture in the Lightbox. The glyph stands in while it loads and when it cannot
 */
@Composable
fun AttachmentThumbnail(blob: String?, name: String?, side: Dp = Metrics.thumb) {
  val media = LocalMedia.current
  val shape = RoundedCornerShape(Metrics.rSm)
  val glyph = @Composable {
    Box(Modifier.size(side).background(Palette.chip, shape), contentAlignment = Alignment.Center) {
      Lucide("image", Palette.fg3, size = Metrics.icon)
    }
  }
  val source = MediaSource.stored(blob, media.session)
  MediaImage(
    source,
    content = { image ->
      Picture(
        image,
        Modifier.size(side).clip(shape).clickable { media.lightbox?.open(LightboxItem(image, name, source)) }.testTag("media.thumb"),
        contentScale = ContentScale.Crop,
        description = t("common.previewImage", "name" to (name ?: t("common.image"))),
      )
    },
    placeholder = glyph,
    failure = glyph,
  )
}

/**
 * Cards.swift AttachmentPill: a file's glyph and name on the chip fill; [onOpen] makes the glyph and name a button,
 * [onRemove] adds a staged pill's ✕ after the name
 */
@Composable
fun AttachmentPill(name: String, icon: String, modifier: Modifier = Modifier, onOpen: (() -> Unit)? = null, onRemove: (() -> Unit)? = null) {
  Row(
    modifier
      .height(WebMetrics.ctlSm)
      .background(Palette.chip, RoundedCornerShape(Metrics.rSm))
      .clip(RoundedCornerShape(Metrics.rSm))
      .padding(horizontal = Metrics.s2),
    verticalAlignment = Alignment.CenterVertically,
    horizontalArrangement = Arrangement.spacedBy(Metrics.s1),
  ) {
    Row(
      (if (onOpen != null) Modifier.clickable(onClick = onOpen).testTag("media.pill") else Modifier).weight(1f, fill = false),
      verticalAlignment = Alignment.CenterVertically,
      horizontalArrangement = Arrangement.spacedBy(Metrics.s1),
    ) {
      Lucide(icon, Palette.fg3, size = WebMetrics.icon)
      Text(name, style = WebType.text3, fontWeight = FontWeight.Medium, color = Palette.fg2, maxLines = 1, overflow = TextOverflow.MiddleEllipsis)
    }
    if (onRemove != null) {
      Box(Modifier.size(WebMetrics.ctlSm).clickable(onClick = onRemove), contentAlignment = Alignment.Center) {
        Lucide("x", Palette.fg3, size = Metrics.chevron, contentDescription = t("common.remove"))
      }
    }
  }
}

/**
 * A prompt's attachments, wrapping and packed to the right (Cards.swift TurnAttachments): image thumbnails, video
 * tiles, file and selection pills. A video shows its first frame and plays in the phone's player; another file on the
 * computer opens its download page
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun TurnAttachments(attachments: List<Attachment>) {
  val media = LocalMedia.current
  val context = LocalContext.current
  var playing by remember { mutableStateOf<Pair<java.io.File, String>?>(null) }
  FlowRow(
    horizontalArrangement = Arrangement.spacedBy(Metrics.gap, Alignment.End),
    verticalArrangement = Arrangement.spacedBy(Metrics.gap),
  ) {
    for (item in attachments) {
      when (item) {
        // Inline pixels when the app staged the photo itself, else the engine's blob
        is Attachment.Image -> AttachmentThumbnail(item.blob, item.name)
        is Attachment.File -> {
          val video = remember(item.uri, item.name, media.session) { VideoSource.of(context, item.uri, item.name, media.session, media.loader) }
          if (video != null) VideoAttachmentTile(video, item.name) { playing = it to item.name }
          else AttachmentPill(item.name, if (isVideoName(item.name)) "fileVideo" else "fileText", onOpen = fileOpener(item.uri))
        }
        is Attachment.Text -> AttachmentPill(item.name, "fileText")
        is Attachment.Selection -> AttachmentPill(
          "${item.name} ${if (item.startLine == item.endLine) "L${item.startLine}" else "L${item.startLine}-${item.endLine}"}",
          "textSelect", onOpen = fileOpener(item.uri),
        )
        is Attachment.Quote -> AttachmentPill(item.text.take(QUOTE_PILL), "messageSquareText")
        is Attachment.Unknown -> {}
      }
    }
  }
  playing?.let { (file, name) -> VideoPlayer(file, name) { playing = null } }
}

/**
 * What a tap on an attached file opens (Composer.swift FileOpener): a file on the computer in its download page. null
 * when nothing can: no live session to stage it from, or no path
 */
@Composable
private fun fileOpener(uri: String): (() -> Unit)? {
  val media = LocalMedia.current
  val model = media.model ?: return null
  val session = media.session ?: return null
  if (!model.live) return null
  val path = runCatching { java.net.URI(uri) }.getOrNull()?.takeIf { it.scheme == "file" }?.path ?: uri.takeIf { MediaSource.isFilePath(it) } ?: return null
  return { model.push(Screen.Download(session, path)) }
}

/** Motion.quick: a picture fading in over its placeholder */
internal const val QUICK_MS = 180

/** A quoted text's pill keeps this many characters */
private const val QUOTE_PILL = 24

/** The engine-read local images of the text being drawn (MediaViews.swift MarkdownImage, `TextBlock.images`) */
val LocalMarkdownImages = compositionLocalOf<List<ImageRef>?> { null }

private val PARAGRAPH_IMAGE = Regex("""!\[([^\]]*)]\(\s*<?([^)\s>]+)>?(?:\s+"[^"]*")?\s*\)""")

/** A paragraph run: text, or an image's source and alt text */
sealed interface ParagraphPart {
  data class Words(val text: String) : ParagraphPart
  data class Picture(val src: String, val alt: String) : ParagraphPart
}

/**
 * A paragraph's text runs and images in order (MediaViews.swift ImageParagraph; Prose.tsx turns a paragraph holding an
 * image into a block): the text between images stands as paragraphs of its own, trimmed, empty runs dropped
 */
fun paragraphParts(text: String): List<ParagraphPart> {
  val out = ArrayList<ParagraphPart>()
  var at = 0
  fun flush(end: Int) {
    val run = text.substring(at, end).trim()
    if (run.isNotEmpty()) out += ParagraphPart.Words(run)
  }
  for (m in PARAGRAPH_IMAGE.findAll(text)) {
    // An escaped `\![…]` stays text
    if (m.range.first > 0 && text[m.range.first - 1] == '\\') continue
    flush(m.range.first)
    out += ParagraphPart.Picture(m.groupValues[2], m.groupValues[1])
    at = m.range.last + 1
  }
  flush(text.length)
  return out
}

/** Whether a paragraph holds an image (it is then drawn by [ImageParagraph]) */
fun holdsImage(text: String): Boolean = "![" in text && paragraphParts(text).any { it is ParagraphPart.Picture }

/** A paragraph with images: its text runs drawn by [words], each image as an agent image card between them */
@Composable
fun ImageParagraph(text: String, words: @Composable (String) -> Unit) {
  val parts = remember(text) { paragraphParts(text) }
  Column(verticalArrangement = Arrangement.spacedBy(Metrics.gap)) {
    for (part in parts) when (part) {
      is ParagraphPart.Words -> words(part.text)
      is ParagraphPart.Picture -> MarkdownImage(part.src, part.alt)
    }
  }
}

/**
 * MediaViews.swift MarkdownImage: a local image resolves to the blob the engine read with the text, or one staged on
 * demand; a web image loads as it is. Either takes the agent image's card, named by its alt text or file name
 */
@Composable
fun MarkdownImage(src: String, alt: String) {
  val media = LocalMedia.current
  val images = LocalMarkdownImages.current
  val source = remember(src, images, media.session) { MediaSource.markdown(src, images, media.session) }
  ImageSlot(source, alt.ifEmpty { src.substringAfterLast('/') })
}
