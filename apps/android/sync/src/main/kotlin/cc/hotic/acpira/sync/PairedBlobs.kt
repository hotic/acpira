package cc.hotic.acpira.sync

import java.io.File

/**
 * A wire that fetches a session's blobs (images the transcript shows) from the engine's machine (apps/ios
 * AcpiraSync/PairedBlobs.swift). A paired connection does it over its own link: the phone cannot read
 * `helloOk.sessionsDir`, which is a path on the computer, and the computer's HTTP blob route is loopback only. Kept
 * beside [PairedConnection] rather than in it, so the screens can ask `transport as? PairedBlobs` without depending on
 * the iroh-backed module, which only exists once `net/` is built
 */
interface PairedBlobs {
  /** The bytes of `<sessionsDir>/<session>/<name>` on the computer */
  suspend fun blob(session: String, name: String): ByteArray

  /**
   * The same blob written into the file [to], resuming after what the file already holds (a large video, a file opened
   * from the phone); answers the blob's size. Fails only once the transfer stalls, never on the whole file's time, and
   * leaves the partial file for the next call. [progress] hears the bytes held and the total. A wire without its own
   * downloads fetches the blob whole
   */
  suspend fun download(session: String, name: String, to: File, progress: (have: Long, total: Long) -> Unit): Long {
    val bytes = blob(session, name)
    to.writeBytes(bytes)
    progress(bytes.size.toLong(), bytes.size.toLong())
    return bytes.size.toLong()
  }
}
