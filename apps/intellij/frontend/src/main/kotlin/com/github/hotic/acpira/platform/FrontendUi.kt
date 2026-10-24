@file:Suppress("UnstableApiUsage")

package com.github.hotic.acpira.platform

import com.github.hotic.acpira.Acpira
import com.github.hotic.acpira.connection.AcpiraConnection
import com.github.hotic.acpira.rpc.UiRequest
import com.github.hotic.acpira.rpc.AcpiraBackendApi
import com.intellij.ide.vfs.virtualFile
import com.intellij.platform.project.projectId
import com.github.hotic.acpira.ui.AcpiraEditors
import com.intellij.ide.BrowserUtil
import com.intellij.ide.FileSelectInContext
import com.intellij.ide.projectView.ProjectView
import com.intellij.notification.NotificationAction
import com.intellij.notification.NotificationGroupManager
import com.intellij.notification.NotificationType
import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.components.service
import com.intellij.openapi.fileEditor.FileEditorManager
import com.intellij.openapi.fileEditor.OpenFileDescriptor
import com.intellij.openapi.project.Project
import com.intellij.openapi.wm.WindowManager
import com.intellij.testFramework.LightVirtualFile
import kotlinx.coroutines.launch

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
                    is UiRequest.Toast -> toast(project, event)
                    is UiRequest.OpenExternal -> BrowserUtil.browse(event.url)
                    is UiRequest.OpenInEditor -> AcpiraEditors.open(project, event.sessionId)
                    is UiRequest.Notify -> announce(project, event)
                    else -> {}
                }
            }
        }
    }

    // A notification; with openPath it carries an Open action that opens the file (resolved the same way as OpenFile) and expires
    private fun toast(project: Project, event: UiRequest.Toast) {
        val notification = NotificationGroupManager.getInstance()
            .getNotificationGroup(Acpira.NOTIFICATION_GROUP)
            .createNotification(event.text, if (event.error) NotificationType.ERROR else NotificationType.INFORMATION)
        val path = event.openPath
        if (path != null) {
            notification.addAction(NotificationAction.createSimpleExpiring(event.openLabel ?: "Open") {
                // The connection's scope: the file is resolved through a backend RPC, which suspends
                project.service<AcpiraConnection>().cs.launch { openFile(project, path, null) }
            })
        }
        notification.notify(project)
    }

    // A session needs the user or has settled. Quiet while this project's frame is active and a showing chat has that session;
    // otherwise one initialized chat plays the chime and the IDE shows a balloon with an Open action. The IDE itself passes
    // balloons on to the OS while it is in the background (Settings | Notifications | "Enable system notifications")
    private fun announce(project: Project, event: UiRequest.Notify) {
        val connection = project.service<AcpiraConnection>()
        val active = WindowManager.getInstance().getFrame(project)?.isActive == true
        if (active && connection.shows(event.sessionId)) return
        if (event.sound) connection.chime()
        if (!event.system) return
        NotificationGroupManager.getInstance().getNotificationGroup(Acpira.NOTIFICATION_GROUP)
            .createNotification(event.title, event.body, NotificationType.INFORMATION)
            .addAction(NotificationAction.createSimpleExpiring(event.openLabel) { AcpiraEditors.open(project, event.sessionId) })
            .notify(project)
    }

    private fun onEdt(project: Project, body: () -> Unit) {
        ApplicationManager.getApplication().invokeLater { if (!project.isDisposed) body() }
    }

    private suspend fun openFile(project: Project, path: String, line: Int?) {
        val resolved = AcpiraBackendApi.getInstance().resolveFile(project.projectId(), path)
        onEdt(project) {
            val file = resolved?.id?.virtualFile()
            if (file == null) {
                Acpira.LOG.warn("frontend could not resolve remote file $path")
                NotificationGroupManager.getInstance().getNotificationGroup(Acpira.NOTIFICATION_GROUP)
                    .createNotification("Could not open file: $path", NotificationType.ERROR).notify(project)
                return@onEdt
            }
            if (resolved.directory) {
                // Select In uses the Client's native remote tree protocol; direct ProjectView selection can affect only a backend tree.
                val target = ProjectView.getInstance(project).currentProjectViewPane?.createSelectInTarget()
                target?.selectIn(FileSelectInContext(project, file), true)
                return@onEdt
            }
            val descriptor = if (line != null && line > 0) OpenFileDescriptor(project, file, line - 1, 0) else OpenFileDescriptor(project, file)
            descriptor.navigate(true)
        }
    }
}
