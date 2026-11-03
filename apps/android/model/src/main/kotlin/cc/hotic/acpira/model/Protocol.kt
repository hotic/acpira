package cc.hotic.acpira.model

import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonObjectBuilder
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

// Mirror of packages/shared/protocol.ts: what a page receives (HostMsg) and sends (WebviewMsg). The page messages the
// phone does not use are left out: an unknown HostMsg decodes to HostMsg.Unknown and is skipped.

/** A user-made group of sessions inside one project */
@Serializable
data class SessionCategory(val id: String, val name: String = "", val cwd: String = "", val collapsed: Boolean? = null)

/** A user-made top-level section of a project shell's list, holding whole projects */
@Serializable
data class SessionSection(val id: String, val name: String = "", val projects: List<String> = emptyList(), val collapsed: Boolean? = null)

/** Every category in list order plus the project list's own state; shared by every window on the machine */
@Serializable
data class SessionCategories(
  val categories: List<SessionCategory> = emptyList(),
  val collapsedProjects: List<String> = emptyList(),
  /** Project folders in the order a project shell lists them */
  val projects: List<String>? = null,
  val removedProjects: List<String>? = null,
  val sections: List<SessionSection>? = null,
)

/** The settings the engine pushes (settings.ts SettingsView), the subset the phone reads */
@Serializable
data class SettingsView(
  val language: String = "auto",
  val locale: String = "en",
  val defaultAgent: AgentId = "acpira",
  val agentOrder: List<AgentId> = emptyList(),
  val disabledAgents: List<AgentId> = emptyList(),
  val theme: String = "auto",
  val steerQueued: Boolean = false,
  val notifySound: Boolean = true,
  val notifySystem: Boolean = false,
)

/** Everything a page needs to start (protocol.ts InitState), sent once after `ready` */
@Serializable
data class InitState(
  val host: String = "sidebar",
  val appearance: Map<String, String> = emptyMap(),
  val agents: List<AgentInfo> = emptyList(),
  val accounts: List<AccountInfo> = emptyList(),
  val sessions: List<SessionSummary> = emptyList(),
  val categories: SessionCategories? = null,
  /** The session the view opens on, whole: the base the next sessionPatch applies to */
  val active: SessionView? = null,
  val settings: SettingsView = SettingsView(),
  val locale: String = "en",
  /** The engine machine's home directory and the view's folder, for shortening paths */
  val home: String = "",
  val cwd: String = "",
  /** Where an attachment blob loads from: `${blobBase}/${sessionId}/${blob}` */
  val blobBase: String? = null,
  /** Where chats' folders live: a session whose cwd is directly inside is a chat, not a project */
  val chats: String? = null,
)

/**
 * The changes since the session view the page was sent last (sessionPatch.ts): keep `turns[0 until keep]` of the view
 * at rev `base` and append `turns`; with `keepBlocks`, `turns[0]` is the head of turn `keep` (its blocks empty) and that
 * turn's blocks are the base turn's first `keepBlocks` followed by `blocks`. `view` is the whole view but its turns
 */
@Serializable
data class SessionPatch(
  val id: String,
  val base: Int,
  val view: SessionView,
  val keep: Int,
  val turns: List<Turn> = emptyList(),
  val keepBlocks: Int? = null,
  val blocks: List<AgentBlock>? = null,
)

/** Engine → page. Only what the phone reads is decoded; the rest arrives as [Unknown] */
sealed interface HostMsg {
  data class Init(val state: InitState) : HostMsg
  data class Agents(val agents: List<AgentInfo>) : HostMsg
  data class Sessions(val sessions: List<SessionSummary>) : HostMsg
  data class Categories(val categories: SessionCategories) : HostMsg
  data object NoSession : HostMsg
  data class Session(val session: SessionView) : HostMsg
  data class Patch(val patch: SessionPatch) : HostMsg
  data class Accounts(val accounts: List<AccountInfo>) : HostMsg
  data class Settings(val settings: SettingsView, val locale: String) : HostMsg
  data class Controls(val agent: AgentId, val controls: List<ConfigControl>, val commands: List<SlashCommand>) : HostMsg
  data class EditTurnResult(val requestId: String, val error: String?) : HostMsg
  data class Unknown(val type: String) : HostMsg

