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
data class ClientInfo(val name: String, val version: String, val capabilities: List<String> = emptyList())

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
  val sessionsDir: String = "",
  val features: List<String>? = null,
)

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

/** Client → engine (ShellMsg), as written */
sealed interface ShellMsg {
  data class HelloMsg(val hello: Hello) : ShellMsg
  data class AttachView(val viewId: String, val host: String = "sidebar", val initial: InitialView? = null) : ShellMsg
  data class WebviewMessage(val viewId: String, val message: WebviewMsg) : ShellMsg
  /** The answer to a platformRequest that carried a requestId */
  data class PlatformResponse(val requestId: String, val error: String? = null) : ShellMsg

  fun toJson(json: Json = WireJson): JsonObject = when (this) {
    is HelloMsg -> JsonObject(mapOf("type" to JsonPrimitive("hello")) + json.encodeToJsonElement(Hello.serializer(), hello).jsonObject)
    is AttachView -> buildJsonObject {
      put("type", "attachView"); put("viewId", viewId); put("host", host)
      initial?.let { put("initial", it.toJson()) }
    }
    is WebviewMessage -> buildJsonObject { put("type", "webviewMessage"); put("viewId", viewId); put("message", message.toJson()) }
    is PlatformResponse -> buildJsonObject {
      put("type", "platformResponse"); put("requestId", requestId)
      error?.let { put("error", it) }
    }
  }
}

/** An IDE action the engine asks the client for (PlatformRequest); the phone carries out toast and notify */
sealed interface PlatformRequest {
  data class Toast(val level: String, val text: String) : PlatformRequest
  data class Notify(val sessionId: String, val title: String, val body: String) : PlatformRequest
  data class Unknown(val method: String) : PlatformRequest
}

/** Engine → client (SidecarMsg) */
sealed interface SidecarMsg {
  data class HelloOkMsg(val ok: HelloOk) : SidecarMsg
  /** The engine refused the wire (another protocol version, shutting down) and closes it */
  data class HelloReject(val protocolVersion: Int, val reason: String) : SidecarMsg
  data class HostMessage(val viewId: String, val message: HostMsg) : SidecarMsg
  /** requestId: present when the engine waits for a platformResponse */
  data class Platform(val requestId: String?, val request: PlatformRequest) : SidecarMsg
  data class Unknown(val type: String) : SidecarMsg

  companion object {
    /** One envelope line; throws when it is not JSON or a known envelope misses what it must carry */
    fun decode(line: String, json: Json = WireJson): SidecarMsg {
      val obj = json.parseToJsonElement(line).jsonObject
      return when (val type = obj.string("type")) {
        "helloOk" -> HelloOkMsg(json.decodeFromJsonElement(HelloOk.serializer(), obj))
        "helloReject" -> HelloReject(obj.int("protocolVersion") ?: 0, obj.string("reason") ?: "")
        "hostMessage" -> HostMessage(
          obj.string("viewId") ?: "",
          HostMsg.decode(json, obj["message"]?.jsonObject ?: throw IllegalArgumentException("hostMessage without message")),
        )
        "platformRequest" -> {
          val r = obj["request"]?.jsonObject ?: JsonObject(emptyMap())
          val request = when (val method = r.string("method")) {
            "toast" -> PlatformRequest.Toast(r.string("level") ?: "info", r.string("text") ?: "")
            "notify" -> PlatformRequest.Notify(r.string("sessionId") ?: "", r.string("title") ?: "", r.string("body") ?: "")
            else -> PlatformRequest.Unknown(method ?: "")
          }
          Platform(obj.string("requestId"), request)
        }
        else -> Unknown(type ?: "")
      }
    }
  }
}
