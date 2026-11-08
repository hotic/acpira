package cc.hotic.acpira.sync

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.withContext
import okhttp3.HttpUrl
import okhttp3.HttpUrl.Companion.toHttpUrlOrNull
import okhttp3.OkHttpClient
import okhttp3.Request
import java.io.IOException
import java.security.MessageDigest
import java.util.concurrent.TimeUnit

/**
 * How the phone reads a file from a session's blob store (images in a transcript): `(sessionId, blobName) -> bytes`.
 * The development engine serves them over HTTP ([HttpBlobs]); a paired connection brings its own ([PairedBlobs])
 */
typealias BlobFetch = suspend (session: String, name: String) -> ByteArray

/**
 * What the phone keeps of fetched media, by key (apps/ios AcpiraSync/MediaCache.swift): values held under a cost budget
 * and a count cap, the least recently used dropped first. Concurrent requests for one key share one load. A failed load
 * is not kept, so the next request tries again, and a value costing more than the whole budget is handed out but never
 * kept. Blob names are content hashes, so a kept value never goes stale. Safe from any thread
 */
class MediaCache<K : Any, V : Any>(
  /** The summed cost the kept values may reach */
  val budget: Long,
  /** How many may be kept */
  val limit: Int,
  private val cost: (V) -> Long,
  /** Where loads run: on past a cancelled caller, so a row scrolled away still fills the cache for the next one */
  private val scope: CoroutineScope = CoroutineScope(SupervisorJob() + Dispatchers.Default),
) {
  private class Entry<V>(val value: V, val cost: Long, var used: Long)

  private val lock = Any()
  private val entries = HashMap<K, Entry<V>>()
  private val loading = HashMap<K, Deferred<V>>()
  private var clock = 0L

  /** The summed cost of the kept values */
  var total = 0L
    private set

  val count: Int get() = synchronized(lock) { entries.size }

  /** The kept value, marked as just used */
  fun cached(key: K): V? = synchronized(lock) {
    val entry = entries[key] ?: return null
    entry.used = ++clock
    entry.value
  }

  /** The kept value, else the load already under way for [key], else a new one from [make] */
  suspend fun load(key: K, make: suspend () -> V): V {
    val running = synchronized(lock) {
      entries[key]?.let { it.used = ++clock; return it.value }
      loading[key] ?: scope.async {
        try {
          make().also { v -> synchronized(lock) { loading.remove(key) }; keep(key, v) }
        } catch (e: Throwable) {
          synchronized(lock) { loading.remove(key) }
          throw e
        }
      }.also { loading[key] = it }
    }
    return running.await()
  }

  /** Stores a value under [key] and drops the least recently used ones until the bounds hold again */
  fun keep(key: K, value: V) = synchronized(lock) {
    val size = cost(value)
    entries.remove(key)?.let { total -= it.cost }
    if (size > budget) return@synchronized
    entries[key] = Entry(value, size, ++clock)
    total += size
    while (total > budget || entries.size > limit) {
      val oldest = entries.entries.filter { it.key != key }.minByOrNull { it.value.used }?.key ?: break
      total -= entries.remove(oldest)?.cost ?: 0
    }
  }
}

/**
 * The development engine's blob route (`acpira --ws`, sidecar/ws.rs): `GET {blobBase}/{sessionId}/{blob}`, no
 * authentication, loopback only (apps/ios HTTPBlobs)
 */
object HttpBlobs {
  class Failure(message: String) : IOException(message)

  private val client: OkHttpClient by lazy {
    OkHttpClient.Builder().connectTimeout(10, TimeUnit.SECONDS).readTimeout(30, TimeUnit.SECONDS).build()
  }

  /** `{base}/{session}/{name}`, each part one percent-encoded path segment */
  fun url(base: String, session: String, name: String): HttpUrl? =
    base.toHttpUrlOrNull()?.newBuilder()?.addPathSegment(session)?.addPathSegment(name)?.build()

  /** A fetch against what [base] answers (InitState.blobBase); null reads as no blob route at all */
  fun fetch(base: () -> String?): BlobFetch = { session, name ->
    val url = base()?.let { url(it, session, name) } ?: throw Failure("no blob route")
    get(url)
  }

  /** The bytes at [url] (a web image too); anything but 200 fails */
  suspend fun get(url: String): ByteArray = get(url.toHttpUrlOrNull() ?: throw Failure("bad url"))

  private suspend fun get(url: HttpUrl): ByteArray = withContext(Dispatchers.IO) {
    client.newCall(Request.Builder().url(url).build()).execute().use { response ->
      if (response.code != 200) throw Failure("status ${response.code}")
      response.body.bytes()
    }
  }
}

/**
 * The blob names the engine stores bytes under (transcript_store.rs `blob_name`): the first 8 bytes of the content's
 * SHA-256 in hex, then an extension. A blob name always holds a dot; base64 never does
 */
object BlobNames {
  /** attachments.rs IMAGE_MIME, the first extension of each type */
  val imageExtensions = mapOf("image/png" to ".png", "image/jpeg" to ".jpg", "image/gif" to ".gif", "image/webp" to ".webp")

  private fun hash(bytes: ByteArray): String =
    MessageDigest.getInstance("SHA-256").digest(bytes).take(8).joinToString("") { "%02x".format(it) }

  /** A sent image's blob (`blob_name` over attachments.rs `ext_of_mime`) */
  fun ofImage(bytes: ByteArray, mimeType: String): String = hash(bytes) + (imageExtensions[mimeType] ?: ".bin")

  /** A sent file's blob (`blob_name` over attachments.rs `ext_of_name`) */
  fun ofFile(bytes: ByteArray, name: String): String = hash(bytes) + extOfName(name)

  /** attachments.rs `ext_of_name`: the name's own lowercased extension when it is a short plain token, `.bin` otherwise */
  fun extOfName(name: String): String {
    val tail = name.substringAfterLast('/').substringAfterLast('\\')
    val dot = tail.lastIndexOf('.')
    if (dot <= 0) return ".bin"
    val ext = tail.substring(dot + 1)
    return if (ext.length in 1..16 && ext.all { it.isLetterOrDigit() && it.code < 128 }) ".${ext.lowercase()}" else ".bin"
  }

  /** Whether [name] has the shape of a stored blob: 16 hex digits, a dot, a plain extension */
  fun isBlobName(name: String): Boolean {
    val dot = name.indexOf('.')
    if (dot != 16) return false
    val ext = name.substring(dot + 1)
    return name.take(16).all { it in '0'..'9' || it in 'a'..'f' || it in 'A'..'F' } &&
      ext.isNotEmpty() && ext.all { (it.isLetterOrDigit() && it.code < 128) || it == '_' }
  }
}
