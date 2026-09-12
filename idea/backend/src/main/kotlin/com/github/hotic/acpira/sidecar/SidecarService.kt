package com.github.hotic.acpira.sidecar

import com.github.hotic.acpira.Acpira
import com.github.hotic.acpira.platform.IdePlatform
import com.github.hotic.acpira.rpc.SidecarState
import com.github.hotic.acpira.settings.AcpiraSettings
import com.google.gson.JsonArray
import com.google.gson.JsonElement
import com.google.gson.JsonObject
import com.intellij.DynamicBundle
import com.intellij.platform.ide.productMode.IdeProductMode
import com.intellij.openapi.Disposable
import com.intellij.openapi.components.Service
import com.intellij.openapi.components.service
import com.intellij.openapi.project.Project
import com.intellij.util.concurrency.AppExecutorUtil
import java.nio.file.Path
import java.nio.file.Paths
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicInteger

// Protocol version of src/shared/sidecar.ts; a sidecar built for another version is rejected at hello, never guessed around
const val SIDECAR_PROTOCOL_VERSION = 1

// One sidecar per project window: the Node host runs with the project folder as its cwd (sessions belong to a project), the views of
// this window (tool window, later editor tabs) attach to it. Envelopes before hello completes wait in an outbox; a crash restarts the
// process with backoff and re-attaches every view on the session it was showing.
//
// Start, handshake, attach, outbox drain, and ordinary sends share one single-thread executor so a second view cannot spawn a second
// process during Node lookup, and a page's one-shot ready cannot overtake attachView. Each process has a generation; stale stdout and
// exit callbacks from a replaced process cannot mutate the new one
@Service(Service.Level.PROJECT)
class SidecarService(private val project: Project) : Disposable {
    interface View {
        val viewId: String
        val host: String
        // Where the view opens: a session id, `{ mostRecent: true }`, or null for a fresh session
        val initial: JsonElement?
        val locale: String
        // The session the view is currently showing, kept by the view from the host messages it received; a restart re-attaches on it
        val lastSessionId: String?
        fun onHostMessage(message: JsonElement)
        fun onState(state: SidecarState, detail: String?)
    }

    private val io = AppExecutorUtil.createBoundedApplicationPoolExecutor("AcpiraSidecar", AppExecutorUtil.getAppExecutorService(), 1, this)
    private val views = LinkedHashMap<String, View>()
    private val outbox = SidecarOutbox()
    private val platform = IdePlatform(project, this)
    private val settings = AcpiraSettings.getInstance()
    private val unsubscribeSettings: () -> Unit
    private var process: SidecarProcess? = null
    private var starting = false
    private val generation = AtomicInteger(0)
    @Volatile var state = SidecarState.STOPPED
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

    private fun enqueue(task: () -> Unit) {
        if (disposed.get()) return
        io.execute {
            if (disposed.get()) return@execute
            try { task() } catch (t: Throwable) { Acpira.LOG.error("sidecar io failed", t) }
        }
    }

    // Spawns the sidecar on the serial executor (locating Node runs `node --version`); safe to call again after a failure
    fun start() { enqueue { startOnIo() } }

    // Retry from the status panel: the consecutive-failure counter is only cleared here, not on helloOk
    fun retry() {
        enqueue {
            restarts = 0
            startOnIo()
        }
    }

