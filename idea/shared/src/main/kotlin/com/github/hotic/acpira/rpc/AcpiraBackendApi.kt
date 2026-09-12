@file:Suppress("UnstableApiUsage")

package com.github.hotic.acpira.rpc

import com.intellij.ide.vfs.VirtualFileId
import com.intellij.platform.project.ProjectId
import com.intellij.platform.rpc.RemoteApiProviderService
import fleet.rpc.RemoteApi
import fleet.rpc.Rpc
import fleet.rpc.core.Blob
import fleet.rpc.remoteApiDescriptor
import kotlinx.coroutines.flow.Flow
import kotlinx.serialization.Serializable

// Shared split-mode contract. `attach` emits ordered events until its collector disappears, which detaches the backend view; `initial`
// carries the frontend's remembered last session on reconnection. UI requests use a second Flow because 261's topic listener is internal.
@Serializable
enum class SidecarState { STARTING, READY, FAILED, STOPPED }

@Serializable
data class ViewAttach(
    val viewId: String,
    val host: String,
    val initial: String?,
    val locale: String,
)

@Serializable
sealed interface ViewEvent {
    @Serializable
    data class Host(val json: String) : ViewEvent

    @Serializable
    data class State(val state: SidecarState, val detail: String?) : ViewEvent
}

// Remote VFS handles do not preserve directory metadata on every Client implementation.
@Serializable
data class ResolvedPath(val id: VirtualFileId, val directory: Boolean)

@Rpc
interface AcpiraBackendApi : RemoteApi<Unit> {
    suspend fun attach(projectId: ProjectId, view: ViewAttach): Flow<ViewEvent>
    suspend fun post(projectId: ProjectId, viewId: String, json: String)
    suspend fun retry(projectId: ProjectId)
    suspend fun windowFocus(projectId: ProjectId)
    suspend fun blob(projectId: ProjectId, sessionId: String, name: String): Blob?
    suspend fun uiRequests(projectId: ProjectId): Flow<UiRequest>
    suspend fun resolveFile(projectId: ProjectId, path: String): ResolvedPath?

    companion object {
        suspend fun getInstance(): AcpiraBackendApi = RemoteApiProviderService.resolve(remoteApiDescriptor<AcpiraBackendApi>())
    }
}

@Serializable
sealed interface UiRequest {
    @Serializable
    data class Toast(val text: String, val error: Boolean) : UiRequest

    @Serializable
    data class OpenExternal(val url: String) : UiRequest

    @Serializable
    data class OpenInEditor(val sessionId: String?) : UiRequest

    @Serializable
    data class OpenFile(val path: String, val line: Int?) : UiRequest

    @Serializable
    data class OpenPlan(val path: String?, val markdown: String?) : UiRequest
}