  companion object {
    /** The `type` literals this build reads */
    val types = listOf("init", "agents", "sessions", "categories", "noSession", "session", "sessionPatch", "accounts", "settings", "controls", "editTurnResult")

    fun decode(json: Json, obj: JsonObject): HostMsg {
      val type = obj.string("type") ?: return Unknown("")
      fun <T> field(key: String, s: kotlinx.serialization.KSerializer<T>): T =
        json.decodeFromJsonElement(s, obj[key] ?: throw IllegalArgumentException("$type without $key"))
      return when (type) {
        "init" -> Init(field("state", InitState.serializer()))
        "agents" -> Agents(field("agents", kotlinx.serialization.builtins.ListSerializer(AgentInfo.serializer())))
        "sessions" -> Sessions(field("sessions", kotlinx.serialization.builtins.ListSerializer(SessionSummary.serializer())))
        "categories" -> Categories(field("categories", SessionCategories.serializer()))
        "noSession" -> NoSession
        "session" -> Session(field("session", SessionView.serializer()))
        "sessionPatch" -> Patch(field("patch", SessionPatch.serializer()))
        "accounts" -> Accounts(field("accounts", kotlinx.serialization.builtins.ListSerializer(AccountInfo.serializer())))
        "settings" -> Settings(field("settings", SettingsView.serializer()), obj.string("locale") ?: "en")
        "controls" -> Controls(
          obj.string("agent") ?: "",
          field("controls", kotlinx.serialization.builtins.ListSerializer(ConfigControl.serializer())),
          obj["commands"]?.let { json.decodeFromJsonElement(kotlinx.serialization.builtins.ListSerializer(SlashCommand.serializer()), it) }.orEmpty(),
        )
        "editTurnResult" -> EditTurnResult(obj.string("requestId") ?: "", obj.string("error"))
        else -> Unknown(type)
      }
    }
  }
}

/** An attachment as the composer sends it (protocol.ts Draft); the phone sends images and pasted text */
sealed interface Draft {
  data class Image(val mimeType: String, val data: String, val name: String? = null) : Draft
  data class Text(val name: String, val text: String) : Draft

  fun toJson(): JsonObject = when (this) {
    is Image -> buildJsonObject { put("kind", "image"); put("mimeType", mimeType); put("data", data); name?.let { put("name", it) } }
    is Text -> buildJsonObject { put("kind", "text"); put("name", name); put("text", text) }
  }
}

/**
 * Page → engine: the actions the phone sends, wrapped in a webviewMessage envelope. A session action carries the id of
 * the session the view shows; the engine refuses one for another session. clientId (send, stop, permission, answer,
 * steerQueued): the engine running the session runs each id once, so a resend after a lost link is safe
 */
sealed interface WebviewMsg {
  /** The page is mounted: the engine answers with `init`. patches: the page applies `sessionPatch` */
  data class Ready(val patches: Boolean = true) : WebviewMsg
  data object Resync : WebviewMsg
  data class Send(val sessionId: String?, val text: String, val attachments: List<Draft>? = null, val steer: Boolean? = null, override val clientId: String? = null) : WebviewMsg
  data class Stop(val sessionId: String?, override val clientId: String? = null) : WebviewMsg
  data class Permission(val sessionId: String, val blockId: String, val optionId: String, override val clientId: String? = null) : WebviewMsg
  data class Answer(val sessionId: String, val blockId: String, val answers: Map<String, JsonElement>, val skip: Boolean? = null, override val clientId: String? = null) : WebviewMsg
  data class SetMode(val sessionId: String?, val id: String) : WebviewMsg
  data class SetConfig(val sessionId: String?, val configId: String, val value: String) : WebviewMsg
  data class SelectSession(val id: String) : WebviewMsg
  /** The list is on top or the app went to the background: the chat no longer counts as watched */
  data class DeselectSession(val id: String) : WebviewMsg
  data class NewSession(val agent: AgentId? = null, val cwd: String? = null, val chat: Boolean? = null) : WebviewMsg
  data class RenameSession(val id: String, val title: String) : WebviewMsg
  data class DeleteSession(val id: String) : WebviewMsg
  data class PinSession(val id: String, val pinned: Boolean) : WebviewMsg
  data class ArchiveSession(val id: String, val archived: Boolean) : WebviewMsg
  data class Dequeue(val sessionId: String, val id: String) : WebviewMsg
  data class SendQueued(val sessionId: String, val id: String) : WebviewMsg
  data class Controls(val agent: AgentId, val fresh: Boolean? = null) : WebviewMsg
  data class Retry(val sessionId: String?) : WebviewMsg
  data class Reconnect(val sessionId: String?) : WebviewMsg
  /** One engine setting by its SettingsView key; the engine answers with `settings` to every window */
  data class SetSetting(val key: String, val value: JsonElement) : WebviewMsg

