package com.github.hotic.acpira.sidecar

import com.google.gson.JsonParser
import java.nio.charset.StandardCharsets
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.Paths
import java.util.concurrent.TimeUnit

// Safe mode (docs/dev/plugins.md): every engine on the machine unloads the user's plugins. The switch goes through the engine binary's
// `acpira safe-mode on|off`, which writes <ACPIRA_HOME>/plugins/state.json, so it works while the sidecar itself is stuck or down;
// running engines follow within a second
object SafeMode {
    // The data home the sidecar uses: ACPIRA_HOME, else ~/.acpira
    fun home(): Path = ShellEnv.get("ACPIRA_HOME")?.let { Paths.get(it) } ?: Paths.get(System.getProperty("user.home"), ".acpira")

    // Whether safe mode is on, from the plugin state (false when there is none or it cannot be read)
    fun isOn(home: Path): Boolean = runCatching {
        val state = JsonParser.parseString(String(Files.readAllBytes(home.resolve("plugins").resolve("state.json")), StandardCharsets.UTF_8))
        state.isJsonObject && state.asJsonObject.get("safeMode")?.takeIf { it.isJsonPrimitive }?.asJsonPrimitive?.let { it.isBoolean && it.asBoolean } == true
    }.getOrDefault(false)

    // Switch with the sidecar binary; null when it worked, else its error output. Blocks until the command ends
    fun set(on: Boolean, home: Path = home()): String? = runCatching {
        val argv = SidecarLocator.command().argv + listOf("safe-mode", if (on) "on" else "off", "--home", home.toString())
        val process = ProcessBuilder(argv).redirectOutput(ProcessBuilder.Redirect.DISCARD).start()
        val stderr = process.errorStream.bufferedReader(StandardCharsets.UTF_8).readText().trim()
        if (!process.waitFor(15, TimeUnit.SECONDS)) { process.destroyForcibly(); return@runCatching "acpira safe-mode timed out" }
        if (process.exitValue() == 0) null else stderr.ifEmpty { "acpira safe-mode exited with ${process.exitValue()}" }
    }.getOrElse { it.message ?: it.toString() }
}
