package com.github.hotic.acpira.sidecar

import java.nio.file.Files
import java.nio.file.Path

// Reads attachment bytes only from safe session/name segments under the real sessions root; missing files, directories, and symlink
// escapes all collapse to null so the frontend resource handler can answer 404 without exposing filesystem details.
object BlobStore {
    private val safeSegment = Regex("^[A-Za-z0-9._-]+$")

    fun read(sessionsDir: Path?, sessionId: String, name: String): ByteArray? {
        val root = sessionsDir ?: return null
        if (!valid(sessionId) || !valid(name)) return null
        val rootReal = runCatching { root.toRealPath() }.getOrNull() ?: return null
        val real = runCatching { root.resolve(sessionId).resolve(name).toRealPath() }.getOrNull() ?: return null
        if (!real.startsWith(rootReal) || !Files.isRegularFile(real)) return null
        return runCatching { Files.readAllBytes(real) }.getOrNull()
    }

    private fun valid(segment: String) = safeSegment.matches(segment) && segment != "." && segment != ".."
}
