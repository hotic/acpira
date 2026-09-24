package com.github.hotic.acpira.sidecar

import com.github.hotic.acpira.Acpira
import com.intellij.util.EnvironmentUtil
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.Paths

class SidecarSetupException(message: String) : Exception(message)

// The process line that starts the sidecar and a label for the log
data class SidecarCommand(val argv: List<String>, val label: String)

// The IDE's login-shell environment (EnvironmentUtil, never System.getenv alone: a Dock launch has no shell PATH), which the sidecar
// and the agent CLIs it spawns run with
object ShellEnv {
    fun map(): Map<String, String> = EnvironmentUtil.getEnvironmentMap()

    fun get(name: String): String? = (System.getenv(name) ?: map()[name])?.trim()?.takeIf { it.isNotEmpty() }
}

// Which sidecar binary runs: ACPIRA_SIDECAR_BIN (Gradle -PsidecarBin) names one for development; otherwise the binary packaged at
// <plugin>/sidecar/bin/<os>-<arch>/acpira[.exe] for the backend machine. Every release package carries it, so a missing or
// unrunnable binary is a setup error rather than something to work around
object SidecarLocator {
    fun command(): SidecarCommand = choose(ShellEnv::get, bundledBinary(), platformLabel())
        .also { Acpira.LOG.info("sidecar binary: ${it.argv[0]}") }

    // Pure selection over the environment and the packaged binary for this machine (null when the package has none)
    internal fun choose(env: (String) -> String?, bundled: Path?, platform: String): SidecarCommand {
        env("ACPIRA_SIDECAR_BIN")?.let { override ->
            val p = Paths.get(override)
            if (!Files.isRegularFile(p) || !Files.isExecutable(p)) throw SidecarSetupException("ACPIRA_SIDECAR_BIN is not an executable file: $p")
            return binary(p)
        }
        if (bundled == null) throw SidecarSetupException("This plugin package has no sidecar binary for $platform; install the package built for this machine.")
        if (!Files.isExecutable(bundled)) runCatching { bundled.toFile().setExecutable(true, false) }
        if (!Files.isExecutable(bundled)) throw SidecarSetupException("The packaged sidecar binary cannot be made executable: $bundled")
        return binary(bundled)
    }

    private fun binary(path: Path) = SidecarCommand(listOf(path.toString()), "binary $path")

    private fun platformLabel() = "${System.getProperty("os.name")} ${System.getProperty("os.arch")}"

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
