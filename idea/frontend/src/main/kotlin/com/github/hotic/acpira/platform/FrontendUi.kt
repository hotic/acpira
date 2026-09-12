@file:Suppress("UnstableApiUsage")

package com.github.hotic.acpira.platform

import com.github.hotic.acpira.Acpira
import com.github.hotic.acpira.rpc.UiRequest
import com.github.hotic.acpira.rpc.AcpiraBackendApi
import com.intellij.ide.vfs.virtualFile
import com.intellij.platform.project.projectId
import com.github.hotic.acpira.ui.AcpiraEditors
import com.intellij.ide.projectView.ProjectView
import com.intellij.ide.BrowserUtil
import com.intellij.notification.NotificationGroupManager
import com.intellij.notification.NotificationType
import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.fileEditor.FileEditorManager
import com.intellij.openapi.fileEditor.OpenFileDescriptor
import com.intellij.openapi.project.Project
import com.intellij.testFramework.LightVirtualFile

// Executes backend UI requests on the frontend. File paths are resolved by a client-initiated backend RPC so the platform can
// serialize a remote VFS handle in the requesting client session, including when the two machines have different filesystems.
object FrontendUi {
    suspend fun handle(project: Project, event: UiRequest) {
        when (event) {
            is UiRequest.OpenFile -> openFile(project, event.path, event.line)
            is UiRequest.OpenPlan -> event.path?.let { openFile(project, it, null) } ?: onEdt(project) {
                FileEditorManager.getInstance(project).openFile(LightVirtualFile("plan.md", event.markdown.orEmpty()), true)
            }
            else -> onEdt(project) {
                when (event) {
                    is UiRequest.Toast -> NotificationGroupManager.getInstance()
                        .getNotificationGroup(Acpira.NOTIFICATION_GROUP)
                        .createNotification(event.text, if (event.error) NotificationType.ERROR else NotificationType.INFORMATION)
                        .notify(project)
                    is UiRequest.OpenExternal -> BrowserUtil.browse(event.url)
                    is UiRequest.OpenInEditor -> AcpiraEditors.open(project, event.sessionId)
                    else -> {}
                }
            }
        }
    }

    private fun onEdt(project: Project, body: () -> Unit) {
        ApplicationManager.getApplication().invokeLater { if (!project.isDisposed) body() }
    }

    private suspend fun openFile(project: Project, path: String, line: Int?) {
        val id = AcpiraBackendApi.getInstance().resolveFile(project.projectId(), path)
        onEdt(project) {
            val file = id?.virtualFile()
            if (file == null) {
                Acpira.LOG.warn("frontend could not resolve remote file $path")
                NotificationGroupManager.getInstance().getNotificationGroup(Acpira.NOTIFICATION_GROUP)
                    .createNotification("Could not open file: $path", NotificationType.ERROR).notify(project)
                return@onEdt
            }
            if (file.isDirectory) {
                // Directories belong in the project tree; opening an editor tab rejects them.
                ProjectView.getInstance(project).select(file, file, true)
                return@onEdt
            }
            val descriptor = if (line != null && line > 0) OpenFileDescriptor(project, file, line - 1, 0) else OpenFileDescriptor(project, file)
            descriptor.navigate(true)
        }
    }
}
