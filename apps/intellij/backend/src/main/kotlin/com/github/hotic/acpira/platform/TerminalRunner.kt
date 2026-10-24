package com.github.hotic.acpira.platform

import com.intellij.openapi.project.Project

// Optional backend service boundary for the Terminal plugin. Absence of the backend-terminal module removes the capability without
// loading any Terminal classes into the core backend module.
interface TerminalRunner {
    fun run(project: Project, title: String, command: String, args: List<String>, env: Map<String, String?>)
}