    private fun startOnIo() {
        if (disposed.get() || process?.alive == true || starting) return
        starting = true
        val gen = generation.incrementAndGet()
        setState(SidecarState.STARTING, null)
        lastStart = System.currentTimeMillis()
        try {
            val node = NodeLocator.node()
            val script = NodeLocator.script()
            if (disposed.get() || gen != generation.get()) {
                starting = false
                return
            }
            process = SidecarProcess(
                node, script, project.basePath?.let { Paths.get(it) },
                { env -> enqueue { onEnvelope(gen, env) } },
                { code -> enqueue { onExit(gen, code) } },
            )
            starting = false
            Acpira.LOG.info("sidecar started: pid ${process?.pid}, script $script, cwd ${project.basePath}, remoteDevHost=${IdeProductMode.isBackend}")
            hello()
        } catch (e: SidecarSetupException) {
            if (gen != generation.get()) return
            starting = false
            Acpira.LOG.warn("sidecar cannot start: ${e.message}")
            setState(SidecarState.FAILED, e.message)
        } catch (e: Exception) {
            if (gen != generation.get()) return
            starting = false
            Acpira.LOG.error("sidecar failed to start", e)
            setState(SidecarState.FAILED, e.toString())
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
        enqueue {
            views[view.viewId] = view
            val s = state
            val d = stateDetail
            view.onState(s, d)
            if (outbox.ready) write(attachEnvelope(view))
            if (process == null && !disposed.get()) startOnIo()
            val backendLocale = DynamicBundle.getLocale().toLanguageTag()
            if (views.size == 1 && view.locale != backendLocale) sendHostLanguage(view.locale)
        }
    }

    fun detach(viewId: String) {
        enqueue {
            if (views.remove(viewId) == null) return@enqueue
            outbox.dropView(viewId)
            if (outbox.ready) write(JsonObject().apply { addProperty("type", "detachView"); addProperty("viewId", viewId) })
        }
    }

    fun webviewMessage(viewId: String, message: JsonElement) {
        send(JsonObject().apply { addProperty("type", "webviewMessage"); addProperty("viewId", viewId); add("message", message) })
    }

    fun windowFocus() {
        send(JsonObject().apply { addProperty("type", "platformEvent"); add("event", JsonObject().apply { addProperty("type", "windowFocus") }) })
    }

    private fun sendHostLanguage(locale: String) {
        write(JsonObject().apply {
            addProperty("type", "platformEvent")
            add("event", JsonObject().apply {
                addProperty("type", "envChanged")
                add("env", JsonObject().apply { addProperty("hostLanguage", locale) })
            })
        })
    }

    fun respond(requestId: String, result: JsonElement?, error: String?) {
        send(JsonObject().apply {
            addProperty("type", "platformResponse")
            addProperty("requestId", requestId)
            if (error != null) addProperty("error", error) else add("result", result)
        })
    }

    private fun send(envelope: JsonObject) { enqueue { write(envelope) } }

    private fun write(envelope: JsonObject) {
        if (disposed.get()) return
        outbox.offer(envelope)?.let { process?.send(it) }
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

    private fun onEnvelope(gen: Int, m: JsonObject) {
        if (gen != generation.get()) return
        when (m.get("type")?.asString) {
            "helloOk" -> {
                sessionsDir = m.get("sessionsDir")?.asString?.let { Paths.get(it) }
                val attaches = views.values.map { attachEnvelope(it) }
                for (e in outbox.flush(attaches)) process?.send(e)
                setState(SidecarState.READY, null)
            }
            "helloReject" -> {
                val reason = m.get("reason")?.asString ?: "rejected"
                Acpira.LOG.warn("sidecar rejected hello: $reason")
                setState(SidecarState.FAILED, "The sidecar does not speak protocol $SIDECAR_PROTOCOL_VERSION: $reason")
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

    private fun onExit(gen: Int, code: Int) {
        if (gen != generation.get()) return
        outbox.reset()
        process = null
        starting = false
        if (disposed.get() || state == SidecarState.FAILED) return
        Acpira.LOG.warn("sidecar exited with code $code")
        // Exponential backoff, giving up after a burst of failures. helloOk does not reset the counter: a process that
        // handshakes and dies immediately still counts. A long-lived run starts a new burst; Retry zeroes it
        val uptime = System.currentTimeMillis() - lastStart
        restarts = if (uptime > 60_000) 1 else restarts + 1
        if (restarts > 5) {
            setState(SidecarState.FAILED, "The sidecar keeps exiting (last code $code); see the IDE log.")
            return
        }
        val delay = minOf(30_000L, 1000L shl (restarts - 1))
        setState(SidecarState.STARTING, "Sidecar exited (code $code), restarting in ${delay / 1000}s…")
        AppExecutorUtil.getAppScheduledExecutorService().schedule({ enqueue { if (!disposed.get() && process == null) startOnIo() } }, delay, TimeUnit.MILLISECONDS)
    }

    private fun setState(s: SidecarState, detail: String?) {
        state = s
        stateDetail = detail
        val snapshot = views.values.toList()
        for (v in snapshot) v.onState(s, detail)
    }

    override fun dispose() {
        if (!disposed.compareAndSet(false, true)) return
        unsubscribeSettings()
        generation.incrementAndGet()
        outbox.reset()
        val p = process
        process = null
        AppExecutorUtil.getAppExecutorService().execute { p?.stop() }
    }

    companion object {
        fun getInstance(project: Project): SidecarService = project.service()
    }
}
