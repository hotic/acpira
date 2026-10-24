package com.github.hotic.acpira.platform

import com.intellij.openapi.project.Project
import com.intellij.openapi.util.SystemInfo
import org.jetbrains.plugins.terminal.TerminalProjectOptionsProvider
import org.jetbrains.plugins.terminal.TerminalToolWindowManager

// Terminal-backed runner loaded only when the optional Terminal plugin is enabled. Commands are rendered in the configured shell's
// dialect and sent on the EDT as one quoted line.
class IdeTerminalRunner : TerminalRunner {
    override fun run(project: Project, title: String, command: String, args: List<String>, env: Map<String, String?>) {
        val shellPath = runCatching { TerminalProjectOptionsProvider.getInstance(project).shellPath }.getOrNull()
        val widget = TerminalToolWindowManager.getInstance(project).createShellWidget(project.basePath, title, true, true)
        widget.sendCommandToExecute(TerminalCommand.build(command, args, env, TerminalCommand.kind(SystemInfo.isWindows, shellPath)))
    }
}
