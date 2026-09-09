package com.github.hotic.acpira.sidecar

import com.github.hotic.acpira.Acpira
import com.github.hotic.acpira.platform.IdePlatform
import com.github.hotic.acpira.settings.AcpiraSettings
import com.github.hotic.acpira.web.AcpiraScheme
import com.google.gson.JsonArray
import com.google.gson.JsonElement
import com.google.gson.JsonObject
import com.intellij.DynamicBundle
import com.intellij.openapi.Disposable
import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.components.Service
import com.intellij.openapi.components.service
import com.intellij.openapi.project.Project
import com.intellij.util.concurrency.AppExecutorUtil
import java.nio.file.Path
import java.nio.file.Paths
import java.util.concurrent.ConcurrentHashMap
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean

// Protocol version of src/shared/sidecar.ts; a sidecar built for another version is rejected at hello, never guessed around
const val SIDECAR_PROTOCOL_VERSION = 1

// One sidecar per project window: the Node host runs with the project folder as its cwd (sessions belong to a project), the views of
// this window (tool window, later editor tabs) attach to it. Envelopes before hello completes wait in an outbox; a crash restarts the
// process with backoff and re-attaches every view on the session it was showing
@Service(Service.Level.PROJECT)
class SidecarService(private val project: Project) : Disposable {
    enum class State { STARTING, READY, FAILED, STOPPED }

    interface View {
        val viewId: String
        val host: String
        // Where the view opens: a session id, `{ mostRecent: true }`, or null for a fresh session
        val initial: JsonElement?
        // The session the view is currently showing, kept by the view from the host messages it received; a restart re-attaches on it
        val lastSessionId: String?
        fun onHostMessage(message: JsonElement)
        fun onState(state: State, detail: String?)
    }

    private val views = ConcurrentHashMap<String, View>()
    private val outbox = CopyOnWriteArrayList<JsonObject>()
    private val platform = IdePlatform(project, this)
    private val settings = AcpiraSettings.getInstance()
    private val unsubscribeSettings: () -> Unit
    private var process: SidecarProcess? = null
    @Volatile private var ready = false
    @Volatile var state = State.STOPPED
        private set
    @Volatile var stateDetail: String? = null
        private set
    private val disposed = AtomicBoolean(false)
    private var restarts = 0
    private var lastStart = 0L
    private var helloSeq = 0

    // Attachment blobs are read from here by the resource handler; the sidecar reports it in helloOk
    @Volatile var sessionsDir: Path? = null
        private set

    init {
        unsubscribeSettings = settings.subscribe { keys, snapshot ->
            send(JsonObject().apply {
                addProperty("type", "platformEvent")
                add("event", JsonObject().apply {
                    addProperty("type", "settingsChanged")
                    add("keys", JsonArray().apply { keys.forEach { add(it) } })
                    add("settings", snapshot)
                })
            })
        }
    }

    // Spawns the sidecar on a pooled thread (locating Node runs `node --version`); safe to call again after a failure
    fun start() {
        if (disposed.get() || process?.alive == true) return
        setState(State.STARTING, null)
        lastStart = System.currentTimeMillis()
        AppExecutorUtil.getAppExecutorService().execute {
            try {
                val node = NodeLocator.node()
                val script = NodeLocator.script()
                synchronized(this) {
                    if (disposed.get()) return@execute
                    process = SidecarProcess(node, script, project.basePath?.let { Paths.get(it) }, ::onEnvelope, ::onExit)
                }
                Acpira.LOG.info("sidecar started: pid ${process?.pid}, script $script")
                hello()
            } catch (e: SidecarSetupException) {
                Acpira.LOG.warn("sidecar cannot start: ${e.message}")
                setState(State.FAILED, e.message)
            } catch (e: Exception) {
                Acpira.LOG.error("sidecar failed to start", e)
                setState(State.FAILED, e.toString())
            }
        }
    }

    private fun hello() {
        val env = JsonObject().apply {
            project.basePath?.let { addProperty("cwd", it) }
            addProperty("hostLanguage", DynamicBundle.getLocale().toLanguageTag())
            addProperty("blobBase", "${Acpira.ORIGIN}/blobs")
        }
        val client = JsonObject().apply {
            addProperty("name", "intellij")
            addProperty("version", Acpira.version)
            add("capabilities", JsonArray().apply { platform.capabilities.forEach { add(it) } })
        }
        process?.send(JsonObject().apply {
            addProperty("type", "hello")
            addProperty("protocolVersion", SIDECAR_PROTOCOL_VERSION)
            addProperty("requestId", "hello-${++helloSeq}")
            add("client", client)
            add("env", env)
            add("settings", settings.snapshot())
        })
    }

