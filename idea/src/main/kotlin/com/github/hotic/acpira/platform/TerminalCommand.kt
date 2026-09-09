package com.github.hotic.acpira.platform

// The one line typed into a fresh terminal for runInTerminal: the command with each argument quoted, prefixed by the environment the
// sidecar asked for. A null value removes the variable (Devin's login must not see ACP_BACKEND), which a shell can only do around the
// command, so POSIX shells get `env -u K A=v cmd …` and PowerShell `$env:K=$null; $env:A='v'; & 'cmd' …`
object TerminalCommand {
    fun build(command: String, args: List<String>, env: Map<String, String?>, windows: Boolean): String =
        if (windows) powershell(command, args, env) else posix(command, args, env)

    private fun posix(command: String, args: List<String>, env: Map<String, String?>): String {
        val words = ArrayList<String>()
        if (env.isNotEmpty()) {
            words += "env"
            env.filterValues { it == null }.keys.forEach { words += "-u"; words += quotePosix(it) }
            env.forEach { (k, v) -> if (v != null) words += "${quotePosix(k)}=${quotePosix(v)}" }
        }
        words += quotePosix(command)
        args.forEach { words += quotePosix(it) }
        return words.joinToString(" ")
    }

    private fun powershell(command: String, args: List<String>, env: Map<String, String?>): String {
        val parts = ArrayList<String>()
        env.forEach { (k, v) -> parts += if (v == null) "\$env:$k=\$null" else "\$env:$k=${quotePs(v)}" }
        parts += (listOf("&", quotePs(command)) + args.map(::quotePs)).joinToString(" ")
        return parts.joinToString("; ")
    }

    // Same rule as the VS Code host: plain words pass through, anything else is single-quoted with ' escaped as '\''
    private val plain = Regex("^[\\w./=:@%+-]+$")

    fun quotePosix(s: String): String = if (plain.matches(s)) s else "'" + s.replace("'", "'\\''") + "'"

    private fun quotePs(s: String): String = "'" + s.replace("'", "''") + "'"
}
