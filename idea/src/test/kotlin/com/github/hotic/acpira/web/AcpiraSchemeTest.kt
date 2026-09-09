package com.github.hotic.acpira.web

import com.github.hotic.acpira.web.AcpiraScheme.Resolution
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import java.nio.file.Files
import java.nio.file.Path

// The resource handler is the only thing between the webview and the file system: every path shape an attacker could put in an
// <img src> or fetch() must stay inside its root. Pure resolution, no CEF involved
class AcpiraSchemeTest {
    private lateinit var sessions: Path
    private lateinit var outside: Path

    @Before
    fun setUp() {
        sessions = Files.createTempDirectory("acpira-sessions")
        outside = Files.createTempDirectory("acpira-outside")
        Files.createDirectories(sessions.resolve("11111111-2222-3333-4444-555555555555"))
        Files.write(sessions.resolve("11111111-2222-3333-4444-555555555555/abcdef0123456789.png"), byteArrayOf(1, 2, 3))
        Files.write(outside.resolve("secret.txt"), "nope".toByteArray())
        Files.write(sessions.resolve("accounts.json"), "[]".toByteArray())
        // A symlink inside the sessions tree pointing out of it must not be followed
        Files.createSymbolicLink(sessions.resolve("11111111-2222-3333-4444-555555555555/link.txt"), outside.resolve("secret.txt"))
        AcpiraScheme.sessionsDir = sessions
        AcpiraScheme.register("tok", ViewPage("sidebar", "/* bridge */", "n0nce", ThemeVars(true, "#ccc", "#181818", "#1f1f1f", "#333", "#07d", "Menlo"), "en"))
    }

    @After
    fun tearDown() {
        AcpiraScheme.unregister("tok")
        AcpiraScheme.sessionsDir = null
        outside.toFile().deleteRecursively()
        sessions.toFile().deleteRecursively()
    }

    private fun status(url: String) = (AcpiraScheme.resolve(url) as Resolution.Status).code

    @Test fun `serves a blob inside the sessions directory with its mime type`() {
        val r = AcpiraScheme.resolve("https://acpira.local/blobs/11111111-2222-3333-4444-555555555555/abcdef0123456789.png") as Resolution.Bytes
        assertEquals("image/png", r.mime)
        assertEquals(3, r.bytes.size)
    }

    @Test fun `refuses every way out of the blob root`() {
        assertEquals(403, status("https://acpira.local/blobs/../accounts.json"))
        assertEquals(403, status("https://acpira.local/blobs/..%2f..%2faccounts.json"))
        assertEquals(403, status("https://acpira.local/blobs/11111111-2222-3333-4444-555555555555/../accounts.json"))
        assertEquals(403, status("https://acpira.local/blobs/11111111-2222-3333-4444-555555555555/%2e%2e/accounts.json"))
        assertEquals(403, status("https://acpira.local/blobs/accounts.json"))
        assertEquals(403, status("https://acpira.local/blobs/11111111-2222-3333-4444-555555555555/a/b"))
        assertEquals(403, status("https://acpira.local/blobs/11111111-2222-3333-4444-555555555555/link.txt"))
        assertEquals(404, status("https://acpira.local/blobs/11111111-2222-3333-4444-555555555555/missing.png"))
    }

    @Test fun `serves webview resources only from within the bundle`() {
        assertEquals(404, status("https://acpira.local/webview/../META-INF/plugin.xml"))
        assertEquals(404, status("https://acpira.local/webview/%2e%2e/META-INF/plugin.xml"))
        assertEquals(403, status("https://acpira.local/webview/"))
        assertEquals(404, status("https://acpira.local/webview/not-there.js"))
    }

    @Test fun `the page needs a registered view token and carries the bridge and nonce`() {
        assertEquals(404, status("https://acpira.local/index.html"))
        assertEquals(404, status("https://acpira.local/index.html?view=other"))
        val html = String((AcpiraScheme.resolve("https://acpira.local/index.html?view=tok") as Resolution.Bytes).bytes)
        assertTrue(html.contains("/* bridge */"))
        assertTrue(html.contains("nonce-n0nce"))
        assertTrue(html.contains("class=\"vscode-dark\""))
        assertTrue(html.contains("window.__acpira={host:\"sidebar\"}"))
    }

    @Test fun `other hosts and unknown paths are not served`() {
        assertEquals(404, status("https://example.com/webview/main.js"))
        assertEquals(404, status("https://acpira.local/etc/passwd"))
        assertEquals(400, status("https://acpira.local/webview/%00"))
    }
}
