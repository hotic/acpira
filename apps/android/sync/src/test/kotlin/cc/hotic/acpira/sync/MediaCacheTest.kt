package cc.hotic.acpira.sync

import kotlinx.coroutines.async
import kotlinx.coroutines.awaitAll
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import java.util.concurrent.atomic.AtomicInteger
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue

class MediaCacheTest {
  @Test
  fun blobNamesMatchTheEngine() {
    // sha256("hello") = 2cf24dba5fb0a30e…; the engine keeps the first 8 bytes (transcript_store.rs blob_name)
    val hello = "hello".toByteArray()
    assertEquals("2cf24dba5fb0a30e.png", BlobNames.ofImage(hello, "image/png"))
    assertEquals("2cf24dba5fb0a30e.jpg", BlobNames.ofImage(hello, "image/jpeg"))
    assertEquals("2cf24dba5fb0a30e.bin", BlobNames.ofImage(hello, "image/tiff"))
    assertEquals("2cf24dba5fb0a30e.mp4", BlobNames.ofFile(hello, "clip.MP4"))
    assertEquals(".bin", BlobNames.extOfName("Makefile"))
    assertEquals(".bin", BlobNames.extOfName(".env"))
    assertEquals(".bin", BlobNames.extOfName("a.tar-gz"))
    assertTrue(BlobNames.isBlobName("2cf24dba5fb0a30e.png"))
    assertFalse(BlobNames.isBlobName("video.mp4"))
    assertFalse(BlobNames.isBlobName("2cf24dba5fb0a30.png"))
  }

  @Test
  fun concurrentLoadsShareOne() = runBlocking {
    val cache = MediaCache<String, ByteArray>(budget = 100, limit = 10, cost = { it.size.toLong() })
    val runs = AtomicInteger()
    val got = (1..5).map {
      async { cache.load("a") { runs.incrementAndGet(); delay(50); ByteArray(4) } }
    }.awaitAll()
    assertEquals(1, runs.get())
    assertTrue(got.all { it === got[0] })
    assertEquals(4, cache.total)
  }

  @Test
  fun failedLoadIsNotKept() = runBlocking {
    val cache = MediaCache<String, ByteArray>(budget = 100, limit = 10, cost = { it.size.toLong() })
    assertFailsWith<IllegalStateException> { cache.load("a") { error("gone") } }
    assertNull(cache.cached("a"))
    assertEquals(3, cache.load("a") { ByteArray(3) }.size)
  }

  @Test
  fun leastRecentlyUsedGoesFirst() {
    val cache = MediaCache<String, ByteArray>(budget = 10, limit = 3, cost = { it.size.toLong() })
    cache.keep("a", ByteArray(4))
    cache.keep("b", ByteArray(4))
    cache.cached("a")
    // Over the budget: b, used least recently, goes
    cache.keep("c", ByteArray(4))
    assertNull(cache.cached("b"))
    assertEquals(8, cache.total)
    // Over the count: the oldest goes
    cache.keep("d", ByteArray(1))
    cache.keep("e", ByteArray(1))
    assertEquals(3, cache.count)
    // Larger than the whole budget: never kept
    cache.keep("huge", ByteArray(11))
    assertNull(cache.cached("huge"))
  }
}
