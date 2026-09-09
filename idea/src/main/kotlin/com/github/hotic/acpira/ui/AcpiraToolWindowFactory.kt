package com.github.hotic.acpira.ui

import com.github.hotic.acpira.sidecar.SidecarService
import com.github.hotic.acpira.web.AcpiraBrowser
import com.google.gson.JsonObject
import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.project.DumbAware
import com.intellij.openapi.project.Project
import com.intellij.openapi.wm.ToolWindow
import com.intellij.openapi.wm.ToolWindowFactory
import com.intellij.ui.components.JBLabel
import com.intellij.ui.components.JBPanel
import com.intellij.ui.content.ContentFactory
import com.intellij.ui.jcef.JBCefApp
import com.intellij.util.ui.JBUI
import java.awt.BorderLayout
import java.awt.FlowLayout
import javax.swing.JButton
import javax.swing.JComponent
import javax.swing.JPanel
import javax.swing.SwingConstants

// The sidebar: a JCEF view of Acpira attached to this project's sidecar. Without JCEF, or while the sidecar cannot start (no Node 22,
// missing script), the same slot shows what to do instead of a blank browser
class AcpiraToolWindowFactory : ToolWindowFactory, DumbAware {
    override fun createToolWindowContent(project: Project, toolWindow: ToolWindow) {
        val root = JPanel(BorderLayout())
        toolWindow.contentManager.addContent(ContentFactory.getInstance().createContent(root, "", false))
        val supported = runCatching { JBCefApp.isSupported() }.getOrDefault(false)
        if (!supported) {
            root.add(message(
                "Acpira needs the embedded browser (JCEF), which this IDE runtime does not provide.",
                "Choose a JetBrains Runtime with JCEF (Help → Find Action → Choose Boot Java Runtime for the IDE) and restart.",
            ), BorderLayout.CENTER)
            return
        }
        lateinit var browser: AcpiraBrowser
        val show = { component: JComponent ->
            root.removeAll()
            root.add(component, BorderLayout.CENTER)
            root.revalidate()
            root.repaint()
        }
        browser = AcpiraBrowser(project, "sidebar", JsonObject().apply { addProperty("mostRecent", true) }, toolWindow.disposable) { state, detail ->
            ApplicationManager.getApplication().invokeLater {
                if (project.isDisposed) return@invokeLater
                when (state) {
                    SidecarService.State.FAILED -> show(message("Acpira could not start its Node sidecar.", detail ?: "See the IDE log.", retry = { SidecarService.getInstance(project).start() }))
                    else -> if (browser.component.parent !== root) show(browser.component)
                }
            }
        }
        show(browser.component)
    }

    private fun message(title: String, detail: String, retry: (() -> Unit)? = null): JComponent {
        val panel = JBPanel<JBPanel<*>>(BorderLayout()).apply { border = JBUI.Borders.empty(16) }
        val text = JBLabel("<html><b>$title</b><br><br>$detail</html>", SwingConstants.LEFT).apply { setAllowAutoWrapping(true); setCopyable(true) }
        panel.add(text, BorderLayout.NORTH)
        if (retry != null) {
            val actions = JPanel(FlowLayout(FlowLayout.LEFT, 0, 0)).apply { border = JBUI.Borders.emptyTop(12) }
            actions.add(JButton("Retry").apply { addActionListener { retry() } })
            panel.add(actions, BorderLayout.CENTER)
        }
        return panel
    }
}
