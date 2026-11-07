package cc.hotic.acpira.model

import kotlinx.serialization.EncodeDefault
import kotlinx.serialization.ExperimentalSerializationApi
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.put

// The envelopes (packages/shared/sidecar.ts): one JSON object per text frame over the harness WebSocket. A client says
// hello, waits for helloOk, attaches a view, then wraps every WebviewMsg in webviewMessage and receives every HostMsg as
// hostMessage.

object SidecarProtocol {
  /** Must equal SIDECAR_PROTOCOL_VERSION in packages/shared/sidecar.ts; the engine rejects any other value at hello */
  const val VERSION = 1
}

/** Facts about the client, sent with hello (ShellEnv) */
@Serializable
data class ShellEnv(
  val cwd: String? = null,
  val hostLanguage: String,
  /** Where views load attachment blobs from: `${blobBase}/${sessionId}/${blob}` */
  val blobBase: String? = null,
  /** The client organizes sessions by project folder, so no session starts on its own without one */
  val projects: Boolean? = null,
)

/** The client in hello */
@Serializable
data class ClientInfo(
  val name: String,
  val version: String,
  /**
   * Platform methods this client carries out (PlatformRequest methods); the engine sends no request outside this list,
   * and falls back to its own handling where it has one
   */
  val capabilities: List<String> = emptyList(),
  /** The shell process's SSH_CONNECTION, for a window of a remote workspace */
  val sshConnection: String? = null,
  /** `peer`: another engine, never a window (docs/dev/remote.md). A plain wire leaves it unset */
  val role: String? = null,
)

/** The first envelope on a wire */
@OptIn(ExperimentalSerializationApi::class)
@Serializable
data class Hello(
  // Always on the wire: WireJson leaves defaults out, and the engine refuses a hello without it
  @EncodeDefault val protocolVersion: Int = SidecarProtocol.VERSION,
  val requestId: String,
  val client: ClientInfo,
  val env: ShellEnv,
  val settings: Map<String, JsonElement> = emptyMap(),
)

/** The engine's answer to hello */
@Serializable
data class HelloOk(
  val requestId: String = "",
  val protocolVersion: Int = 0,
  /** The engine's sessions directory on its own machine; blobs live at `<sessionsDir>/<sessionId>/<blob>` */
  val sessionsDir: String = "",
  /**
   * Additions usable without a protocol bump (`projects`, `remote`, `localDir`, `providers`, `remoteAccess` …); absent on
   * older engines
   */
  val features: List<String>? = null,
  val sidecar: Sidecar? = null,
  /** Every user plugin is unloaded (docs/dev/plugins.md) */
  val safeMode: Boolean? = null,
) {
  @Serializable
  data class Sidecar(val version: String = "", val pid: Int = 0)

  /** Whether the engine announced [feature] */
  fun has(feature: String): Boolean = features?.contains(feature) == true
}

/** What a view opens on (attachView `initial`) */
sealed interface InitialView {
  data class Session(val id: String) : InitialView
  data object MostRecent : InitialView
  /** Nothing until a session is selected: what a phone opening on its list wants */
  data object Empty : InitialView

  fun toJson(): JsonElement = when (this) {
    is Session -> JsonPrimitive(id)
    MostRecent -> buildJsonObject { put("mostRecent", true) }
    Empty -> buildJsonObject { put("empty", true) }
  }
}

/**
 * Client → engine (ShellMsg), as written. Not mirrored, as on iOS: platformEvent (a window's focus, settings and
 * environment) and previewOpen / previewClose (the desktop browser panel)
 */
sealed interface ShellMsg {
  data class HelloMsg(val hello: Hello) : ShellMsg
  /** blobBase: this view's own blob base; the env's when absent */
  data class AttachView(val viewId: String, val host: String = "sidebar", val initial: InitialView? = null, val blobBase: String? = null) : ShellMsg
  data class DetachView(val viewId: String) : ShellMsg
  data class WebviewMessage(val viewId: String, val message: WebviewMsg) : ShellMsg
  /** The answer to a platformRequest that carried a requestId */
  data class PlatformResponse(val requestId: String, val error: String? = null, val result: JsonElement? = null) : ShellMsg
  data object Shutdown : ShellMsg

