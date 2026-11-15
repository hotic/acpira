package com.github.hotic.acpira.editor

import com.github.hotic.acpira.connection.AcpiraConnection
import com.google.gson.JsonObject
import com.intellij.openapi.actionSystem.ActionUpdateThread
import com.intellij.openapi.actionSystem.AnActionEvent
import com.intellij.openapi.components.service
import com.intellij.openapi.fileEditor.FileEditorManager
import com.intellij.openapi.project.DumbAwareAction
import com.intellij.openapi.wm.IdeFocusManager
import com.intellij.openapi.wm.ToolWindowManager
import java.awt.KeyboardFocusManager
import javax.swing.SwingUtilities

// Toggles focus between the active code editor and the Acpira chat composer (⌘Esc / Ctrl+Esc).
class FocusInputAction : DumbAwareAction() {
    override fun getActionUpdateThread() = ActionUpdateThread.EDT

    override fun update(e: AnActionEvent) {
        e.presentation.isEnabledAndVisible = e.project != null
    }

    override fun actionPerformed(e: AnActionEvent) {
        val project = e.project ?: return
        val window = ToolWindowManager.getInstance(project).getToolWindow("Acpira")
        val focusOwner = KeyboardFocusManager.getCurrentKeyboardFocusManager().focusOwner
        if (window != null && window.isVisible && focusOwner != null && SwingUtilities.isDescendingFrom(focusOwner, window.component)) {
            val editor = FileEditorManager.getInstance(project).selectedTextEditor
            if (editor != null) {
                IdeFocusManager.getInstance(project).requestFocus(editor.contentComponent, true)
                return
            }
        }
        val focusMsg = JsonObject().apply { addProperty("type", "focusComposer") }
        if (window == null) {
            project.service<AcpiraConnection>().shellTo("sidebar", focusMsg)
            return
        }
        window.activate { project.service<AcpiraConnection>().shellTo("sidebar", focusMsg) }
    }
}
