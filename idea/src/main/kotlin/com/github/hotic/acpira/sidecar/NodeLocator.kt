package com.github.hotic.acpira.sidecar

import com.github.hotic.acpira.Acpira
import com.intellij.util.EnvironmentUtil
import java.io.File
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.Paths
import java.util.concurrent.TimeUnit

const val MIN_NODE_MAJOR = 22

class SidecarSetupException(message: String) : Exception(message)

// Where the sidecar comes from. Node: ACPIRA_NODE, then the runtime a per-platform plugin distribution carries at <plugin>/node/node,
// then the IDE's login-shell PATH (EnvironmentUtil, never System.getenv alone: a Dock launch has no shell PATH). Script:
// ACPIRA_HOST_SERVER for development against a repository build, otherwise the host-server.cjs packaged next to the plugin
object NodeLocator {
    fun shellEnv(): Map<String, String> = EnvironmentUtil.getEnvironmentMap()

    fun node(): Path {
        env("ACPIRA_NODE")?.let { return checked(Paths.get(it), "ACPIRA_NODE") }
        bundledNode()?.let { bundled ->
            // A runtime that does not start on this machine (wrong arch in a hand-copied plugin dir, blocked binary) must not take the
            // sidecar down when a system Node would do
            runCatching { checked(bundled, "bundled") }.onFailure { Acpira.LOG.warn("bundled node unusable, trying the shell PATH: ${it.message}") }
                .getOrNull()?.let { return it }
        }
        val onPath = findInPath("node") ?: throw SidecarSetupException(
            "Node.js $MIN_NODE_MAJOR+ was not found on the shell PATH. Install it (https://nodejs.org) or set ACPIRA_NODE to the executable.",
        )
        return checked(onPath, "PATH")
    }

    // The runtime of a -<os>-<arch> distribution; the plain zip has none. The IDE's plugin installer keeps zip permissions, but a copy
    // that lost the bit (a manual unpack) gets it back rather than an error
    private fun bundledNode(): Path? {
        val dir = Acpira.descriptor?.pluginPath?.resolve("node") ?: return null
        val exe = if (isWindows) "node.exe" else "node"
        val path = dir.resolve(exe)
        if (!Files.isRegularFile(path)) return null
        if (!Files.isExecutable(path)) runCatching { path.toFile().setExecutable(true, false) }
        return path
    }

    private fun checked(path: Path, source: String): Path {
        if (!Files.isExecutable(path)) throw SidecarSetupException("Node.js executable is not runnable: $path")
        val version = version(path)
        val major = version.removePrefix("v").substringBefore('.').toIntOrNull() ?: 0
        if (major < MIN_NODE_MAJOR) throw SidecarSetupException("Node.js $MIN_NODE_MAJOR+ is required, found $version at $path")
        Acpira.LOG.info("sidecar node ($source): $path ($version)")
        return path
    }

    private val isWindows get() = System.getProperty("os.name").lowercase().contains("win")

    fun script(): Path {
        env("ACPIRA_HOST_SERVER")?.let { override ->
            val p = Paths.get(override)
            if (!Files.isRegularFile(p)) throw SidecarSetupException("ACPIRA_HOST_SERVER points to a missing file: $p")
            return p
        }
        val bundled = Acpira.descriptor?.pluginPath?.resolve("sidecar/host-server.cjs")
        if (bundled == null || !Files.isRegularFile(bundled)) throw SidecarSetupException(
            "The plugin's sidecar (sidecar/host-server.cjs) is missing; reinstall the plugin or set ACPIRA_HOST_SERVER to a repository build.",
        )
        return bundled
    }

    // First executable of that name on the shell PATH (PathEnvironmentVariableUtil.findInPath is scheduled for removal)
    fun findInPath(name: String): Path? {
        val names = if (isWindows) listOf("$name.exe", "$name.cmd", name) else listOf(name)
        return (shellEnv()["PATH"] ?: "").split(File.pathSeparator).asSequence()
            .filter { it.isNotBlank() }
            .flatMap { dir -> names.asSequence().map { Paths.get(dir, it) } }
            .firstOrNull { Files.isRegularFile(it) && Files.isExecutable(it) }
    }

    private fun env(name: String): String? = (System.getenv(name) ?: shellEnv()[name])?.trim()?.takeIf { it.isNotEmpty() }

    private fun version(node: Path): String {
        val pb = ProcessBuilder(node.toString(), "--version").redirectErrorStream(true)
        pb.environment().putAll(shellEnv())
        val p = pb.start()
        val out = p.inputStream.bufferedReader().readText().trim()
        if (!p.waitFor(10, TimeUnit.SECONDS)) { p.destroyForcibly(); throw SidecarSetupException("node --version did not answer") }
        return out.lines().lastOrNull { it.startsWith("v") } ?: throw SidecarSetupException("node --version answered '$out'")
    }
}
