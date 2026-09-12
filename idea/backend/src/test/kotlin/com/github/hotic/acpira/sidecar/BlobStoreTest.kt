package com.github.hotic.acpira.sidecar

import org.junit.After
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertNull
import org.junit.Before
import org.junit.Test
import java.nio.file.Files
import java.nio.file.Path

class BlobStoreTest {
    private lateinit var sessions: Path
    private lateinit var outside: Path
    private val sessionId = "11111111-2222-3333-4444-555555555555"

    @Before
    fun setUp() {
        sessions = Files.createTempDirectory("acpira-sessions")
        outside = Files.createTempDirectory("acpira-outside")
        Files.createDirectories(sessions.resolve(sessionId))
        Files.write(sessions.resolve("$sessionId/abcdef0123456789.png"), byteArrayOf(1, 2, 3))
        Files.write(sessions.resolve("accounts.json"), "[]".toByteArray())
        Files.write(outside.resolve("secret.txt"), "nope".toByteArray())
        Files.createSymbolicLink(sessions.resolve("$sessionId/link.txt"), outside.resolve("secret.txt"))
    }

    @After
    fun tearDown() {
        outside.toFile().deleteRecursively()
        sessions.toFile().deleteRecursively()
    }

    @Test
    fun `reads a regular blob inside its session directory`() {
        assertArrayEquals(byteArrayOf(1, 2, 3), BlobStore.read(sessions, sessionId, "abcdef0123456789.png"))
    }

    @Test
    fun `returns null for missing unsafe and escaping blobs`() {
        assertNull(BlobStore.read(null, sessionId, "abcdef0123456789.png"))
        assertNull(BlobStore.read(sessions, sessionId, "missing.png"))
        assertNull(BlobStore.read(sessions, "..", "accounts.json"))
        assertNull(BlobStore.read(sessions, sessionId, ".."))
        assertNull(BlobStore.read(sessions, sessionId, "a/b"))
        assertNull(BlobStore.read(sessions, sessionId, "link.txt"))
    }
}
