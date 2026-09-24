package com.github.hotic.acpira.sidecar

import com.github.hotic.acpira.Acpira
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.Paths

// The process line that starts the sidecar and a label for the log
data class SidecarCommand(val argv: List<String>, val label: String)

// Which sidecar engine runs. The Rust binary speaks the same envelope protocol as host-server.cjs, so the choice is
// invisible above SidecarProcess. Order: ACPIRA_ENGINE=node forces Node; ACPIRA_SIDECAR_BIN names a Rust binary;
// ACPIRA_HOST_SERVER (a repository script) keeps Node; a binary packaged at <plugin>/sidecar/bin/<os>-<arch>/acpira
// (every release package carries one) is preferred next; otherwise Node from the shell PATH runs the packaged script.
// A packaged binary that cannot be made executable falls back to Node
object SidecarLocator {
    fun command(): SidecarCommand = choose(NodeLocator::env, bundledBinary(), ::nodeCommand) { Acpira.LOG.warn(it) }
        .also { if (it.argv.size == 1) Acpira.LOG.info("sidecar engine rust: ${it.argv[0]}") }

    // Pure selection; `warn` reports a packaged binary that had to be skipped
    internal fun choose(env: (String) -> String?, bundled: Path?, node: () -> SidecarCommand, warn: (String) -> Unit): SidecarCommand {
        if (env("ACPIRA_ENGINE").equals("node", ignoreCase = true)) return node()
        env("ACPIRA_SIDECAR_BIN")?.let { override ->
            val p = Paths.get(override)
            if (!Files.isRegularFile(p) || !Files.isExecutable(p)) throw SidecarSetupException("ACPIRA_SIDECAR_BIN is not an executable file: $p")
            return rust(p)
        }
        if (env("ACPIRA_HOST_SERVER") != null || bundled == null) return node()
        if (!Files.isExecutable(bundled)) runCatching { bundled.toFile().setExecutable(true, false) }
        if (Files.isExecutable(bundled)) return rust(bundled)
        warn("bundled sidecar binary is not executable, using Node: $bundled")
        return node()
    }

    private fun rust(path: Path) = SidecarCommand(listOf(path.toString()), "binary $path")

    private fun nodeCommand(): SidecarCommand {
        val node = NodeLocator.node()
        val script = NodeLocator.script()
        return SidecarCommand(listOf(node.toString(), script.toString()), "script $script")
    }

    private fun bundledBinary(): Path? {
        val dir = Acpira.descriptor?.pluginPath?.resolve("sidecar/bin") ?: return null
        return binaryPath(dir, System.getProperty("os.name"), System.getProperty("os.arch"))
    }

    internal fun binaryPath(dir: Path, osName: String, archName: String): Path? {
        val os = when {
            osName.startsWith("Mac", ignoreCase = true) -> "mac"
            osName.startsWith("Windows", ignoreCase = true) -> "windows"
            osName.startsWith("Linux", ignoreCase = true) -> "linux"
            else -> return null
        }
        val arch = when (archName.lowercase()) {
            "aarch64", "arm64" -> "arm64"
            "amd64", "x86_64" -> "x86_64"
            else -> return null
        }
        val exe = if (os == "windows") "acpira.exe" else "acpira"
        return dir.resolve("$os-$arch").resolve(exe).takeIf { Files.isRegularFile(it) }
    }
}
