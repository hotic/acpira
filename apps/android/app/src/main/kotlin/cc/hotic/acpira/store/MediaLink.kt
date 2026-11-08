package cc.hotic.acpira.store

import android.content.Context
import android.util.Base64
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.model.Draft
import cc.hotic.acpira.model.HostMsg
import cc.hotic.acpira.model.InitState
import cc.hotic.acpira.model.ProjectError
import cc.hotic.acpira.model.WebviewMsg
import cc.hotic.acpira.sync.HttpBlobs
import cc.hotic.acpira.sync.PairedBlobs
import cc.hotic.acpira.ui.VideoCache
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.withTimeoutOrNull

/** What staging a file answered: its blob and size, or why not (apps/ios Files/FileDownload.swift StagedFile) */
sealed interface StagedFile {
  data class Staged(val blob: String, val size: Long) : StagedFile
  data class Failed(val error: ProjectError) : StagedFile
}

/**
 * The store's media side of one wire (apps/ios Store/EngineLink.swift `media`, `stage`, and the stageFile part of
 * AppModel+Files.swift): the chat's image loader, files on the computer staged into a session's blob store, and the
 * photos and videos a send hands the engine kept under the blob names it will give them. A paired wire fetches blobs over
 * its own link (the computer's HTTP route is loopback only) and stages only files the session shows (`shown`: the
 * gateway refuses any other path); the development engine serves `{blobBase}/{session}/{blob}`
 */
class MediaLink(context: Context, wire: Wire, private val post: (WebviewMsg) -> Boolean) {
  private val app = context.applicationContext
  /** Where blobs load from: hello's, until InitState names the engine's own. A paired wire ignores it */
  @Volatile var blobBase: String? = (wire as? Wire.Engine)?.let { "http://127.0.0.1:${it.address.port}/blobs" }
    private set
  private val paired: PairedBlobs? = (wire as? Wire.Paired)?.transport as? PairedBlobs

  val loader: MediaLoader = when (wire) {
    is Wire.Engine, is Wire.Paired -> MediaLoader(
      fetch = paired?.let { p -> { s: String, n: String -> p.blob(s, n) } } ?: HttpBlobs.fetch { blobBase },
      stage = { s, p -> stage(s, p) },
      download = paired?.let { p -> { s, n, to, progress -> p.download(s, n, to, progress) } },
    )
    else -> MediaLoader.demo(app)
  }

  private val lock = Any()
  private var seq = 0
  private val staging = HashMap<Int, CompletableDeferred<String?>>()
  private val stagingFiles = HashMap<Int, CompletableDeferred<HostMsg.StagedFile>>()

  /** The engine's init: its own blob route, unless the blobs come over a paired link */
  fun init(state: InitState) {
    if (paired == null) state.blobBase?.let { blobBase = it }
  }

  /** The engine's answers this side asked for; false for anything else */
  fun receive(message: HostMsg): Boolean {
    when (message) {
      is HostMsg.ImageBlob -> synchronized(lock) { staging.remove(message.seq) }?.complete(message.blob)
      is HostMsg.StagedFile -> synchronized(lock) { stagingFiles.remove(message.seq) }?.complete(message)
      else -> return false
    }
    return true
  }

  /**
   * Stages a file on the engine's machine into the session's blob store (App.tsx imageBlob) and answers its blob name;
   * null when the engine could not read it or never answered
   */
  suspend fun stage(session: String, path: String): String? {
    val answer = CompletableDeferred<String?>()
    val n = synchronized(lock) { (++seq).also { staging[it] = answer } }
    if (!post(WebviewMsg.ImageBlob(session, path, n, shown = paired != null))) {
      synchronized(lock) { staging.remove(n) }
      return null
    }
    return withTimeoutOrNull(STAGE_TIMEOUT_MS) { answer.await() }.also { synchronized(lock) { staging.remove(n) } }
  }

  /** A file staged on the computer for the phone to download, whatever its type (FileDownload) */
  suspend fun stageFile(session: String, path: String): StagedFile {
    val engineNeeded = StagedFile.Failed(ProjectError.Message(t("mobile.engineNeeded")))
    val answer = CompletableDeferred<HostMsg.StagedFile>()
    val n = synchronized(lock) { (++seq).also { stagingFiles[it] = answer } }
    if (!post(WebviewMsg.StageFile(session, path, n))) {
      synchronized(lock) { stagingFiles.remove(n) }
      return engineNeeded
    }
    // No answer: an engine from before stageFile (its gateway drops the request) or a lost wire
    val got = withTimeoutOrNull(ASK_TIMEOUT_MS) { answer.await() }
    synchronized(lock) { stagingFiles.remove(n) }
    val blob = got?.blob
    return when {
      blob != null -> StagedFile.Staged(blob, got.size ?: 0)
      got?.error != null -> StagedFile.Failed(got.error!!)
      else -> engineNeeded
    }
  }

  /**
   * A send's attachments kept under the blob names the engine will store them as, so the engine's copy of the prompt
   * shows the photo and video already on the phone without fetching them back (MediaLoader.adopt, VideoCache.adopt)
   */
  fun adopt(session: String, drafts: List<Draft>) {
    loader.adopt(drafts.filterIsInstance<Draft.Image>().map { it.data to it.mimeType }, session)
    for (d in drafts) {
      if (d is Draft.File && d.data != null && isVideoName(d.name)) {
        val data = d.data
        VideoCache.adopt(app, d.name) { Base64.decode(data, Base64.DEFAULT) }
      }
    }
  }

  companion object {
    /** App.tsx IMAGE_BLOB_TIMEOUT: an answer that never comes reads as an unreadable file */
    const val STAGE_TIMEOUT_MS = 10_000L
    /** AppModel+Files.swift `ask`: an answer that never comes ends after this */
    const val ASK_TIMEOUT_MS = 20_000L
  }
}

/** A staged or sent file that is a video, by its name (`.ts` is TypeScript in a chat about code, not MPEG-TS) */
fun isVideoName(name: String): Boolean {
  val ext = name.substringAfterLast('.', "").lowercase()
  if (ext.isEmpty() || ext == "ts") return false
  val mime = android.webkit.MimeTypeMap.getSingleton().getMimeTypeFromExtension(ext)
  return mime?.startsWith("video/") == true || ext in VIDEO_EXTENSIONS
}

private val VIDEO_EXTENSIONS = setOf("mp4", "m4v", "mov", "webm", "mkv", "3gp", "avi")