  fun toJson(json: Json = WireJson): JsonObject = when (this) {
    is HelloMsg -> JsonObject(mapOf("type" to JsonPrimitive("hello")) + json.encodeToJsonElement(Hello.serializer(), hello).jsonObject)
    is AttachView -> buildJsonObject {
      put("type", "attachView"); put("viewId", viewId); put("host", host)
      initial?.let { put("initial", it.toJson()) }
      blobBase?.let { put("blobBase", it) }
    }
    is DetachView -> buildJsonObject { put("type", "detachView"); put("viewId", viewId) }
    is WebviewMessage -> buildJsonObject { put("type", "webviewMessage"); put("viewId", viewId); put("message", message.toJson()) }
    is PlatformResponse -> buildJsonObject {
      put("type", "platformResponse"); put("requestId", requestId)
      result?.let { put("result", it) }
      error?.let { put("error", it) }
    }
    Shutdown -> buildJsonObject { put("type", "shutdown") }
  }

  companion object {
    /** The `type` literals this build writes */
    val types = listOf("hello", "attachView", "detachView", "webviewMessage", "platformResponse", "shutdown")
  }
}

/**
 * An IDE action the engine asks the client for (PlatformRequest). It only sends methods hello's capabilities name; one
 * with a requestId waits for a platformResponse. The phone carries out toast and notify
 */
sealed interface PlatformRequest {
  /** `open` adds a button for that file (label already localized) */
  data class Toast(val level: String, val text: String, val open: ToastOpen? = null) : PlatformRequest
  /**
   * A session needs the user or has settled: a chime (`sound`) and / or a system notification (`system`), each only when
   * its setting is on; `open` is the label of the button that brings the session up
   */
  data class Notify(
    val sessionId: String,
    val title: String,
    val body: String,
    val open: String = "",
    val sound: Boolean = false,
    val system: Boolean = false,
  ) : PlatformRequest
  data class Unknown(val method: String) : PlatformRequest

  @Serializable
  data class ToastOpen(val label: String = "", val path: String = "")

  companion object {
    /** The `method` literals this build carries out */
    val methods = listOf("toast", "notify")
  }
}

/** Engine → client (SidecarMsg) */
sealed interface SidecarMsg {
  data class HelloOkMsg(val ok: HelloOk) : SidecarMsg
  /** The engine refused the wire (another protocol version, shutting down, its runtime failing to start) and closes it */
  data class HelloReject(val protocolVersion: Int, val reason: String, val requestId: String = "") : SidecarMsg
  data class HostMessage(val viewId: String, val message: HostMsg) : SidecarMsg
  /** requestId: present when the engine waits for a platformResponse */
  data class Platform(val requestId: String?, val request: PlatformRequest) : SidecarMsg
  data object ShutdownOk : SidecarMsg
  data class Unknown(val type: String) : SidecarMsg

  companion object {
    /** The `type` literals this build reads */
    val types = listOf("helloOk", "helloReject", "hostMessage", "platformRequest", "shutdownOk")

    /** One envelope line; throws when it is not JSON or a known envelope misses what it must carry */
    fun decode(line: String, json: Json = WireJson): SidecarMsg {
      val obj = json.parseToJsonElement(line).jsonObject
      return when (val type = obj.string("type")) {
        "helloOk" -> HelloOkMsg(json.decodeFromJsonElement(HelloOk.serializer(), obj))
        "helloReject" -> HelloReject(obj.int("protocolVersion") ?: 0, obj.string("reason") ?: "", obj.string("requestId") ?: "")
        "hostMessage" -> HostMessage(
          obj.string("viewId") ?: "",
          HostMsg.decode(json, obj["message"]?.jsonObject ?: throw IllegalArgumentException("hostMessage without message")),
        )
        "platformRequest" -> {
          val r = obj["request"]?.jsonObject ?: JsonObject(emptyMap())
          val request = when (val method = r.string("method")) {
            "toast" -> PlatformRequest.Toast(
              r.string("level") ?: "info", r.string("text") ?: "",
              r["open"]?.let { runCatching { json.decodeFromJsonElement(PlatformRequest.ToastOpen.serializer(), it) }.getOrNull() },
            )
            "notify" -> PlatformRequest.Notify(
              r.string("sessionId") ?: "", r.string("title") ?: "", r.string("body") ?: "", r.string("open") ?: "",
              r.bool("sound") ?: false, r.bool("system") ?: false,
            )
            else -> PlatformRequest.Unknown(method ?: "")
          }
          Platform(obj.string("requestId"), request)
        }
        "shutdownOk" -> ShutdownOk
        else -> Unknown(type ?: "")
      }
    }
  }
}
