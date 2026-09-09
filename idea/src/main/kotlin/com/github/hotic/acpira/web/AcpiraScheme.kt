package com.github.hotic.acpira.web

import com.github.hotic.acpira.Acpira
import com.intellij.ui.jcef.JBCefApp
import org.cef.CefApp
import org.cef.browser.CefBrowser
import org.cef.browser.CefFrame
import org.cef.callback.CefCallback
import org.cef.callback.CefSchemeHandlerFactory
import org.cef.handler.CefResourceHandler
import org.cef.handler.CefResourceHandlerAdapter
import org.cef.misc.IntRef
import org.cef.misc.StringRef
import org.cef.network.CefRequest
import org.cef.network.CefResponse
import java.net.URI
import java.net.URLDecoder
import java.nio.charset.StandardCharsets
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.Paths
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.atomic.AtomicBoolean

// One page configuration per view: the HTML the handler generates is specific to the browser that asked (its JSQuery bridge)
class ViewPage(val host: String, val bridgeJs: String, val nonce: String, val theme: ThemeVars, val locale: String)

// Serves https://acpira.local: the generated page (per view token), the webview bundle from plugin resources, and attachment blobs from
// the sessions directory. Registered once on the running CefApp. Every path is URL-decoded, normalized and confined to its root before it
// is read; blobs are additionally checked on the real (symlink-resolved) path so a link out of the sessions directory is refused too
object AcpiraScheme {
    const val HOST = "acpira.local"
    private val registered = AtomicBoolean(false)
    private val pages = ConcurrentHashMap<String, ViewPage>()
    @Volatile var sessionsDir: Path? = null

    fun ensureRegistered() {
        if (!registered.compareAndSet(false, true)) return
        JBCefApp.getInstance()
        val ok = CefApp.getInstance().registerSchemeHandlerFactory("https", HOST, Factory)
        Acpira.LOG.info("resource handler for ${Acpira.ORIGIN}: $ok")
    }

    fun register(token: String, page: ViewPage) { pages[token] = page }
    fun unregister(token: String) { pages.remove(token) }

    sealed class Resolution {
        data class Status(val code: Int) : Resolution()
        class Bytes(val bytes: ByteArray, val mime: String) : Resolution()
    }

    private object Factory : CefSchemeHandlerFactory {
        override fun create(browser: CefBrowser?, frame: CefFrame?, schemeName: String?, request: CefRequest?): CefResourceHandler {
            val url = request?.url ?: return StatusHandler(400)
            return when (val r = resolve(url)) {
                is Resolution.Status -> StatusHandler(r.code)
                is Resolution.Bytes -> BytesHandler(r.bytes, r.mime)
            }
        }
    }

    // Session ids are UUIDs and blob names content hashes; anything else in a blob URL is refused before the file system is asked
    private val SAFE_SEGMENT = Regex("^[A-Za-z0-9._-]+$")

    fun resolve(url: String): Resolution {
        val uri = runCatching { URI(url) }.getOrNull() ?: return Resolution.Status(400)
        if (uri.host != HOST) return Resolution.Status(404)
        val path = URLDecoder.decode(uri.rawPath ?: "/", StandardCharsets.UTF_8)
        if (path.contains('\u0000')) return Resolution.Status(400)
        if (path == "/index.html" || path == "/") {
            val token = uri.rawQuery?.split('&')?.firstOrNull { it.startsWith("view=") }?.removePrefix("view=")
            val page = token?.let { pages[it] } ?: return Resolution.Status(404)
            return Resolution.Bytes(PageTemplate.render(page).toByteArray(StandardCharsets.UTF_8), "text/html")
        }
        if (path.startsWith("/webview/")) {
            val rel = confine(path.removePrefix("/webview/")) ?: return Resolution.Status(403)
            val bytes = AcpiraScheme::class.java.classLoader.getResourceAsStream("webview/$rel")?.use { it.readBytes() } ?: return Resolution.Status(404)
            return Resolution.Bytes(bytes, mime(rel))
        }
        if (path.startsWith("/blobs/")) {
            val parts = path.removePrefix("/blobs/").split('/')
            if (parts.size != 2 || parts.any { !SAFE_SEGMENT.matches(it) || it == "." || it == ".." }) return Resolution.Status(403)
            val root = sessionsDir ?: return Resolution.Status(404)
            val file = root.resolve(parts[0]).resolve(parts[1])
            val real = runCatching { file.toRealPath() }.getOrNull() ?: return Resolution.Status(404)
            val rootReal = runCatching { root.toRealPath() }.getOrNull() ?: return Resolution.Status(404)
            if (!real.startsWith(rootReal) || !Files.isRegularFile(real)) return Resolution.Status(403)
            return Resolution.Bytes(Files.readAllBytes(real), mime(parts[1]))
        }
        return Resolution.Status(404)
    }

    // Normalize and keep inside the resources root: `..`, `%2e%2e`, `%2f` all end up rejected
    private fun confine(rel: String): String? {
        if (rel.isEmpty()) return null
        val normalized = Paths.get("/$rel").normalize()
        if (normalized.nameCount == 0 || normalized.toString() == "/") return null
        return normalized.toString().removePrefix("/")
    }

    fun mime(name: String) = when (name.substringAfterLast('.', "").lowercase()) {
        "html" -> "text/html"
        "js", "mjs" -> "text/javascript"
        "css" -> "text/css"
        "json" -> "application/json"
        "svg" -> "image/svg+xml"
        "png" -> "image/png"
        "jpg", "jpeg" -> "image/jpeg"
        "gif" -> "image/gif"
        "webp" -> "image/webp"
        "woff2" -> "font/woff2"
        "woff" -> "font/woff"
        "ttf" -> "font/ttf"
        "wasm" -> "application/wasm"
        "txt", "md" -> "text/plain"
        else -> "application/octet-stream"
    }
}

private class StatusHandler(private val status: Int) : CefResourceHandlerAdapter() {
    override fun processRequest(request: CefRequest?, callback: CefCallback?): Boolean { callback?.Continue(); return true }
    override fun getResponseHeaders(response: CefResponse?, responseLength: IntRef?, redirectUrl: StringRef?) {
        response?.status = status
        response?.mimeType = "text/plain"
        responseLength?.set(0)
    }
    override fun readResponse(dataOut: ByteArray?, bytesToRead: Int, bytesRead: IntRef?, callback: CefCallback?): Boolean { bytesRead?.set(0); return false }
}

private class BytesHandler(private val bytes: ByteArray, private val mime: String) : CefResourceHandlerAdapter() {
    private var offset = 0
    override fun processRequest(request: CefRequest?, callback: CefCallback?): Boolean { callback?.Continue(); return true }
    override fun getResponseHeaders(response: CefResponse?, responseLength: IntRef?, redirectUrl: StringRef?) {
        response?.status = 200
        response?.mimeType = mime
        response?.setHeaderByName("Cache-Control", "no-store", true)
        responseLength?.set(bytes.size)
    }
    override fun readResponse(dataOut: ByteArray?, bytesToRead: Int, bytesRead: IntRef?, callback: CefCallback?): Boolean {
        if (dataOut == null || offset >= bytes.size) { bytesRead?.set(0); return false }
        val n = minOf(bytesToRead, bytes.size - offset)
        System.arraycopy(bytes, offset, dataOut, 0, n)
        offset += n
        bytesRead?.set(n)
        return true
    }
}
