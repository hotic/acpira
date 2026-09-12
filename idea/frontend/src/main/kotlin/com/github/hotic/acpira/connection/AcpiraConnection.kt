@file:Suppress("UnstableApiUsage")

package com.github.hotic.acpira.connection

import com.github.hotic.acpira.Acpira
import com.github.hotic.acpira.platform.FrontendUi
import com.github.hotic.acpira.rpc.AcpiraBackendApi
import com.github.hotic.acpira.rpc.SidecarState
import com.github.hotic.acpira.rpc.ViewAttach
import com.github.hotic.acpira.rpc.ViewEvent
import com.google.gson.JsonElement
import com.google.gson.JsonParser
import com.google.gson.JsonPrimitive
import com.intellij.DynamicBundle
import com.intellij.openapi.components.Service
import com.intellij.openapi.project.Project
import com.intellij.platform.project.projectId
import fleet.rpc.client.durable
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import java.util.concurrent.ConcurrentHashMap

// One frontend project connection multiplexes views over native RPC. Each view keeps an unlimited ordered post queue gated by the
// current attach attempt; a 15 s watchdog follows that gate. RPC discovery retries through durable, while other transient failures use
// an outer delay loop. Retrying a post may duplicate one WebviewMsg when its response is lost; protocol v1 accepts that trade-off.
@Service(Service.Level.PROJECT)
class AcpiraConnection(private val project: Project, val cs: CoroutineScope) {
    interface View {
        val viewId: String
        val host: String
        val initial: JsonElement?
        val lastSessionId: String?
        fun onHostMessage(message: JsonElement)
        fun onState(state: SidecarState, detail: String?)
    }

    private data class Binding(val posts: Channel<String>, val job: Job)

    private val bindings = ConcurrentHashMap<String, Binding>()

    init {
        cs.launch {
            retryNonRpc("backend UI request stream") {
                durable {
                    AcpiraBackendApi.getInstance().uiRequests(project.projectId()).collect { FrontendUi.handle(project, it) }
                }
            }
        }
    }

    fun attach(view: View): Job {
        bindings.remove(view.viewId)?.let { old -> old.posts.close(); old.job.cancel() }
        val posts = Channel<String>(Channel.UNLIMITED)
        val attached = MutableStateFlow(false)
        val job = cs.launch {
            coroutineScope {
                launch {
                    attached.collectLatest { ready ->
                        if (!ready) {
                            delay(15_000)
                            view.onState(
                                SidecarState.FAILED,
                                "Acpira's backend part is not reachable. In Remote Development, install Acpira on the remote host as well.",
                            )
                        }
                    }
                }
                launch {
                    for (json in posts) {
                        attached.first { it }
                        // A lost RPC response can make durable send this message twice; v1 prefers possible duplication over silent loss.
                        durable { AcpiraBackendApi.getInstance().post(project.projectId(), view.viewId, json) }
                    }
                }
                retryNonRpc("backend view attach") {
                    durable {
                        attached.value = false
                        val initial = view.lastSessionId?.let { JsonPrimitive(it).toString() } ?: view.initial?.toString()
                        AcpiraBackendApi.getInstance().attach(
                            project.projectId(),
                            ViewAttach(view.viewId, view.host, initial, DynamicBundle.getLocale().toLanguageTag()),
                        ).collect { event ->
                            attached.value = true
                            when (event) {
                                is ViewEvent.Host -> view.onHostMessage(JsonParser.parseString(event.json))
                                is ViewEvent.State -> view.onState(event.state, event.detail)
                            }
                        }
                    }
                }
            }
        }
        val binding = Binding(posts, job)
        bindings[view.viewId] = binding
        job.invokeOnCompletion {
            bindings.remove(view.viewId, binding)
            posts.close()
        }
        return job
    }

    fun post(viewId: String, json: String) {
        if (bindings[viewId]?.posts?.trySend(json)?.isSuccess != true) {
            Acpira.LOG.warn("webview message dropped for detached view $viewId")
        }
    }

    fun retry() {
        cs.launch { runCatching { AcpiraBackendApi.getInstance().retry(project.projectId()) }.onFailure { Acpira.LOG.warn("backend retry failed", it) } }
    }

    fun windowFocus() {
        cs.launch { runCatching { AcpiraBackendApi.getInstance().windowFocus(project.projectId()) }.onFailure { Acpira.LOG.warn("backend focus event failed", it) } }
    }

    suspend fun blob(sessionId: String, name: String): ByteArray? =
        AcpiraBackendApi.getInstance().blob(project.projectId(), sessionId, name)?.bytes

    private suspend fun retryNonRpc(label: String, block: suspend () -> Unit) {
        while (currentCoroutineContext().isActive) {
            try {
                block()
                delay(3_000)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Throwable) {
                Acpira.LOG.warn("$label failed; retrying", e)
                delay(3_000)
            }
        }
    }
}
