package com.github.hotic.acpira.platform

// The one line typed into a fresh terminal for runInTerminal: the command with each argument quoted, prefixed by the environment the
// sidecar asked for. A null value removes the variable (Devin's login must not see ACP_BACKEND), which a shell can only do around the
// command, so POSIX shells get `env -u K A=v cmd …`, PowerShell `$env:K=$null; $env:A='v'; & 'cmd' …`, and cmd.exe `set "K=" & set "A=v" & cmd …`.
// The dialect follows the IDE's configured shell, not just the OS: Windows users may run cmd, Git Bash, or WSL in the Terminal tool window
object TerminalCommand {
    enum class ShellKind { Posix, PowerShell, Cmd }

    fun kind(windows: Boolean, shellPath: String?): ShellKind {
        if (!windows) return ShellKind.Posix
        val name = (shellPath ?: "").substringAfterLast('/').substringAfterLast('\\').lowercase()
        val stem = name.removeSuffix(".exe")
        return when {
            stem == "cmd" || stem == "command" -> ShellKind.Cmd
            stem == "bash" || stem == "zsh" || stem == "sh" || stem == "dash" || stem == "fish" ||
                stem.contains("wsl") || name.contains("git-bash") || name.contains("git-cmd") -> ShellKind.Posix
            else -> ShellKind.PowerShell
        }
    }

    fun build(command: String, args: List<String>, env: Map<String, String?>, windows: Boolean): String =
        build(command, args, env, if (windows) ShellKind.PowerShell else ShellKind.Posix)

    fun build(command: String, args: List<String>, env: Map<String, String?>, kind: ShellKind): String =
        when (kind) {
            ShellKind.Posix -> posix(command, args, env)
            ShellKind.PowerShell -> powershell(command, args, env)
            ShellKind.Cmd -> cmd(command, args, env)
        }

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

    private fun cmd(command: String, args: List<String>, env: Map<String, String?>): String {
        val parts = ArrayList<String>()
        env.forEach { (k, v) ->
            parts += if (v == null) "set \"$k=\"" else "set \"$k=${v.replace("%", "%%").replace("\"", "\"\"")}\""
        }
        parts += (listOf(quoteCmd(command)) + args.map(::quoteCmd)).joinToString(" ")
        return parts.joinToString(" & ")
    }

    // Same rule as the VS Code host: plain words pass through, anything else is single-quoted with ' escaped as '\''
    private val plain = Regex("^[\\w./=:@%+-]+$")

    fun quotePosix(s: String): String = if (plain.matches(s)) s else "'" + s.replace("'", "'\\''") + "'"

    private fun quotePs(s: String): String = "'" + s.replace("'", "''") + "'"

    private fun quoteCmd(s: String): String {
        if (plain.matches(s) && !s.contains('%')) return s
        return "\"" + s.replace("%", "%%").replace("\"", "\"\"") + "\""
    }
}
