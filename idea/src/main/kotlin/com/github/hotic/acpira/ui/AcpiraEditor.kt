package com.github.hotic.acpira.ui

import com.github.hotic.acpira.sidecar.SidecarService
import com.github.hotic.acpira.web.AcpiraBrowser
import com.google.gson.JsonPrimitive
import com.intellij.icons.AllIcons
import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.fileEditor.FileEditor
import com.intellij.openapi.fileEditor.FileEditorManager
import com.intellij.openapi.fileEditor.FileEditorPolicy
import com.intellij.openapi.fileEditor.FileEditorProvider
import com.intellij.openapi.fileEditor.FileEditorState
import com.intellij.openapi.fileEditor.ex.FileEditorManagerEx
import com.intellij.openapi.fileEditor.impl.EditorTabTitleProvider
import com.intellij.openapi.fileTypes.FileType
import com.intellij.openapi.project.DumbAware
import com.intellij.openapi.project.Project
import com.intellij.openapi.util.Disposer
import com.intellij.openapi.util.UserDataHolderBase
import com.intellij.openapi.vfs.VirtualFile
import com.intellij.testFramework.LightVirtualFile
import com.intellij.ui.jcef.JBCefApp
import java.awt.BorderLayout
import java.beans.PropertyChangeListener
import javax.swing.Icon
import javax.swing.JComponent
import javax.swing.JPanel

// A conversation in an editor tab: the same JCEF view as the sidebar, attached to the project's sidecar as its own viewer (host
// "editor", wide layout) on the given session or a fresh one. The tab is an in-memory virtual file of a private type, so no other
// editor provider claims it and nothing is ever written to disk
object AcpiraSessionFileType : FileType {
    override fun getName() = "Acpira Session"
    override fun getDescription() = "Acpira conversation"
    override fun getDefaultExtension() = ""
    override fun getIcon(): Icon = AllIcons.Toolwindows.WebToolWindow
    override fun isBinary() = true
    override fun isReadOnly() = true
}

class AcpiraSessionFile(val sessionId: String?) : LightVirtualFile("Acpira", AcpiraSessionFileType, "") {
    // What the tab shows now (the viewer can move to another session); the tab is named after it through AcpiraTabTitle
    @Volatile var currentSessionId: String? = sessionId
    @Volatile var title: String = "Acpira"

    init { isWritable = false }
}

class AcpiraEditorProvider : FileEditorProvider, DumbAware {
    override fun accept(project: Project, file: VirtualFile) = file is AcpiraSessionFile
    override fun createEditor(project: Project, file: VirtualFile): FileEditor = AcpiraEditor(project, file as AcpiraSessionFile)
    override fun getEditorTypeId() = "acpira-session"
    override fun getPolicy() = FileEditorPolicy.HIDE_DEFAULT_EDITOR
}

class AcpiraTabTitle : EditorTabTitleProvider {
    override fun getEditorTabTitle(project: Project, file: VirtualFile): String? = (file as? AcpiraSessionFile)?.title
}

class AcpiraEditor(private val project: Project, private val file: AcpiraSessionFile) : UserDataHolderBase(), FileEditor {
    private val root = JPanel(BorderLayout())
    private var browser: AcpiraBrowser? = null

    init {
        if (!runCatching { JBCefApp.isSupported() }.getOrDefault(false)) {
            root.add(StatusPanel.message("Acpira needs the embedded browser (JCEF), which this IDE runtime does not provide.", StatusPanel.JCEF_HINT), BorderLayout.CENTER)
        } else {
            val b = AcpiraBrowser(project, "editor", file.sessionId?.let(::JsonPrimitive), this, ::onState) { id, title ->
                file.currentSessionId = id
                if (title != file.title) {
                    file.title = title
                    ApplicationManager.getApplication().invokeLater { if (!project.isDisposed) FileEditorManagerEx.getInstanceEx(project).updateFileName(file) }
                }
            }
            browser = b
            root.add(b.component, BorderLayout.CENTER)
        }
    }

    private fun onState(state: SidecarService.State, detail: String?) {
        ApplicationManager.getApplication().invokeLater {
            if (project.isDisposed) return@invokeLater
            val b = browser ?: return@invokeLater
            val next: JComponent = when (state) {
                SidecarService.State.FAILED -> StatusPanel.message("Acpira could not start its Node sidecar.", detail ?: "See the IDE log.") { SidecarService.getInstance(project).start() }
                else -> b.component
            }
            if (root.componentCount == 1 && root.getComponent(0) === next) return@invokeLater
            root.removeAll()
            root.add(next, BorderLayout.CENTER)
            root.revalidate()
            root.repaint()
        }
    }

    override fun getComponent(): JComponent = root
    override fun getPreferredFocusedComponent(): JComponent? = browser?.component
    override fun getName() = "Acpira"
    override fun getFile(): VirtualFile = file
    override fun setState(state: FileEditorState) {}
    override fun isModified() = false
    override fun isValid() = true
    override fun addPropertyChangeListener(listener: PropertyChangeListener) {}
    override fun removePropertyChangeListener(listener: PropertyChangeListener) {}
    override fun dispose() {}
}

object AcpiraEditors {
    // Opens a tab on the session (a fresh one when null) and focuses it; a tab already showing that session is reused
    fun open(project: Project, sessionId: String?) {
        val manager = FileEditorManager.getInstance(project)
        val existing = sessionId?.let { id -> manager.openFiles.firstOrNull { it is AcpiraSessionFile && it.currentSessionId == id } }
        manager.openFile(existing ?: AcpiraSessionFile(sessionId), true)
    }
}
