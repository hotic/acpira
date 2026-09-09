package com.github.hotic.acpira.platform

import com.intellij.openapi.project.Project
import com.intellij.openapi.util.SystemInfo
import org.jetbrains.plugins.terminal.TerminalToolWindowManager

// runInTerminal on the Terminal plugin, an optional dependency: `available` is checked before anything here is touched, so a disabled
// plugin leaves the capability undeclared instead of failing with a missing class. Runs on the EDT
object IdeTerminal {
    val available: Boolean by lazy {
        runCatching { Class.forName("org.jetbrains.plugins.terminal.TerminalToolWindowManager", false, IdeTerminal::class.java.classLoader) }.isSuccess
    }

    fun run(project: Project, title: String, command: String, args: List<String>, env: Map<String, String?>) {
        val widget = TerminalToolWindowManager.getInstance(project).createShellWidget(project.basePath, title, true, true)
        widget.sendCommandToExecute(TerminalCommand.build(command, args, env, SystemInfo.isWindows))
    }
}
