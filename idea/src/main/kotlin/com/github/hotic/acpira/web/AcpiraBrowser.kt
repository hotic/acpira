package com.github.hotic.acpira.web

import com.github.hotic.acpira.Acpira
import com.github.hotic.acpira.sidecar.SidecarService
import com.google.gson.JsonElement
import com.google.gson.JsonParser
import com.intellij.DynamicBundle
import com.intellij.openapi.Disposable
import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.project.Project
import com.intellij.openapi.util.Disposer
import com.intellij.ui.jcef.JBCefBrowser
import com.intellij.ui.jcef.JBCefBrowserBase
import com.intellij.ui.jcef.JBCefJSQuery
import org.cef.CefSettings
import org.cef.browser.CefBrowser
import org.cef.browser.CefFrame
import org.cef.handler.CefDisplayHandlerAdapter
import org.cef.handler.CefLifeSpanHandlerAdapter
import org.cef.handler.CefLoadHandler
import org.cef.handler.CefLoadHandlerAdapter
import org.cef.handler.CefRequestHandlerAdapter
import org.cef.network.CefRequest
import java.security.SecureRandom
import java.util.Base64
import java.util.UUID
import javax.swing.JComponent

// One JCEF view of Acpira: loads the generated page from https://acpira.local, carries the webview's messages up through a JBCefJSQuery
// and the host's messages down as Base64 → JSON.parse → MessageEvent (never JSON spliced into a JS literal), refuses navigation off our
// origin and popups, and re-attaches to the sidecar with the session it was showing after a sidecar restart
class AcpiraBrowser(
    private val project: Project,
    override val host: String,
    override val initial: JsonElement?,
    parent: Disposable,
    private val onState: (SidecarService.State, String?) -> Unit,
) : SidecarService.View, Disposable {
    private val token = UUID.randomUUID().toString()
    override val viewId = "view-$token"
    @Volatile override var lastSessionId: String? = null
        private set
    private val sidecar = SidecarService.getInstance(project)
    private val browser: JBCefBrowser
    private val query: JBCefJSQuery
    @Volatile private var pageLoaded = false
    @Volatile private var wasReady = false
    val component: JComponent get() = browser.component

    init {
        Disposer.register(parent, this)
        AcpiraScheme.ensureRegistered()
        browser = JBCefBrowser.createBuilder().setEnableOpenDevToolsMenuItem(ApplicationManager.getApplication().isInternal).build()
        Disposer.register(this, browser)
        query = JBCefJSQuery.create(browser as JBCefBrowserBase)
        Disposer.register(this, query)
        query.addHandler { raw ->
            val parsed = runCatching { JsonParser.parseString(raw) }.getOrNull()
            if (parsed == null || !parsed.isJsonObject) Acpira.LOG.warn("webview posted something that is not a message: ${raw.take(120)}")
            else sidecar.webviewMessage(viewId, parsed)
            null
        }
        val bridge = """
            window.__acpiraApi = { postMessage(m) { ${query.inject("JSON.stringify(m)")} } };
            window.__acpiraReceive = function (b64) {
              const bytes = Uint8Array.from(atob(b64), c => c.charCodeAt(0));
              window.dispatchEvent(new MessageEvent('message', { data: JSON.parse(new TextDecoder().decode(bytes)) }));
            };
        """.trimIndent()
        val nonce = ByteArray(16).also { SecureRandom().nextBytes(it) }.let { Base64.getUrlEncoder().withoutPadding().encodeToString(it) }
        AcpiraScheme.register(token, ViewPage(host, bridge, nonce, ThemeVars.current(), DynamicBundle.getLocale().toLanguageTag()))

        val client = browser.jbCefClient
        client.addRequestHandler(object : CefRequestHandlerAdapter() {
            override fun onBeforeBrowse(b: CefBrowser?, frame: CefFrame?, request: CefRequest?, userGesture: Boolean, isRedirect: Boolean): Boolean {
                val url = request?.url ?: return false
                if (url.startsWith("${Acpira.ORIGIN}/")) return false
                Acpira.LOG.info("navigation refused: ${url.take(120)}")
                return true
            }
        }, browser.cefBrowser)
        client.addLifeSpanHandler(object : CefLifeSpanHandlerAdapter() {
            override fun onBeforePopup(b: CefBrowser?, frame: CefFrame?, targetUrl: String?, targetFrameName: String?): Boolean {
                Acpira.LOG.info("popup refused: ${targetUrl?.take(120)}")
                return true
            }
        }, browser.cefBrowser)
        client.addDisplayHandler(object : CefDisplayHandlerAdapter() {
            override fun onConsoleMessage(b: CefBrowser?, level: CefSettings.LogSeverity?, message: String?, source: String?, line: Int): Boolean {
                val text = "webview console [$level] $message ($source:$line)"
                if (level == CefSettings.LogSeverity.LOGSEVERITY_ERROR || level == CefSettings.LogSeverity.LOGSEVERITY_FATAL) Acpira.LOG.warn(text) else Acpira.LOG.debug(text)
                return false
            }
        }, browser.cefBrowser)
        client.addLoadHandler(object : CefLoadHandlerAdapter() {
            override fun onLoadEnd(b: CefBrowser?, frame: CefFrame?, httpStatusCode: Int) { if (frame?.isMain == true) pageLoaded = true }
            override fun onLoadError(b: CefBrowser?, frame: CefFrame?, errorCode: CefLoadHandler.ErrorCode?, errorText: String?, failedUrl: String?) {
                // ERR_ABORTED is the refused navigation above; anything else on our own origin is a real problem
                if (errorCode != CefLoadHandler.ErrorCode.ERR_ABORTED) Acpira.LOG.warn("webview load error $errorCode $errorText at $failedUrl")
            }
        }, browser.cefBrowser)

        sidecar.attach(this)
        browser.loadURL("${Acpira.ORIGIN}/index.html?view=$token")
    }

    override fun onHostMessage(message: JsonElement) {
        if (message.isJsonObject) {
            val o = message.asJsonObject
            when (o.get("type")?.asString) {
                "session" -> o.getAsJsonObject("session")?.get("id")?.asString?.let { lastSessionId = it }
                "init" -> o.getAsJsonObject("state")?.getAsJsonObject("active")?.get("id")?.asString?.let { lastSessionId = it }
            }
        }
        val b64 = Base64.getEncoder().encodeToString(message.toString().toByteArray())
        browser.cefBrowser.executeJavaScript("window.__acpiraReceive && window.__acpiraReceive('$b64')", browser.cefBrowser.url, 0)
    }

    // A sidecar that came back after the page had already initialized needs the page to start over (it posts `ready` once, at mount)
    override fun onState(state: SidecarService.State, detail: String?) {
        if (state == SidecarService.State.READY) {
            if (wasReady && pageLoaded) browser.cefBrowser.reload()
            wasReady = true
        }
        onState.invoke(state, detail)
    }

    override fun dispose() {
        sidecar.detach(viewId)
        AcpiraScheme.unregister(token)
    }
}
