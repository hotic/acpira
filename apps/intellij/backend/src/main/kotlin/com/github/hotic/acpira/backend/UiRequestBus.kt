package com.github.hotic.acpira.backend

import com.github.hotic.acpira.rpc.UiRequest
import com.intellij.openapi.components.Service
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.buffer
import kotlinx.coroutines.flow.callbackFlow
import java.util.concurrent.CopyOnWriteArraySet

// Project-local fan-out for backend requests that must execute in the frontend process. Subscribers own unlimited ordered buffers and
// disappear with their RPC flow, avoiding the internal ProjectRemoteTopicListener API on 261.
@Service(Service.Level.PROJECT)
class UiRequestBus {
    private val listeners = CopyOnWriteArraySet<(UiRequest) -> Unit>()

    fun send(request: UiRequest) {
        for (listener in listeners) listener(request)
    }

    fun events(): Flow<UiRequest> = callbackFlow {
        val listener: (UiRequest) -> Unit = { trySend(it) }
        listeners += listener
        awaitClose { listeners -= listener }
    }.buffer(Channel.UNLIMITED)
}
