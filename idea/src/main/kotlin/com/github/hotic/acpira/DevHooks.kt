package com.github.hotic.acpira

import com.github.hotic.acpira.sidecar.SidecarService
import com.intellij.openapi.application.ApplicationActivationListener
import com.intellij.openapi.project.Project
import com.intellij.openapi.project.ProjectManager
import com.intellij.openapi.startup.ProjectActivity
import com.intellij.openapi.wm.IdeFrame
import com.intellij.openapi.wm.ToolWindowManager

// `-Dacpira.dev.autoOpen=true` (runIde -PautoOpen) opens the tool window as soon as a project is up, so a sandbox session exercises the
// whole path without a click; a no-op otherwise
class DevAutoOpen : ProjectActivity {
    override suspend fun execute(project: Project) {
        if (!System.getProperty("acpira.dev.autoOpen").toBoolean()) return
        val manager = ToolWindowManager.getInstance(project)
        manager.invokeLater { manager.getToolWindow(Acpira.TOOL_WINDOW_ID)?.show() }
    }
}

// Coming back to the IDE (from a terminal where a CLI was installed or logged in, or from another window on the same ~/.acpira) is the
// moment the sidecar re-checks executables and reconciles the session list
class WindowFocusRelay : ApplicationActivationListener {
    override fun applicationActivated(ideFrame: IdeFrame) {
        for (project in ProjectManager.getInstance().openProjects) {
            if (project.isDisposed) continue
            project.getServiceIfCreated(SidecarService::class.java)?.windowFocus()
        }
    }
}