    fun attach(view: View) {
        views[view.viewId] = view
        view.onState(state, stateDetail)
        if (ready) send(attachEnvelope(view))
        if (process == null && !disposed.get()) start()
    }

    fun detach(viewId: String) {
        if (views.remove(viewId) == null) return
        outbox.removeIf { it.get("viewId")?.asString == viewId }
        if (ready) send(JsonObject().apply { addProperty("type", "detachView"); addProperty("viewId", viewId) })
    }

    fun webviewMessage(viewId: String, message: JsonElement) {
        send(JsonObject().apply { addProperty("type", "webviewMessage"); addProperty("viewId", viewId); add("message", message) })
    }

    fun windowFocus() {
        if (ready) send(JsonObject().apply { addProperty("type", "platformEvent"); add("event", JsonObject().apply { addProperty("type", "windowFocus") }) })
    }

    fun respond(requestId: String, result: JsonElement?, error: String?) {
        send(JsonObject().apply {
            addProperty("type", "platformResponse")
            addProperty("requestId", requestId)
            if (error != null) addProperty("error", error) else add("result", result)
        })
    }

    // Anything but hello waits for helloOk; the outbox keeps order
    private fun send(envelope: JsonObject) {
        if (disposed.get()) return
        if (!ready) { outbox.add(envelope); return }
        process?.send(envelope)
    }

    private fun attachEnvelope(view: View) = JsonObject().apply {
        addProperty("type", "attachView")
        addProperty("viewId", view.viewId)
        addProperty("host", view.host)
        val session = view.lastSessionId
        when {
            session != null -> addProperty("initial", session)
            view.initial != null -> add("initial", view.initial)
        }
    }

    private fun onEnvelope(m: JsonObject) {
        when (m.get("type")?.asString) {
            "helloOk" -> {
                sessionsDir = m.get("sessionsDir")?.asString?.let { Paths.get(it) }
                // Every project's sidecar shares ACPIRA_HOME, so the one resource handler serves blobs from the same directory
                AcpiraScheme.sessionsDir = sessionsDir
                ready = true
                restarts = 0
                // Views first (in attach order), then whatever the views already posted; a view's ready waits behind its attach
                for (view in views.values) process?.send(attachEnvelope(view))
                val queued = outbox.toList()
                outbox.clear()
                for (e in queued) process?.send(e)
                setState(State.READY, null)
            }
            "helloReject" -> {
                val reason = m.get("reason")?.asString ?: "rejected"
                Acpira.LOG.warn("sidecar rejected hello: $reason")
                setState(State.FAILED, "The sidecar does not speak protocol $SIDECAR_PROTOCOL_VERSION: $reason")
                process?.stop(500)
            }
            "hostMessage" -> {
                val view = views[m.get("viewId")?.asString] ?: return
                m.get("message")?.let { view.onHostMessage(it) }
            }
            "platformRequest" -> platform.handle(m.get("requestId")?.takeIf { !it.isJsonNull }?.asString, m.getAsJsonObject("request"))
            "shutdownOk" -> {}
            else -> Acpira.LOG.warn("sidecar sent an unknown envelope: ${m.get("type")}")
        }
    }

    private fun onExit(code: Int) {
        ready = false
        synchronized(this) { process = null }
        if (disposed.get() || state == State.FAILED) return
        Acpira.LOG.warn("sidecar exited with code $code")
        // Exponential backoff, giving up after a burst of failures; a view's Retry starts over
        val uptime = System.currentTimeMillis() - lastStart
        restarts = if (uptime > 60_000) 1 else restarts + 1
        if (restarts > 5) {
            setState(State.FAILED, "The sidecar keeps exiting (last code $code); see the IDE log.")
            return
        }
        val delay = minOf(30_000L, 1000L shl (restarts - 1))
        setState(State.STARTING, "Sidecar exited (code $code), restarting in ${delay / 1000}s…")
        AppExecutorUtil.getAppScheduledExecutorService().schedule({ if (!disposed.get() && process == null) start() }, delay, TimeUnit.MILLISECONDS)
    }

    private fun setState(s: State, detail: String?) {
        state = s
        stateDetail = detail
        val snapshot = views.values.toList()
        ApplicationManager.getApplication().invokeLater { for (v in snapshot) v.onState(s, detail) }
    }

    override fun dispose() {
        if (!disposed.compareAndSet(false, true)) return
        unsubscribeSettings()
        ready = false
        val p = synchronized(this) { process.also { process = null } }
        AppExecutorUtil.getAppExecutorService().execute { p?.stop() }
    }

    companion object {
        fun getInstance(project: Project): SidecarService = project.service()
    }
}
