package com.github.hotic.acpira.platform

import com.github.hotic.acpira.Acpira
import com.github.hotic.acpira.rpc.UiRequest
import com.github.hotic.acpira.backend.UiRequestBus
import com.github.hotic.acpira.settings.AcpiraSettings
import com.github.hotic.acpira.sidecar.SidecarService
import com.google.gson.JsonArray
import com.google.gson.JsonNull
import com.google.gson.JsonObject
import com.intellij.ide.actions.RevealFileAction
import com.intellij.platform.ide.productMode.IdeProductMode
import com.intellij.openapi.application.ApplicationManager
import com.intellij.openapi.project.Project
import com.intellij.openapi.vfs.LocalFileSystem
import com.intellij.openapi.vfs.VirtualFile
import com.intellij.util.concurrency.AppExecutorUtil
import java.nio.file.Files
import java.nio.file.Paths

// The IDE side of the platform RPC: every request arrives with resolved arguments (an absolute path, an allowlisted URL, a ready-made
// command line) and is only executed here. Methods this shell does not implement are simply not declared in `capabilities`, and the
// sidecar falls back on its own (a toast showing the command when there is no terminal)
class IdePlatform(private val project: Project, private val sidecar: SidecarService) {
    private val files = IdeFiles(project)
    private val uiRequests = project.getService(UiRequestBus::class.java)
    private val terminal = ApplicationManager.getApplication().getService(TerminalRunner::class.java)

    val capabilities: List<String> = listOf("openResolvedFile", "openPlanDocument", "openExternal", "revealInOS", "toast", "writeSetting", "searchFiles", "openInEditor") +
        (if (terminal != null) listOf("runInTerminal") else emptyList())

    fun handle(requestId: String?, request: JsonObject) {
        val method = request.get("method")?.asString
        val done = { error: String? -> if (requestId != null) sidecar.respond(requestId, JsonNull.INSTANCE, error) }
        try {
            when (method) {
                "toast" -> {
                    uiRequests.send(UiRequest.Toast(request.get("text").asString, request.get("level")?.asString == "error"))
                    done(null)
                }
                "openExternal" -> {
                    uiRequests.send(UiRequest.OpenExternal(request.get("url").asString))
                    done(null)
                }
                "openResolvedFile" -> {
                    val path = request.get("path").asString
                    val line = request.get("line")?.takeIf { !it.isJsonNull }?.asInt
                    done(resolveFile(path, allowDirectory = true).fold(
                        onSuccess = { uiRequests.send(UiRequest.OpenFile(it.path, line)); null },
                        onFailure = { it.message ?: it.toString() },
                    ))
                }
                "openPlanDocument" -> {
                    val target = request.getAsJsonObject("target")
                    val path = target.get("path")?.takeIf { !it.isJsonNull }?.asString
                    if (path != null) {
                        done(resolveFile(path).fold(
                            onSuccess = { uiRequests.send(UiRequest.OpenPlan(it.path, null)); null },
                            onFailure = { it.message ?: it.toString() },
                        ))
                    } else {
                        uiRequests.send(UiRequest.OpenPlan(null, target.get("markdown")?.asString ?: ""))
                        done(null)
                    }
                }
                "revealInOS" -> {
                    if (IdeProductMode.isBackend) done("Reveal in Finder / Explorer is not available for a remote project")
                    else {
                        val p = Paths.get(request.get("path").asString)
                        if (Files.isDirectory(p)) RevealFileAction.openDirectory(p) else RevealFileAction.openFile(p)
                        done(null)
                    }
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
                "openInEditor" -> {
                    uiRequests.send(UiRequest.OpenInEditor(request.get("sessionId")?.takeIf { it.isJsonPrimitive }?.asString))
                    done(null)
                }
                "runInTerminal" -> onEdt(done) {
                    val runner = terminal ?: return@onEdt "The Terminal plugin is not available"
                    val args = request.getAsJsonArray("args")?.map { it.asString } ?: emptyList()
                    val env = request.get("env")?.takeIf { it.isJsonObject }?.asJsonObject?.entrySet()
                        ?.associate { (k, v) -> k to (if (v.isJsonNull) null else v.asString) } ?: emptyMap()
                    runner.run(project, request.get("title").asString, request.get("command").asString, args, env)
                    null
                }
                else -> done("$method is not supported by this IDE shell")
            }
        } catch (e: Exception) {
            Acpira.LOG.warn("platform request $method failed", e)
            done(e.toString())
        }
    }

    private fun resolveFile(path: String, allowDirectory: Boolean = false): Result<VirtualFile> {
        val file = LocalFileSystem.getInstance().refreshAndFindFileByNioFile(Paths.get(path)) ?: return Result.failure(IllegalArgumentException("File not found: $path"))
        if (file.isDirectory && !allowDirectory) return Result.failure(IllegalArgumentException("Not a file: $path"))
        return Result.success(file)
    }

    // Editor work happens on the EDT; the response always goes out, a project closed meanwhile answers with an error instead of a 30 s timeout
    private fun onEdt(done: (String?) -> Unit, body: () -> String?) {
        ApplicationManager.getApplication().invokeLater {
            if (project.isDisposed) { done("project closed"); return@invokeLater }
            done(runCatching(body).getOrElse { e -> Acpira.LOG.warn("platform request failed on the EDT", e); e.toString() })
        }
    }
}
