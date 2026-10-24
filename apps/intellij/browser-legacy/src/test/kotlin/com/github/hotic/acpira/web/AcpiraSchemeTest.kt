package com.github.hotic.acpira.web

import com.github.hotic.acpira.web.AcpiraScheme.Resolution
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test

class AcpiraSchemeTest {
    @Before
    fun setUp() {
        AcpiraScheme.register("tok", ViewPage("sidebar", "/* bridge */", "n0nce", ThemeVars(true, "#ccc", "#181818", "#1f1f1f", "#333", "#07d", "Menlo"), "en"))
    }

    @After
    fun tearDown() {
        AcpiraScheme.unregister("tok")
    }

    private fun status(url: String) = (AcpiraScheme.resolve(url) as Resolution.Status).code

    @Test fun `routes a safe blob URL to backend RPC with its mime type`() {
        val r = AcpiraScheme.resolve("https://acpira.local/blobs/11111111-2222-3333-4444-555555555555/abcdef0123456789.png") as Resolution.Blob
        assertEquals("11111111-2222-3333-4444-555555555555", r.sessionId)
        assertEquals("abcdef0123456789.png", r.name)
        assertEquals("image/png", r.mime)
    }

    @Test fun `refuses every malformed blob route`() {
        assertEquals(403, status("https://acpira.local/blobs/../accounts.json"))
        assertEquals(403, status("https://acpira.local/blobs/..%2f..%2faccounts.json"))
        assertEquals(403, status("https://acpira.local/blobs/session/../accounts.json"))
        assertEquals(403, status("https://acpira.local/blobs/session/%2e%2e/accounts.json"))
        assertEquals(403, status("https://acpira.local/blobs/session"))
        assertEquals(403, status("https://acpira.local/blobs/session/a/b"))
    }

    @Test fun `serves webview resources only from within the bundle`() {
        assertEquals(403, status("https://acpira.local/webview/../META-INF/plugin.xml"))
        assertEquals(403, status("https://acpira.local/webview/%2e%2e/META-INF/plugin.xml"))
        assertEquals(403, status("https://acpira.local/webview/"))
        assertEquals(404, status("https://acpira.local/webview/not-there.js"))
    }

    @Test fun `resource paths stay slash-separated so Windows cannot turn main js into a backslash lookup`() {
        assertEquals("main.js", AcpiraScheme.confine("main.js"))
        assertEquals("assets/app.js", AcpiraScheme.confine("assets/app.js"))
        assertEquals("assets/app.js", AcpiraScheme.confine("foo/../assets/app.js"))
        assertEquals("assets/app.js", AcpiraScheme.confine("foo\\..\\assets\\app.js"))
        assertEquals(null, AcpiraScheme.confine("../main.js"))
        assertEquals(null, AcpiraScheme.confine("..\\main.js"))
        assertEquals(null, AcpiraScheme.confine(""))
        assertEquals(null, AcpiraScheme.confine("."))
    }

    @Test fun `an existing main js is served with a javascript mime type`() {
        val js = AcpiraScheme.resolve("https://acpira.local/webview/main.js") as Resolution.Bytes
        assertEquals("text/javascript", js.mime)
        assertTrue(js.bytes.isNotEmpty())
        val css = AcpiraScheme.resolve("https://acpira.local/webview/main.css") as Resolution.Bytes
        assertEquals("text/css", css.mime)
        assertTrue(css.bytes.isNotEmpty())
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
