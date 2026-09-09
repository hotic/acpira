package com.github.hotic.acpira.platform

import com.github.hotic.acpira.Acpira
import com.github.hotic.acpira.settings.AcpiraSettings
import com.github.hotic.acpira.sidecar.SidecarService
import com.github.hotic.acpira.ui.AcpiraEditors
import com.google.gson.JsonArray
import com.google.gson.JsonNull
import com.google.gson.JsonObject
import com.intellij.ide.BrowserUtil
import com.intellij.ide.actions.RevealFileAction
import com.intellij.notification.NotificationGroupManager
import com.intellij.notification.NotificationType
import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.fileEditor.FileEditorManager
import com.intellij.openapi.fileEditor.OpenFileDescriptor
import com.intellij.openapi.project.Project
import com.intellij.openapi.vfs.LocalFileSystem
import com.intellij.testFramework.LightVirtualFile
import com.intellij.util.concurrency.AppExecutorUtil
import java.nio.file.Files
import java.nio.file.Paths

// The IDE side of the platform RPC: every request arrives with resolved arguments (an absolute path, an allowlisted URL, a ready-made
// command line) and is only executed here. Methods this shell does not implement are simply not declared in `capabilities`, and the
// sidecar falls back on its own (a toast showing the command when there is no terminal)
class IdePlatform(private val project: Project, private val sidecar: SidecarService) {
    private val files = IdeFiles(project)

    val capabilities: List<String> = listOf("openResolvedFile", "openPlanDocument", "openExternal", "revealInOS", "toast", "writeSetting", "searchFiles", "openInEditor") +
        (if (IdeTerminal.available) listOf("runInTerminal") else emptyList())

    fun handle(requestId: String?, request: JsonObject) {
        val method = request.get("method")?.asString
        val done = { error: String? -> if (requestId != null) sidecar.respond(requestId, JsonNull.INSTANCE, error) }
        try {
            when (method) {
                "toast" -> notify(request.get("text").asString, if (request.get("level")?.asString == "error") NotificationType.ERROR else NotificationType.INFORMATION)
                "openExternal" -> BrowserUtil.browse(request.get("url").asString)
                "openResolvedFile" -> onEdt(done) {
                    val path = request.get("path").asString
                    val line = request.get("line")?.takeIf { !it.isJsonNull }?.asInt
                    openFile(path, line)
                }
                "openPlanDocument" -> onEdt(done) {
                    val target = request.getAsJsonObject("target")
                    val path = target.get("path")?.takeIf { !it.isJsonNull }?.asString
                    if (path != null) openFile(path, null)
                    else {
                        FileEditorManager.getInstance(project).openFile(LightVirtualFile("plan.md", target.get("markdown")?.asString ?: ""), true)
                        null
                    }
                }
                "revealInOS" -> {
                    val p = Paths.get(request.get("path").asString)
                    if (Files.isDirectory(p)) RevealFileAction.openDirectory(p) else RevealFileAction.openFile(p)
                    done(null)
                }
                "writeSetting" -> {
                    AcpiraSettings.getInstance().write(request.get("key").asString, request.get("value"))
                    done(null)
                }
                // Off the sidecar's reader thread: the first listing of a large project takes a moment and must not hold up other envelopes
                "searchFiles" -> AppExecutorUtil.getAppExecutorService().execute {
                    val result = runCatching {
                        val hits = files.search(request.get("query")?.asString ?: "")
                        JsonArray().apply { hits.forEach { add(JsonObject().apply { addProperty("uri", it.uri); addProperty("path", it.path) }) } }
                    }
                    if (requestId != null) result.fold({ sidecar.respond(requestId, it, null) }, { sidecar.respond(requestId, null, it.toString()) })
                }
                "openInEditor" -> onEdt(done) {
                    AcpiraEditors.open(project, request.get("sessionId")?.takeIf { it.isJsonPrimitive }?.asString)
                    null
                }
                "runInTerminal" -> onEdt(done) {
                    val args = request.getAsJsonArray("args")?.map { it.asString } ?: emptyList()
                    val env = request.get("env")?.takeIf { it.isJsonObject }?.asJsonObject?.entrySet()
                        ?.associate { (k, v) -> k to (if (v.isJsonNull) null else v.asString) } ?: emptyMap()
                    IdeTerminal.run(project, request.get("title").asString, request.get("command").asString, args, env)
                    null
                }
                else -> done("$method is not supported by this IDE shell")
            }
        } catch (e: Exception) {
            Acpira.LOG.warn("platform request $method failed", e)
            done(e.toString())
        }
    }

    // Returns an error text, or null when the file opened; images and other binaries go to whatever editor the IDE has for them
    private fun openFile(path: String, line: Int?): String? {
        val vf = LocalFileSystem.getInstance().refreshAndFindFileByNioFile(Paths.get(path)) ?: return "File not found: $path"
        if (vf.isDirectory) return "Not a file: $path"
        val descriptor = if (line != null && line > 0) OpenFileDescriptor(project, vf, line - 1, 0) else OpenFileDescriptor(project, vf)
        descriptor.navigate(true)
        return null
    }

    private fun notify(text: String, type: NotificationType) {
        NotificationGroupManager.getInstance().getNotificationGroup(Acpira.NOTIFICATION_GROUP).createNotification(text, type).notify(project)
    }

    // Editor work happens on the EDT; the response always goes out, a project closed meanwhile answers with an error instead of a 30 s timeout
    private fun onEdt(done: (String?) -> Unit, body: () -> String?) {
        ApplicationManager.getApplication().invokeLater {
            if (project.isDisposed) { done("project closed"); return@invokeLater }
            done(runCatching(body).getOrElse { e -> Acpira.LOG.warn("platform request failed on the EDT", e); e.toString() })
        }
    }
}
