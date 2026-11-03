package com.github.hotic.acpira.sidecar

import com.github.hotic.acpira.Acpira
import com.google.gson.JsonObject
import com.google.gson.JsonParser
import java.io.IOException
import java.io.InputStream
import java.net.UnixDomainSocketAddress
import java.nio.ByteBuffer
import java.nio.channels.SocketChannel
import java.nio.charset.StandardCharsets
import java.nio.file.Path
import java.nio.file.Paths
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicReference

// The machine's one engine (per OS user and data root, rust/crates/acpira-host/src/machine_engine/), shared with every IDE window, VS Code
// and the desktop app: sessions run there, so closing the project or the IDE only disconnects. Mirrors reachMachineEngine in
// packages/engine-client/engine.ts and reach_machine_engine in rust/crates/acpira-desktop/src/engine.rs
object MachineEngine {
    // How long `acpira engine` may take: landing a large build, an older engine handing over, a fresh start (its own limit is 40 s)
    private const val ENSURE_WAIT_MS = 45_000L

    fun wanted(): Boolean = wanted(ShellEnv::get, System.getProperty("os.name"))

    // macOS and Linux reach it over a Unix socket; Windows keeps the child-process sidecar. ACPIRA_ENGINE=stdio opts out anywhere (debugging)
    internal fun wanted(env: (String) -> String?, osName: String): Boolean =
        !osName.startsWith("Windows", ignoreCase = true) && env("ACPIRA_ENGINE") != "stdio"

    // `<binary> engine --home H` makes sure the engine runs (and is at least this build) and prints where it listens. Blocks for up to
    // ENSURE_WAIT_MS; its stderr goes to the IDE log. Throws SidecarSetupException with the engine's own reason when it fails
    fun reach(command: SidecarCommand, home: Path, cwd: Path?): Path {
        // The subcommand goes first: the binary reads it from the first argument
        val argv = listOf(command.argv[0], "engine", "--home", home.toString()) + command.argv.drop(1)
        val pb = ProcessBuilder(argv)
        cwd?.let { pb.directory(it.toFile()) }
        pb.environment().putAll(ShellEnv.map())
        val process = pb.start()
        process.outputStream.close()
        val stdout = StringBuilder()
        val lastErr = AtomicReference("")
        val out = daemonThread("acpira-engine-stdout") {
            process.inputStream.bufferedReader(StandardCharsets.UTF_8).useLines { lines -> lines.forEach { synchronized(stdout) { stdout.appendLine(it) } } }
        }
        val err = daemonThread("acpira-engine-stderr") {
            process.errorStream.bufferedReader(StandardCharsets.UTF_8).useLines { lines ->
                lines.map { it.trim() }.filter { it.isNotEmpty() }.forEach { Acpira.LOG.info("engine: $it"); lastErr.set(it) }
            }
        }
        if (!process.waitFor(ENSURE_WAIT_MS, TimeUnit.MILLISECONDS)) {
            process.destroyForcibly()
            throw SidecarSetupException("acpira engine gave no answer within ${ENSURE_WAIT_MS / 1000}s")
        }
        // The pipes close with the process; the bound only guards against a launched engine that inherited them
        out.join(2000)
        err.join(2000)
        if (process.exitValue() != 0) throw SidecarSetupException(lastErr.get().ifEmpty { "acpira engine exited ${process.exitValue()}" })
        val reached = parse(synchronized(stdout) { stdout.toString() })
        Acpira.LOG.info("machine engine ${reached.version} (pid ${reached.pid}) at ${reached.socket}")
        return Paths.get(reached.socket)
    }

    internal data class Reached(val socket: String, val version: String?, val pid: Long?)

    // The last non-empty stdout line: {"socket":…,"version":…,"pid":…}
    internal fun parse(stdout: String): Reached {
        val line = stdout.lines().lastOrNull { it.isNotBlank() }?.trim() ?: ""
        val o = runCatching { JsonParser.parseString(line) }.getOrNull()?.takeIf { it.isJsonObject }?.asJsonObject
        val socket = o?.get("socket")?.takeIf { it.isJsonPrimitive && it.asJsonPrimitive.isString }?.asString
            ?: throw SidecarSetupException("acpira engine printed ${line.ifEmpty { "nothing" }}")
        return Reached(
            socket,
            o.get("version")?.takeIf { it.isJsonPrimitive }?.asString,
            o.get("pid")?.takeIf { it.isJsonPrimitive && it.asJsonPrimitive.isNumber }?.asLong,
        )
    }
}

// One window's connection to the machine's engine: the same envelopes as a sidecar's stdio, over its Unix socket. The engine closing it
// (it ended, or a newer build took the machine over) reports `onExit` like a process exit, so SidecarService reconnects with its backoff
// and runs `acpira engine` again. Reads and writes go straight through the channel: the streams of Channels.newInputStream /
// newOutputStream share the channel's blocking lock, so a write could wait behind a blocked read
class EngineConnection(
    socket: Path,
    private val onEnvelope: (JsonObject) -> Unit,
    private val onExit: (what: String) -> Unit,
) : SidecarLink {
    private val channel = SocketChannel.open(UnixDomainSocketAddress.of(socket))
    private val ended = AtomicBoolean(false)
    override val alive: Boolean get() = channel.isOpen && !ended.get()

    init {
        val input = object : InputStream() {
            override fun read(): Int {
                val one = ByteArray(1)
                return if (read(one, 0, 1) < 0) -1 else one[0].toInt() and 0xff
            }
            override fun read(b: ByteArray, off: Int, len: Int): Int = if (len == 0) 0 else channel.read(ByteBuffer.wrap(b, off, len))
        }
        daemonThread("acpira-engine-connection") {
            try {
                pumpEnvelopes(input, "engine connection", onEnvelope)
            } catch (e: IOException) {
                // A close of our own (stop) ends the read too; only a failure on an open connection is worth a line
                if (channel.isOpen) Acpira.LOG.warn("engine connection: $e")
            } finally {
                runCatching { channel.close() }
                if (ended.compareAndSet(false, true)) onExit("connection closed")
            }
        }
    }

    @Synchronized
    override fun send(envelope: JsonObject) {
        if (!alive) return
        try {
            val buf = StandardCharsets.UTF_8.encode(envelope.toString() + "\n")
            while (buf.hasRemaining()) channel.write(buf)
        } catch (e: Exception) {
            Acpira.LOG.warn("engine connection write failed: $e")
        }
    }

    // Only disconnects: the engine detaches this window's views and keeps its sessions running. The reader then reports the close
    override fun stop(graceMs: Long) {
        runCatching { channel.close() }
    }
}
