package com.github.hotic.acpira.ui

import com.github.hotic.acpira.sidecar.SidecarService
import com.github.hotic.acpira.web.AcpiraBrowser
import com.google.gson.JsonObject
import com.intellij.icons.AllIcons
import com.intellij.openapi.actionSystem.AnActionEvent
import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.project.DumbAware
import com.intellij.openapi.project.DumbAwareAction
import com.intellij.openapi.project.Project
import com.intellij.openapi.wm.ToolWindow
import com.intellij.openapi.wm.ToolWindowFactory
import com.intellij.ui.content.ContentFactory
import com.intellij.ui.jcef.JBCefApp
import java.awt.BorderLayout
import javax.swing.JComponent
import javax.swing.JPanel

// The sidebar: a JCEF view of Acpira attached to this project's sidecar. Without JCEF, or while the sidecar cannot start (no Node 22,
// missing script), the same slot shows what to do instead of a blank browser. The title bar's "Open in Editor" starts a fresh
// conversation in an editor tab, like the VS Code view title button
class AcpiraToolWindowFactory : ToolWindowFactory, DumbAware {
    override fun createToolWindowContent(project: Project, toolWindow: ToolWindow) {
        val root = JPanel(BorderLayout())
        toolWindow.contentManager.addContent(ContentFactory.getInstance().createContent(root, "", false))
        toolWindow.setTitleActions(listOf(object : DumbAwareAction("Open in Editor", "Start a conversation in an editor tab", AllIcons.Actions.OpenNewTab) {
            override fun actionPerformed(e: AnActionEvent) { AcpiraEditors.open(project, null) }
        }))
        val supported = runCatching { JBCefApp.isSupported() }.getOrDefault(false)
        if (!supported) {
            root.add(StatusPanel.message("Acpira needs the embedded browser (JCEF), which this IDE runtime does not provide.", StatusPanel.JCEF_HINT), BorderLayout.CENTER)
            return
        }
        lateinit var browser: AcpiraBrowser
        val show = { component: JComponent ->
            root.removeAll()
            root.add(component, BorderLayout.CENTER)
            root.revalidate()
            root.repaint()
        }
        browser = AcpiraBrowser(project, "sidebar", JsonObject().apply { addProperty("mostRecent", true) }, toolWindow.disposable, { state, detail ->
            ApplicationManager.getApplication().invokeLater {
                if (project.isDisposed) return@invokeLater
                when (state) {
                    SidecarService.State.FAILED -> show(StatusPanel.message("Acpira could not start its Node sidecar.", detail ?: "See the IDE log.") { SidecarService.getInstance(project).retry() })
                    else -> if (browser.component.parent !== root) show(browser.component)
                }
            }
        })
        show(browser.component)
    }
}