  /** The wire `type` */
  val type: String
    get() = when (this) {
      is Ready -> "ready"
      Resync -> "resync"
      is Send -> "send"
      is Stop -> "stop"
      is Permission -> "permission"
      is Answer -> "answer"
      is SetMode -> "setMode"
      is SetConfig -> "setConfig"
      is SelectSession -> "selectSession"
      is DeselectSession -> "deselectSession"
      is NewSession -> "newSession"
      is RenameSession -> "renameSession"
      is DeleteSession -> "deleteSession"
      is PinSession -> "pinSession"
      is ArchiveSession -> "archiveSession"
      is Dequeue -> "dequeue"
      is SendQueued -> "sendQueued"
      is Controls -> "controls"
      is Retry -> "retry"
      is Reconnect -> "reconnect"
      is SetSetting -> "setSetting"
    }

  /** The id the engine dedupes this action by; null for actions that carry none */
  val clientId: String? get() = null

  /** The action with a fresh clientId where it can carry one and has none yet; a resend keeps the id it was given */
  fun stamped(id: () -> String): WebviewMsg = when (this) {
    is Send -> if (clientId == null) copy(clientId = id()) else this
    is Stop -> if (clientId == null) copy(clientId = id()) else this
    is Permission -> if (clientId == null) copy(clientId = id()) else this
    is Answer -> if (clientId == null) copy(clientId = id()) else this
    else -> this
  }

  /** Worth sending again on the next wire: what the user did to a session */
  val replayable: Boolean
    get() = this is Send || this is Stop || this is Permission || this is Answer || this is SendQueued || this is SetMode || this is SetConfig

  fun toJson(): JsonObject = buildJsonObject {
    put("type", type)
    when (val m = this@WebviewMsg) {
      is Ready -> put("patches", m.patches)
      Resync -> {}
      is Send -> {
        opt("sessionId", m.sessionId); put("text", m.text)
        m.attachments?.let { a -> put("attachments", buildJsonArray { a.forEach { add(it.toJson()) } }) }
        m.steer?.let { put("steer", it) }; opt("clientId", m.clientId)
      }
      is Stop -> { opt("sessionId", m.sessionId); opt("clientId", m.clientId) }
      is Permission -> { put("sessionId", m.sessionId); put("blockId", m.blockId); put("optionId", m.optionId); opt("clientId", m.clientId) }
      is Answer -> {
        put("sessionId", m.sessionId); put("blockId", m.blockId); put("answers", JsonObject(m.answers))
        m.skip?.let { put("skip", it) }; opt("clientId", m.clientId)
      }
      is SetMode -> { opt("sessionId", m.sessionId); put("id", m.id) }
      is SetConfig -> { opt("sessionId", m.sessionId); put("configId", m.configId); put("value", m.value) }
      is SelectSession -> put("id", m.id)
      is DeselectSession -> put("id", m.id)
      is NewSession -> { opt("agent", m.agent); opt("cwd", m.cwd); m.chat?.let { put("chat", it) } }
      is RenameSession -> { put("id", m.id); put("title", m.title) }
      is DeleteSession -> put("id", m.id)
      is PinSession -> { put("id", m.id); put("pinned", m.pinned) }
      is ArchiveSession -> { put("id", m.id); put("archived", m.archived) }
      is Dequeue -> { put("sessionId", m.sessionId); put("id", m.id) }
      is SendQueued -> { put("sessionId", m.sessionId); put("id", m.id) }
      is Controls -> { put("agent", m.agent); m.fresh?.let { put("fresh", it) } }
      is Retry -> opt("sessionId", m.sessionId)
      is Reconnect -> opt("sessionId", m.sessionId)
      is SetSetting -> { put("key", m.key); put("value", m.value) }
    }
  }

  companion object {
    /** The `type` literals this build sends */
    val types = listOf(
      "ready", "resync", "send", "stop", "permission", "answer", "setMode", "setConfig", "selectSession", "deselectSession",
      "newSession", "renameSession", "deleteSession", "pinSession", "archiveSession", "dequeue", "sendQueued", "controls",
      "retry", "reconnect", "setSetting",
    )
  }
}

private fun JsonObjectBuilder.opt(key: String, value: String?) {
  if (value != null) put(key, JsonPrimitive(value))
}
