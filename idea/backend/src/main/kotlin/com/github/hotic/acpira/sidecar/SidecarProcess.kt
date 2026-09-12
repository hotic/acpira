package com.github.hotic.acpira.sidecar

import com.github.hotic.acpira.Acpira
import com.google.gson.JsonObject
import com.google.gson.JsonParser
import java.io.BufferedWriter
import java.io.OutputStreamWriter
import java.nio.charset.StandardCharsets
import java.nio.file.Path
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean

// One Node sidecar process: ndjson envelopes on stdout (one JsonObject per line; anything else is logged and skipped), the sidecar's
// log on stderr, our envelopes on stdin. Reader threads hand parsed envelopes to `onEnvelope`; `onExit` fires once, however it ended
class SidecarProcess(
    node: Path,
    script: Path,
    cwd: Path?,
    private val onEnvelope: (JsonObject) -> Unit,
    private val onExit: (code: Int) -> Unit,
) {
    private val process: Process
    private val writer: BufferedWriter
    private val exited = AtomicBoolean(false)
    val pid: Long get() = process.pid()
    val alive: Boolean get() = process.isAlive

    init {
        val pb = ProcessBuilder(node.toString(), script.toString())
        cwd?.let { pb.directory(it.toFile()) }
        pb.environment().putAll(NodeLocator.shellEnv())
        // The sidecar honours ACPIRA_HOME itself; nothing else of ours goes into its environment
        process = pb.start()
        writer = BufferedWriter(OutputStreamWriter(process.outputStream, StandardCharsets.UTF_8))
        thread("acpira-sidecar-stdout-$pid") {
            process.inputStream.bufferedReader(StandardCharsets.UTF_8).useLines { lines ->
                for (line in lines) {
                    if (line.isBlank()) continue
                    val parsed = runCatching { JsonParser.parseString(line) }.getOrNull()
                    if (parsed == null || !parsed.isJsonObject) { Acpira.LOG.warn("sidecar stdout is not an envelope: ${line.take(200)}"); continue }
                    try { onEnvelope(parsed.asJsonObject) } catch (e: Throwable) { Acpira.LOG.error("sidecar envelope handler failed", e) }
                }
            }
        }
        thread("acpira-sidecar-stderr-$pid") {
            process.errorStream.bufferedReader(StandardCharsets.UTF_8).useLines { lines -> lines.forEach { Acpira.LOG.info("sidecar: $it") } }
        }
        process.onExit().thenAccept { p -> if (exited.compareAndSet(false, true)) onExit(p.exitValue()) }
    }

    @Synchronized
    fun send(envelope: JsonObject) {
        if (!process.isAlive) return
        try {
            writer.write(envelope.toString())
            writer.write("\n")
            writer.flush()
        } catch (e: Exception) {
            Acpira.LOG.warn("sidecar stdin write failed: $e")
        }
    }

    // Ask nicely, then insist: shutdown → wait for exit → SIGTERM → SIGKILL
    fun stop(graceMs: Long = 3000) {
        if (!process.isAlive) return
        send(JsonObject().apply { addProperty("type", "shutdown") })
        if (process.waitFor(graceMs, TimeUnit.MILLISECONDS)) return
        process.destroy()
        if (process.waitFor(2000, TimeUnit.MILLISECONDS)) return
        process.destroyForcibly()
    }

    private fun thread(name: String, body: () -> Unit) = Thread({ runCatching(body).onFailure { Acpira.LOG.warn("$name ended: $it") } }, name).apply { isDaemon = true; start() }
}
