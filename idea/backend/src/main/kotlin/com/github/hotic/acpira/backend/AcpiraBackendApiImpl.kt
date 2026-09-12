@file:Suppress("UnstableApiUsage")

package com.github.hotic.acpira.backend

import com.github.hotic.acpira.rpc.ResolvedPath
import com.intellij.ide.vfs.rpcId
import com.intellij.openapi.vfs.LocalFileSystem
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import com.github.hotic.acpira.rpc.AcpiraBackendApi
import com.github.hotic.acpira.rpc.SidecarState
import com.github.hotic.acpira.rpc.UiRequest
import com.github.hotic.acpira.rpc.ViewAttach
import com.github.hotic.acpira.rpc.ViewEvent
import com.github.hotic.acpira.sidecar.BlobStore
import com.github.hotic.acpira.sidecar.SidecarService
import com.google.gson.JsonElement
import com.google.gson.JsonParser
import com.intellij.platform.project.ProjectId
import com.intellij.platform.project.findProjectOrNull
import fleet.rpc.core.Blob
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.buffer
import kotlinx.coroutines.flow.callbackFlow

// Backend RPC adapter around the project SidecarService. A view is attached for exactly the collector lifetime, host events retain their
// source order through an unlimited channel, and backend-only blob access remains confined by BlobStore.
class AcpiraBackendApiImpl : AcpiraBackendApi {
    override suspend fun attach(projectId: ProjectId, view: ViewAttach): Flow<ViewEvent> {
        val project = projectId.findProjectOrNull() ?: error("Acpira backend project is not available: $projectId")
        val service = SidecarService.getInstance(project)
        return callbackFlow {
            val attached = object : SidecarService.View {
                override val viewId = view.viewId
                override val host = view.host
                override val initial: JsonElement? = view.initial?.let { JsonParser.parseString(it) }
                override val locale = view.locale
                @Volatile override var lastSessionId: String? = null

                override fun onHostMessage(message: JsonElement) {
                    if (message.isJsonObject) {
                        val obj = message.asJsonObject
                        val session = when (obj.get("type")?.asString) {
                            "session" -> obj.getAsJsonObject("session")
                            "init" -> obj.getAsJsonObject("state")?.getAsJsonObject("active")
                            else -> null
                        }
                        session?.get("id")?.takeIf { it.isJsonPrimitive }?.asString?.let { lastSessionId = it }
                    }
                    trySend(ViewEvent.Host(message.toString()))
                }

                override fun onState(state: SidecarState, detail: String?) {
                    trySend(ViewEvent.State(state, detail))
                }
            }
            service.attach(attached)
            awaitClose { service.detach(view.viewId) }
        }.buffer(Channel.UNLIMITED)
    }

    override suspend fun post(projectId: ProjectId, viewId: String, json: String) {
        service(projectId).webviewMessage(viewId, JsonParser.parseString(json))
    }

    override suspend fun retry(projectId: ProjectId) {
        service(projectId).retry()
    }

    override suspend fun windowFocus(projectId: ProjectId) {
        service(projectId).windowFocus()
    }

    override suspend fun blob(projectId: ProjectId, sessionId: String, name: String): Blob? {
        val service = service(projectId)
        return BlobStore.read(service.sessionsDir, sessionId, name)?.let(::Blob)
    }

    override suspend fun uiRequests(projectId: ProjectId): Flow<UiRequest> {
        val project = projectId.findProjectOrNull() ?: error("Acpira backend project is not available: $projectId")
        return project.getService(UiRequestBus::class.java).events()
    }

    // RPC carries the requesting ClientId. Serializing on the sidecar reader thread instead binds to the local backend session,
    // which cannot produce a file handle for the remote client.
    override suspend fun resolveFile(projectId: ProjectId, path: String): ResolvedPath? = withContext(Dispatchers.IO) {
        if (projectId.findProjectOrNull() == null) return@withContext null
        val file = LocalFileSystem.getInstance().refreshAndFindFileByPath(path)
        file?.let { ResolvedPath(it.rpcId(), it.isDirectory) }
    }

    private fun service(projectId: ProjectId): SidecarService {
        val project = projectId.findProjectOrNull() ?: error("Acpira backend project is not available: $projectId")
        return SidecarService.getInstance(project)
    }
}
