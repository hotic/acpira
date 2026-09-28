package com.github.hotic.acpira.editor

import com.github.hotic.acpira.connection.AcpiraConnection
import com.intellij.openapi.actionSystem.ActionUpdateThread
import com.intellij.openapi.actionSystem.AnActionEvent
import com.intellij.openapi.actionSystem.CommonDataKeys
import com.intellij.openapi.components.service
import com.intellij.openapi.project.DumbAwareAction
import com.intellij.openapi.wm.ToolWindowManager

// Editor context menu "Add to Chat": pins the selection (or the caret's line) into the Acpira tool window's composer,
// independent of the live-selection setting. The tool window is opened first; its view receives the range once its page is up
class AddSelectionAction : DumbAwareAction() {
    override fun getActionUpdateThread() = ActionUpdateThread.EDT

    override fun update(e: AnActionEvent) {
        val editor = e.getData(CommonDataKeys.EDITOR)
        val project = e.project
        e.presentation.isEnabledAndVisible = editor != null && project != null && project.service<EditorRelay>().selectionOf(editor, lineFallback = true) != null
    }

    override fun actionPerformed(e: AnActionEvent) {
        val project = e.project ?: return
        val editor = e.getData(CommonDataKeys.EDITOR) ?: return
        val range = project.service<EditorRelay>().selectionOf(editor, lineFallback = true) ?: return
        val message = EditorRelay.message("addSelection", range)
        val window = ToolWindowManager.getInstance(project).getToolWindow("Acpira")
        if (window == null) { project.service<AcpiraConnection>().shellTo("sidebar", message); return }
        window.activate { project.service<AcpiraConnection>().shellTo("sidebar", message) }
    }
}
