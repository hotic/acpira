@file:Suppress("EnumEntryName")

package cc.hotic.acpira.model

import kotlinx.serialization.KSerializer
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.builtins.MapSerializer
import kotlinx.serialization.builtins.serializer
import kotlinx.serialization.descriptors.PrimitiveKind
import kotlinx.serialization.descriptors.PrimitiveSerialDescriptor
import kotlinx.serialization.descriptors.SerialDescriptor
import kotlinx.serialization.encoding.Decoder
import kotlinx.serialization.encoding.Encoder
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonDecoder
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonEncoder
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonObjectBuilder
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.add
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put

// Mirror of packages/shared/protocol.ts: what a page receives (HostMsg) and sends (WebviewMsg), as the subset the iOS
// app mirrors (apps/ios AcpiraModels/Protocol.swift). An unknown HostMsg decodes to HostMsg.Unknown and is skipped.

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

/** A cross-harness subagent from the settings (subagents.ts SubagentPersona): summoned through `ask_agent` or `@name` */
@Serializable
data class SubagentPersona(
  /** Stable slug: the tool's `agent` value */
  val id: String,
  val name: String = "",
  /** The CLI it runs in */
  val agent: AgentId = "",
  /** A value of that agent's model select; absent: the CLI's default */
  val model: String? = null,
  /** A value of its reasoning-effort select; absent: the default */
  val effort: String? = null,
  /** Its Fast switch (only the built-in agent's sidekick sets it); absent: left as the CLI has it */
  val fast: Boolean? = null,
  val mode: RelayMode = RelayMode.consult,
  /** Tells the model when to summon it (`when` on the wire) */
  @SerialName("when") val summonWhen: String = "",
  val brief: String? = null,
  val enabled: Boolean = true,
  /** general | scout | executor | reviewer; absent: general */
  val duty: String? = null,
  /** Every new conversation starts with the handoff pack of the session's changes */
  val changes: Boolean? = null,
)

/** Personas summoned together by one `@name` (subagents.ts SubagentGroup) */
@Serializable
data class SubagentGroup(
  val id: String,
  val name: String = "",
  /** Persona ids, in the order their rounds start */
  val members: List<String> = emptyList(),
  @SerialName("when") val summonWhen: String = "",
  val enabled: Boolean = true,
)

/**
 * The settings the engine pushes (settings.ts SettingsView), the subset the phone reads. Not mirrored, as on iOS: the
 * desktop / IDE-only keys (sessionScope, font sizes, proxy, panel* …)
 */
@Serializable
data class SettingsView(
  val language: String = "auto",
  val locale: String = "en",
  val defaultAgent: AgentId = "acpira",
  val agentOrder: List<AgentId> = emptyList(),
  val disabledAgents: List<AgentId> = emptyList(),
  val autoCompact: Boolean = true,
  val compactAtTokens: Int = 300_000,
  /** agent → control id → hidden model family keys (models.ts keys, shared with the desktop) */
  val hiddenOptions: Map<AgentId, Map<String, List<String>>> = emptyMap(),
  /** off | earliestReset | mostRemaining | listOrder */
  val accountSwitch: String = "off",
  val theme: String = "auto",
  val steerQueued: Boolean = false,
  val notifySound: Boolean = true,
  val notifySystem: Boolean = false,
  /** Agents whose Plan mode approves tool requests without a card */
  val planAutoApprove: List<AgentId> = emptyList(),
  /** The cross-harness subagents and their groups (~/.acpira/subagents.json): the composer's `@` summons */
  val subagents: List<SubagentPersona> = emptyList(),
  val subagentGroups: List<SubagentGroup> = emptyList(),
)

/** How an account action comes in: import the CLI's own login, log in a new one, `auto`, or unlock the credential store */
@Serializable(with = AccountActionVia.S::class)
enum class AccountActionVia { `import`, login, auto, unlock, unknown;
  object S : TolerantEnumSerializer<AccountActionVia>("AccountActionVia", entries, unknown)
}

@Serializable(with = AccountActionStatus.S::class)
enum class AccountActionStatus { pending, success, missing, cancelled, error, unknown;
  object S : TolerantEnumSerializer<AccountActionStatus>("AccountActionStatus", entries, unknown)
}

/** An account import / login / unlock the engine runs, kept engine-side so every page shows the same progress */
@Serializable
data class AccountAction(
  val agent: AgentId = "",
  val via: AccountActionVia = AccountActionVia.unknown,
  val status: AccountActionStatus = AccountActionStatus.unknown,
  val error: String? = null,
)

/** How an account comes in (addAccount): import the CLI's own login, log in a new one in a terminal, or `auto` */
@Serializable(with = AddAccountVia.S::class)
enum class AddAccountVia { `import`, login, auto, unknown;
  object S : TolerantEnumSerializer<AddAccountVia>("AddAccountVia", entries, unknown)
}

/** An export's format: the Markdown for reading, the record as JSON, or plain text holding only what was said */
@Serializable(with = ExportFormat.S::class)
enum class ExportFormat { markdown, json, text, unknown;
  object S : TolerantEnumSerializer<ExportFormat>("ExportFormat", entries, unknown)
}

/** A session whose saved conversation holds every term of a history search, with the text around the first match */
@Serializable
data class SessionHit(val id: String, val snippet: String = "", val summary: SessionSummary? = null)

/** The model a plan is built with (buildPlan `model`): a model control's id and the option picked */
@Serializable
data class ModelPick(val configId: String, val value: String)

/** One hit of the @ file search: the file's URI and the workspace-relative path the list shows */
@Serializable
data class FileHit(val uri: String = "", val path: String = "")

// The reads of a session's project (rust/crates/acpira-host/src/project_files.rs). Paths are relative to the project
// folder with `/` separators

/** An entry of a project folder */
@Serializable
data class ProjectEntry(val name: String = "", val dir: Boolean? = null)

/**
 * A changed stretch of a file as it is now: `add` lines from line `at` (1-based) are new, the `del` lines stood just
 * before line `at` (one past the last line for lines removed at the end)
 */
@Serializable
data class ProjectHunk(val at: Int = 0, val add: Int = 0, val del: List<String>? = null)

/** A file the session changed against what it held before the session's first edit; absolute when outside the project */
@Serializable
data class ProjectChange(val path: String = "", val add: Int = 0, val del: Int = 0, val gone: Boolean? = null)

/** What a file's hunks are measured against: before the session's first edit of it, or the last commit */
@Serializable(with = ProjectBase.S::class)
enum class ProjectBase { session, head, unknown;
  object S : TolerantEnumSerializer<ProjectBase>("ProjectBase", entries, unknown)
}

/** A path's git mark; a folder as a whole ends in `/` */
@Serializable(with = GitMark.S::class)
enum class GitMark(val wire: String) {
  modified("M"), added("A"), deleted("D"), untracked("U"), ignored("I"), unknown("");
  object S : TolerantEnumSerializer<GitMark>("GitMark", entries, unknown, { it.wire })
}

/** Why a project listing or read failed (`ProjectError | string`): a known reason, or the system's own message */
@Serializable(with = ProjectError.S::class)
sealed interface ProjectError {
  /** The wire string */
  val raw: String

  data object NoProject : ProjectError { override val raw = "noProject" }
  data object Outside : ProjectError { override val raw = "outside" }
  data object Missing : ProjectError { override val raw = "missing" }
  data object Binary : ProjectError { override val raw = "binary" }
  data object TooLarge : ProjectError { override val raw = "tooLarge" }
  /** Anything else: the system's own message */
  data class Message(val text: String) : ProjectError { override val raw get() = text }

  companion object {
    /** The reasons this build knows (ProjectError's literals) */
    val reasons: List<ProjectError> = listOf(NoProject, Outside, Missing, Binary, TooLarge)
    fun of(raw: String): ProjectError = reasons.firstOrNull { it.raw == raw } ?: Message(raw)
  }

  object S : KSerializer<ProjectError> {
    override val descriptor: SerialDescriptor = PrimitiveSerialDescriptor("ProjectError", PrimitiveKind.STRING)
    override fun deserialize(decoder: Decoder): ProjectError = of(decoder.decodeString())
    override fun serialize(encoder: Encoder, value: ProjectError) = encoder.encodeString(value.raw)
  }
}

/** How a terminal's shell ended: its exit code, or the signal that ended it */
@Serializable
data class TerminalExit(val code: Int = 0, val signal: String? = null)

/**
 * One edit of the category list, applied by the engine to what is on disk so concurrent windows compose (transcript.ts
 * CategoryOp), as the subset the phone sends: the project list's own ops
 */
sealed interface CategoryOp {
  /** Register a folder at the end of the project list (undoing a removal) */
  data class AddProject(val cwd: String) : CategoryOp
  /** Take a folder off the list; its sessions stay */
  data class RemoveProject(val cwd: String) : CategoryOp
  /** Move a folder before `before`, or to the end */
  data class ReorderProject(val cwd: String, val before: String? = null) : CategoryOp
  data class CollapseProject(val cwd: String, val collapsed: Boolean) : CategoryOp
  /** File a folder into a desktop section before `before` (at its end without one), or take it out (`section` null) */
  data class SetProjectSection(val cwd: String, val section: String? = null, val before: String? = null) : CategoryOp

  /** The wire `op` */
  val op: String
    get() = when (this) {
      is AddProject -> "addProject"
      is RemoveProject -> "removeProject"
      is ReorderProject -> "reorderProject"
      is CollapseProject -> "collapseProject"
      is SetProjectSection -> "setProjectSection"
    }

  /** The op's fields, `op` first; written flat into a categoryOp message */
  fun fields(into: JsonObjectBuilder) {
    into.put("op", op)
    when (this) {
      is AddProject -> into.put("cwd", cwd)
      is RemoveProject -> into.put("cwd", cwd)
      is ReorderProject -> { into.put("cwd", cwd); into.opt("before", before) }
      is CollapseProject -> { into.put("cwd", cwd); into.put("collapsed", collapsed) }
      is SetProjectSection -> { into.put("cwd", cwd); into.opt("section", section); into.opt("before", before) }
    }
  }

  companion object {
    /** The `op` literals this build sends */
    val ops = listOf("addProject", "removeProject", "reorderProject", "collapseProject", "setProjectSection")
  }
}

/**
 * A rewrite of a sent prompt (WebviewMsg editTurn): the user turn at `turnIndex` and everything after it give way to
 * `text` with the kept attachments (by index) and new drafts, sent with `settings`. turnCount, originalText and turnId
 * are the view the page edited; the engine refuses the edit as stale once they no longer match. intent `continue`: keep
 * the native session (which still holds the original) instead of rebuilding the context
 */
data class EditTurnRequest(
  val sessionId: String,
  val turnIndex: Int,
  val turnCount: Int,
  val originalText: String,
  val turnId: String? = null,
  val text: String,
  val retainedAttachments: List<Int> = emptyList(),
  val attachments: List<Draft> = emptyList(),
  val settings: TurnSettings = TurnSettings(),
  val intent: Intent? = null,
) {
  @Serializable(with = Intent.S::class)
  enum class Intent { replace, `continue`, unknown;
    object S : TolerantEnumSerializer<Intent>("EditTurnIntent", entries, unknown)
  }

  fun toJson(): JsonObject = buildJsonObject {
    put("sessionId", sessionId); put("turnIndex", turnIndex); put("turnCount", turnCount); put("originalText", originalText)
    opt("turnId", turnId); put("text", text)
    put("retainedAttachments", buildJsonArray { retainedAttachments.forEach { add(it) } })
    put("attachments", drafts(attachments))
    // TurnSettings.config is always on the wire (TypeScript requires it), even when empty
    put("settings", buildJsonObject {
      opt("modeId", settings.modeId)
      put("config", buildJsonObject { settings.config.forEach { (k, v) -> put(k, v) } })
    })
    intent?.let { put("intent", it.name) }
  }
}

/** Everything a page needs to start (protocol.ts InitState), sent once after `ready`. Not mirrored: agentInstalls */
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
  val accountActions: List<AccountAction>? = null,
  val hidden: HiddenMap = emptyMap(),
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

/**
 * The session list as runs over the list the phone holds (protocol.ts SessionsPatch, acpira-shared sessions_patch.rs).
 * The engine sends it instead of the whole list once `ready` said `listPatches`
 */
@Serializable
data class SessionsPatch(
  /** The list revision the patch applies to and the one it makes: a whole list is 0, each patch one more */
  val base: Int,
  val rev: Int,
  /** Length of the list the patch applies to */
  val size: Int,
  val runs: List<Run> = emptyList(),
) {
  /** One run of the new list: entries copied from the held list (`[from, count]` on the wire), or a new or changed entry */
  @Serializable(with = Run.S::class)
  sealed interface Run {
    data class Keep(val from: Int, val count: Int) : Run
    data class Entry(val summary: SessionSummary) : Run

    object S : KSerializer<Run> {
      override val descriptor: SerialDescriptor = PrimitiveSerialDescriptor("SessionsPatchRun", PrimitiveKind.STRING)
      override fun deserialize(decoder: Decoder): Run {
        val input = decoder as JsonDecoder
        return when (val e = input.decodeJsonElement()) {
          is JsonArray -> {
            require(e.size == 2) { "a kept run is [from, count]" }
            Keep(e[0].jsonPrimitive.int, e[1].jsonPrimitive.int)
          }
          else -> Entry(input.json.decodeFromJsonElement(SessionSummary.serializer(), e))
        }
      }
      override fun serialize(encoder: Encoder, value: Run) {
        val output = encoder as JsonEncoder
        output.encodeJsonElement(when (value) {
          is Keep -> buildJsonArray { add(value.from); add(value.count) }
          is Entry -> output.json.encodeToJsonElement(SessionSummary.serializer(), value.summary)
        })
      }
    }
  }

  /** `held` (at revision `heldRev`) with the patch applied; null when it does not fit (then ask for the whole list) */
  fun apply(held: List<SessionSummary>, heldRev: Int): List<SessionSummary>? {
    if (heldRev != base || held.size != size) return null
    val out = ArrayList<SessionSummary>(size)
    for (run in runs) when (run) {
      is Run.Keep -> {
        if (run.from < 0 || run.count < 0 || run.from > held.size - run.count) return null
        out.addAll(held.subList(run.from, run.from + run.count))
      }
      is Run.Entry -> out.add(run.summary)
    }
    return out
  }
}

/** Engine → page. Only what the phone reads is decoded; the rest arrives as [Unknown] */
sealed interface HostMsg {
  data class Init(val state: InitState) : HostMsg
  data class Agents(val agents: List<AgentInfo>) : HostMsg
  data class Sessions(val sessions: List<SessionSummary>) : HostMsg
  data class Categories(val categories: SessionCategories) : HostMsg
  data object NoSession : HostMsg
  data class Session(val session: SessionView) : HostMsg
  data class Patch(val patch: SessionPatch) : HostMsg
  /** sessionsPatch: the session list as runs over the list sent last; only to pages that said `listPatches` in ready */
  data class ListPatch(val patch: SessionsPatch) : HostMsg
  data class Accounts(val accounts: List<AccountInfo>) : HostMsg
  data class AccountActions(val actions: List<AccountAction>) : HostMsg
  data class Hidden(val hidden: HiddenMap) : HostMsg
  data class Settings(val settings: SettingsView, val locale: String) : HostMsg
  /**
   * An agent's model controls; shapes: its remembered per-model ones; commands: the slash commands its newest session
   * advertised; `cwd`: the request's, echoed (the answer is for a new session in that folder)
   */
  data class Controls(
    val agent: AgentId,
    val controls: List<ConfigControl>,
    val commands: List<SlashCommand>,
    val cwd: String? = null,
    val shapes: ModelShapes? = null,
  ) : HostMsg
  /** The answer to searchSessions with the same seq; a lower seq is stale */
  data class SessionHits(val seq: Int, val hits: List<SessionHit>) : HostMsg
  /** One observed subagent's transcript, pushed to the viewer that asked for it; rev grows with every change */
  data class Subagent(val sessionId: String, val subagentId: String, val rev: Int, val running: Boolean, val turns: List<Turn>) : HostMsg {
    val transcript: SubagentTranscript get() = SubagentTranscript(rev, running, turns)
  }
  /** The answer to searchFiles with the same seq */
  data class Files(val seq: Int, val files: List<FileHit>) : HostMsg
  /** The answer to diffSource: the whole files behind one diff, absent when the session or the diff is gone */
  data class DiffSource(val seq: Int, val source: FullDiffSource?) : HostMsg
  /** The answer to imageBlob: the session blob name for the image file, absent when unreadable */
  data class ImageBlob(val seq: Int, val blob: String?) : HostMsg
  /** The answer to stageFile: the staged file's session blob name and size in bytes, or why not */
  data class StagedFile(val seq: Int, val blob: String?, val size: Long?, val error: ProjectError?) : HostMsg
  /** The answer to sessionMarkdown: the session as Markdown (or the format asked for), or why it could not be read */
  data class SessionMarkdown(val seq: Int, val markdown: String?, val error: String?) : HostMsg
  /** The answers to the project reads, each echoing its request's seq */
  data class ProjectDir(val seq: Int, val entries: List<ProjectEntry>, val error: ProjectError?) : HostMsg
  /** The project folder and the git marks by relative path, empty outside a work tree */
  data class ProjectStatus(val seq: Int, val root: String?, val marks: Map<String, GitMark>) : HostMsg
  data class ProjectFile(val seq: Int, val text: String?, val hunks: List<ProjectHunk>?, val error: ProjectError?) : HostMsg
  /** The files the session's edits left changed, in the order it first edited them */
  data class SessionChanges(val seq: Int, val files: List<ProjectChange>) : HostMsg
  /** A terminal's output as it comes, to every page viewing it */
  data class TerminalOutput(val id: String, val data: String) : HostMsg
  /**
   * A terminal's state. Answering terminalOpen it carries the size, the title, the output so far (`replay`, shown instead
   * of what the page had) and the exit if the shell ended; later only what changed. error: the shell could not start
   */
  data class TerminalState(
    val id: String,
    val title: String? = null,
    val cols: Int? = null,
    val rows: Int? = null,
    val replay: String? = null,
    val exit: TerminalExit? = null,
    val error: String? = null,
  ) : HostMsg
  data class EditTurnResult(val requestId: String, val error: String?) : HostMsg
  /** Only a completed deletion whose engine holds the undo may offer Restore */
  data class DeleteSessionResult(val requestId: String, val undo: Boolean, val error: String?) : HostMsg
  /** The appearance axes changed (axis → value, every value a string) */
  data class Appearance(val appearance: Map<String, String>) : HostMsg
  /** The SSH hosts, after `connections` was asked and on every change since */
  data class Connections(val connections: List<ConnectionView>) : HostMsg
  /** A folder of the engine's computer for the add-project picker (helloOk.features 'localDir'); `parent` absent at a root */
  data class LocalDir(val seq: Int, val path: String, val parent: String?, val entries: List<DirEntry>, val error: String?) : HostMsg
  /** The model sources (helloOk.features 'providers'), also the answer to every providerAction; `error`: that action failed */
  data class Providers(val view: ProvidersView, val error: String?) : HostMsg
  /** The answer to the providerProbe with this id */
  data class ProviderProbed(val id: String, val outcome: ProbeOutcome) : HostMsg
  data class Unknown(val type: String) : HostMsg

  companion object {
    /** The `type` literals this build reads */
    val types = listOf(
      "init", "agents", "sessions", "categories", "noSession", "session", "sessionPatch", "sessionsPatch", "accounts",
      "accountActions", "hidden", "settings", "controls", "sessionHits", "subagent", "files", "diffSource", "imageBlob",
      "stagedFile", "sessionMarkdown", "projectDir", "projectStatus", "projectFile", "sessionChanges", "terminalOutput",
      "terminalState", "editTurnResult", "deleteSessionResult", "appearance", "connections", "localDir", "providers",
      "providerProbed",
    )

    /**
     * One push. A payload the message cannot do without (the view of `session`, a whole list that replaces the held one)
     * throws when missing, so a malformed push never clears state; scalars and answer lists read as their defaults,
     * unknown enum values as their placeholders
     */
    fun decode(json: Json, obj: JsonObject): HostMsg {
      val type = obj.string("type") ?: return Unknown("")
      fun <T> field(key: String, s: KSerializer<T>): T =
        json.decodeFromJsonElement(s, obj[key] ?: throw IllegalArgumentException("$type without $key"))
      fun <T> optional(key: String, s: KSerializer<T>): T? = obj[key]?.takeIf { it !is kotlinx.serialization.json.JsonNull }?.let { json.decodeFromJsonElement(s, it) }
      fun <T> list(key: String, s: KSerializer<T>): List<T> = optional(key, ListSerializer(s)).orEmpty()
      fun error(): ProjectError? = obj.string("error")?.let(ProjectError::of)
      val seq = obj.int("seq") ?: 0
      return when (type) {
        "init" -> Init(field("state", InitState.serializer()))
        "agents" -> Agents(field("agents", ListSerializer(AgentInfo.serializer())))
        "sessions" -> Sessions(field("sessions", ListSerializer(SessionSummary.serializer())))
        "categories" -> Categories(field("categories", SessionCategories.serializer()))
        "noSession" -> NoSession
        "session" -> Session(field("session", SessionView.serializer()))
        "sessionPatch" -> Patch(field("patch", SessionPatch.serializer()))
        "sessionsPatch" -> ListPatch(field("patch", SessionsPatch.serializer()))
        "accounts" -> Accounts(field("accounts", ListSerializer(AccountInfo.serializer())))
        "accountActions" -> AccountActions(field("actions", ListSerializer(AccountAction.serializer())))
        "hidden" -> Hidden(field("hidden", hiddenSerializer))
        "settings" -> Settings(field("settings", SettingsView.serializer()), obj.string("locale") ?: "en")
        "controls" -> Controls(
          obj.string("agent") ?: "",
          field("controls", ListSerializer(ConfigControl.serializer())),
          list("commands", SlashCommand.serializer()),
          obj.string("cwd"),
          optional("shapes", shapesSerializer),
        )
        "sessionHits" -> SessionHits(seq, list("hits", SessionHit.serializer()))
        "subagent" -> Subagent(
          obj.string("sessionId") ?: "", obj.string("subagentId") ?: "", obj.int("rev") ?: 0, obj.bool("running") ?: false,
          list("turns", Turn.serializer()),
        )
        "files" -> Files(seq, list("files", FileHit.serializer()))
        "diffSource" -> DiffSource(seq, optional("source", FullDiffSource.serializer()))
        "imageBlob" -> ImageBlob(seq, obj.string("blob"))
        "stagedFile" -> StagedFile(seq, obj.string("blob"), obj.double("size")?.toLong(), error())
        "sessionMarkdown" -> SessionMarkdown(seq, obj.string("markdown"), obj.string("error"))
        "projectDir" -> ProjectDir(seq, list("entries", ProjectEntry.serializer()), error())
        "projectStatus" -> ProjectStatus(seq, obj.string("root"), optional("marks", MapSerializer(String.serializer(), GitMark.serializer())).orEmpty())
        "projectFile" -> ProjectFile(seq, obj.string("text"), optional("hunks", ListSerializer(ProjectHunk.serializer())), error())
        "sessionChanges" -> SessionChanges(seq, list("files", ProjectChange.serializer()))
        "terminalOutput" -> TerminalOutput(obj.string("id") ?: "", obj.string("data") ?: "")
        "terminalState" -> TerminalState(
          obj.string("id") ?: "", obj.string("title"), obj.int("cols"), obj.int("rows"), obj.string("replay"),
          optional("exit", TerminalExit.serializer()), obj.string("error"),
        )
        "editTurnResult" -> EditTurnResult(obj.string("requestId") ?: "", obj.string("error"))
        "deleteSessionResult" -> DeleteSessionResult(obj.string("requestId") ?: "", obj.bool("undo") ?: false, obj.string("error"))
        "appearance" -> Appearance(field("appearance", MapSerializer(String.serializer(), String.serializer())))
        "connections" -> Connections(field("connections", ListSerializer(ConnectionView.serializer())))
        "localDir" -> LocalDir(seq, obj.string("path") ?: "", obj.string("parent"), list("entries", DirEntry.serializer()), obj.string("error"))
        "providers" -> Providers(ProvidersView.decode(obj["view"]), obj.string("error"))
        "providerProbed" -> ProviderProbed(obj.string("id") ?: "", ProbeOutcome.decode(obj["outcome"]))
        else -> Unknown(type)
      }
    }

    private val hiddenSerializer = MapSerializer(String.serializer(), MapSerializer(String.serializer(), ListSerializer(String.serializer())))
    private val shapesSerializer = MapSerializer(String.serializer(), ListSerializer(ConfigControl.serializer()))
  }
}

/**
 * An attachment as the composer sends it (protocol.ts Draft); the phone sends images, pasted text and files it picked,
 * a file's bytes in `data` (the engine stages them and links the staged path). Not mirrored yet: `selection` and `quote`
 * (an exhaustive `when` in ui/Composer.kt would need a branch for each)
 */
sealed interface Draft {
  data class Image(val mimeType: String, val data: String, val name: String? = null) : Draft
  /** context: where a shared text came from */
  data class Text(val name: String, val text: String, val context: TextContext? = null) : Draft
  data class File(val uri: String, val name: String, val data: String? = null) : Draft

  fun toJson(): JsonObject = when (this) {
    is Image -> buildJsonObject { put("kind", "image"); put("mimeType", mimeType); put("data", data); name?.let { put("name", it) } }
    is Text -> buildJsonObject {
      put("kind", "text"); put("name", name); put("text", text)
      context?.let { put("context", WireJson.encodeToJsonElement(TextContext.serializer(), it)) }
    }
    is File -> buildJsonObject { put("kind", "file"); put("uri", uri); put("name", name); data?.let { put("data", it) } }
  }

  companion object {
    /** The `kind` literals this build sends */
    val kinds = listOf("image", "text", "file")
  }
}

/**
 * Page → engine: the actions the phone sends, wrapped in a webviewMessage envelope. A session action carries the id of
 * the session the view shows; the engine refuses one for another session. clientId (send, stop, permission, answer,
 * steerQueued): the engine running the session runs each id once, so a resend after a lost link is safe
 */
sealed interface WebviewMsg {
  /** The page is mounted: the engine answers with `init`. patches: the page applies `sessionPatch`; listPatches: `sessionsPatch` */
  data class Ready(val patches: Boolean = true, val listPatches: Boolean? = null) : WebviewMsg
  /** A sessionPatch did not fit the view the page holds: the engine sends the whole view again */
  data object Resync : WebviewMsg
  /** A sessionsPatch did not fit the list the page holds: the engine sends the whole list again */
  data object ResyncSessions : WebviewMsg
  /** steer: join the running turn instead of queueing behind it (SessionView.canSteer) */
  data class Send(val sessionId: String?, val text: String, val attachments: List<Draft>? = null, val steer: Boolean? = null, override val clientId: String? = null) : WebviewMsg
  data class Stop(val sessionId: String?, override val clientId: String? = null) : WebviewMsg
  data class Permission(val sessionId: String, val blockId: String, val optionId: String, override val clientId: String? = null) : WebviewMsg
  /** The question card closed: `answers` holds the answered questions only; skip tells the agent to go on without them */
  data class Answer(
    val sessionId: String,
    val blockId: String,
    val answers: Map<String, JsonElement>,
    val skip: Boolean? = null,
    override val clientId: String? = null,
    /** Images sent with the answers, by question id (QuestionBlock.images) */
    val images: Map<String, List<Draft>>? = null,
  ) : WebviewMsg
  /** Build a reviewed plan: optionId picks the card's option (build / revise …), model the model it is built with */
  data class BuildPlan(val sessionId: String, val planId: String, val optionId: String? = null, val model: ModelPick? = null) : WebviewMsg
  data class SetMode(val sessionId: String?, val id: String) : WebviewMsg
  data class SetConfig(val sessionId: String?, val configId: String, val value: String) : WebviewMsg
  data class SelectSession(val id: String) : WebviewMsg
  /** The list is on top or the app went to the background: the chat no longer counts as watched */
  data class DeselectSession(val id: String) : WebviewMsg
  /** category: file it under that category from the start; `chat`: a session in a fresh folder of its own, `cwd` ignored */
  data class NewSession(val agent: AgentId? = null, val cwd: String? = null, val chat: Boolean? = null, val category: String? = null) : WebviewMsg
  data class RenameSession(val id: String, val title: String) : WebviewMsg
  /** requestId: answered with deleteSessionResult echoing it */
  data class DeleteSession(val id: String, val requestId: String? = null) : WebviewMsg
  /** Undo a deletion (deleteSessionResult said `undo`) */
  data class RestoreSession(val id: String) : WebviewMsg
  data class PinSession(val id: String, val pinned: Boolean) : WebviewMsg
  data class ArchiveSession(val id: String, val archived: Boolean) : WebviewMsg
  /** "Mark as unread / read" for one session */
  data class SetSessionUnread(val id: String, val unread: Boolean) : WebviewMsg
  /** Every unread mark goes, for every engine and window on the machine */
  data object MarkAllRead : WebviewMsg
  /** Full-text search over saved conversations, answered with `sessionHits` echoing seq */
  data class SearchSessions(val query: String, val seq: Int) : WebviewMsg
  /** Rebind the session to this account (also the agent's default account from then on) */
  data class SelectAccount(val sessionId: String?, val id: String) : WebviewMsg
  /** An account list came into view: refresh that agent's quotas */
  data class RefreshQuota(val agent: AgentId, val sessionId: String? = null) : WebviewMsg
  data class Compact(val sessionId: String?) : WebviewMsg
  /**
   * SessionView.canTakeOver: end the agent another engine left holding the session, or ask the engine that has this
   * read-only copy open to hand it over
   */
  data class TakeOverSession(val sessionId: String?) : WebviewMsg
  data class Dequeue(val sessionId: String, val id: String) : WebviewMsg
  data class SendQueued(val sessionId: String, val id: String) : WebviewMsg
  /** Steer a queued prompt into the running turn */
  data class SteerQueued(val sessionId: String, val id: String, override val clientId: String? = null) : WebviewMsg
  /** Replace a queued prompt in place: kept attachments by index, new drafts alongside */
  data class EditQueued(val sessionId: String, val id: String, val text: String, val retainedAttachments: List<Int>, val attachments: List<Draft>) : WebviewMsg
  /** An edit of the project list; `file`: with a create, file that session under the new category in the same step */
  data class CategoryOp(val op: cc.hotic.acpira.model.CategoryOp, val file: String? = null) : WebviewMsg
  /**
   * Re-home a session into another project folder (absent: the view's workspace folder), the agent working there from the
   * next open on; `category`, one of that project's, files it there in the same step
   */
  data class MoveSession(val id: String, val cwd: String? = null, val category: String? = null) : WebviewMsg
  /** Export: the session as Markdown (or `format`), answered with `sessionMarkdown` echoing seq */
  data class SessionMarkdown(val id: String, val seq: Int, val format: ExportFormat? = null) : WebviewMsg
  data class AddAccount(val agent: AgentId, val via: AddAccountVia, val sessionId: String? = null) : WebviewMsg
  data class RemoveAccount(val id: String, val sessionId: String? = null) : WebviewMsg
  /** AgentInfo.credentialsLocked: unlock the credential store in a terminal, then reconnect the sessions stuck on sign-in */
  data class UnlockCredentials(val agent: AgentId, val sessionId: String? = null) : WebviewMsg
  /** Sign in with one of SessionView.authMethods */
  data class Login(val sessionId: String?, val methodId: String? = null) : WebviewMsg
  /** `cwd`: the folder a new session would start in; on an SSH host the agent may run in the host's engine, with its models */
  data class Controls(val agent: AgentId, val fresh: Boolean? = null, val cwd: String? = null) : WebviewMsg
  data class Retry(val sessionId: String?) : WebviewMsg
  /** The failed last turn and its prompt go, the prompt runs again (`retry` restarts the session instead) */
  data class RetryTurn(val sessionId: String?) : WebviewMsg
  /** Rewrite a sent prompt, answered with `editTurnResult` echoing requestId */
  data class EditTurn(val requestId: String, val edit: EditTurnRequest) : WebviewMsg
  /** A new session holding this one's turns through `turnIndex`; agent: continue it on another agent */
  data class ForkSession(val sessionId: String, val turnIndex: Int, val agent: AgentId? = null) : WebviewMsg
  /** Drop the agent process and resume the same native session */
  data class Reconnect(val sessionId: String?) : WebviewMsg
  /** Follow / stop following one subagent's transcript (the `subagent` push); per viewer, one at a time */
  data class ObserveSubagent(val sessionId: String, val subagentId: String) : WebviewMsg
  data class UnobserveSubagent(val sessionId: String, val subagentId: String) : WebviewMsg
  /** Ask the agent to cancel one child */
  data class CancelSubagent(val sessionId: String, val subagentId: String) : WebviewMsg
  /** Ask the adapter to stop one AIR async task */
  data class StopAsyncTask(val sessionId: String, val taskId: String) : WebviewMsg
  /** A goal control (SessionView.goalActions); turn-starting ones go out as a `/goal …` prompt */
  data class Goal(val sessionId: String, val action: GoalAction, val objective: String? = null) : WebviewMsg
  /** @ mention: fuzzy search over the workspace's files, answered with `files` echoing seq */
  data class SearchFiles(val query: String, val seq: Int) : WebviewMsg
  /** One engine setting by its SettingsView key; the engine answers with `settings` to every window */
  data class SetSetting(val key: String, val value: JsonElement) : WebviewMsg
  /**
   * The session's project, each answered by the push of the same name echoing seq: a folder (`dir` relative, "" for the
   * project itself), its git marks, a file with its hunks against `base`, the files the session's edits left changed
   */
  data class ProjectDir(val sessionId: String, val dir: String, val seq: Int) : WebviewMsg
  data class ProjectStatus(val sessionId: String, val seq: Int) : WebviewMsg
  data class ProjectFile(val sessionId: String, val path: String, val seq: Int, val base: ProjectBase? = null) : WebviewMsg
  data class SessionChanges(val sessionId: String, val seq: Int) : WebviewMsg
  /**
   * The whole files behind the `nth` diff of a tool call (counted in `contents`, or `content` alone), answered with
   * `diffSource` echoing seq; subagentId: the subagent transcript the diff was shown in
   */
  data class DiffSource(val sessionId: String, val toolCallId: String, val nth: Int, val seq: Int, val subagentId: String? = null) : WebviewMsg
  /**
   * Stage a file path into the session's blob store, answered with `imageBlob` echoing seq. `shown`: a file the session's
   * transcript names, the only form a paired device may send (the engine checks it)
   */
  data class ImageBlob(val sessionId: String, val path: String, val seq: Int, val shown: Boolean = false) : WebviewMsg
  /** Stage any file of the session's project (or an absolute path) to download and open; answered with `stagedFile` */
  data class StageFile(val sessionId: String, val path: String, val seq: Int) : WebviewMsg
  /** Open a plan document; the engine passes it on as the openPlanDocument platform request */
  data class OpenPlan(val sessionId: String, val planId: String) : WebviewMsg
  /**
   * The engine's terminals: open terminal `id` (a login shell in `cwd`) or view it again; keys and pastes (`binary`: each
   * character one byte); the laid-out size; how much output (UTF-16 units) the page has rendered; stop viewing it; end
   * its shell
   */
  data class TerminalOpen(val id: String, val cwd: String? = null, val cols: Int? = null, val rows: Int? = null) : WebviewMsg
  data class TerminalInput(val id: String, val data: String, val binary: Boolean? = null) : WebviewMsg
  data class TerminalResize(val id: String, val cols: Int, val rows: Int) : WebviewMsg
  data class TerminalAck(val id: String, val chars: Int) : WebviewMsg
  data class TerminalClose(val id: String) : WebviewMsg
  data class TerminalKill(val id: String) : WebviewMsg
  /** The SSH hosts' state, answered with a `connections` push and pushed again on every change */
  data object Connections : WebviewMsg
  /** A folder of the engine's computer (`path` absent: the user's home, a leading `~` expands), answered with localDir */
  data class LocalDir(val path: String?, val seq: Int) : WebviewMsg
  /** The project list's addProject op (categoryOp): a folder registered at the end of the user's order */
  data class AddProject(val cwd: String) : WebviewMsg
  /** The built-in agent's model sources, answered with providers */
  data object Providers : WebviewMsg
  data class ProviderAction(val action: cc.hotic.acpira.model.ProviderAction) : WebviewMsg
  /** Answered by providerProbed with the same id */
  data class ProviderProbe(val id: String, val probe: cc.hotic.acpira.model.ProviderProbe) : WebviewMsg

  /** The wire `type` */
  val type: String
    get() = when (this) {
      is Ready -> "ready"
      Resync -> "resync"
      ResyncSessions -> "resyncSessions"
      is Send -> "send"
      is Stop -> "stop"
      is Permission -> "permission"
      is Answer -> "answer"
      is BuildPlan -> "buildPlan"
      is SetMode -> "setMode"
      is SetConfig -> "setConfig"
      is SelectSession -> "selectSession"
      is DeselectSession -> "deselectSession"
      is NewSession -> "newSession"
      is RenameSession -> "renameSession"
      is DeleteSession -> "deleteSession"
      is RestoreSession -> "restoreSession"
      is PinSession -> "pinSession"
      is ArchiveSession -> "archiveSession"
      is SetSessionUnread -> "setSessionUnread"
      MarkAllRead -> "markAllRead"
      is SearchSessions -> "searchSessions"
      is SelectAccount -> "selectAccount"
      is RefreshQuota -> "refreshQuota"
      is Compact -> "compact"
      is TakeOverSession -> "takeOverSession"
      is Dequeue -> "dequeue"
      is SendQueued -> "sendQueued"
      is SteerQueued -> "steerQueued"
      is EditQueued -> "editQueued"
      is CategoryOp -> "categoryOp"
      is MoveSession -> "moveSession"
      is SessionMarkdown -> "sessionMarkdown"
      is AddAccount -> "addAccount"
      is RemoveAccount -> "removeAccount"
      is UnlockCredentials -> "unlockCredentials"
      is Login -> "login"
      is Controls -> "controls"
      is Retry -> "retry"
      is RetryTurn -> "retryTurn"
      is EditTurn -> "editTurn"
      is ForkSession -> "forkSession"
      is Reconnect -> "reconnect"
      is ObserveSubagent -> "observeSubagent"
      is UnobserveSubagent -> "unobserveSubagent"
      is CancelSubagent -> "cancelSubagent"
      is StopAsyncTask -> "stopAsyncTask"
      is Goal -> "goal"
      is SearchFiles -> "searchFiles"
      is SetSetting -> "setSetting"
      is ProjectDir -> "projectDir"
      is ProjectStatus -> "projectStatus"
      is ProjectFile -> "projectFile"
      is SessionChanges -> "sessionChanges"
      is DiffSource -> "diffSource"
      is ImageBlob -> "imageBlob"
      is StageFile -> "stageFile"
      is OpenPlan -> "openPlan"
      is TerminalOpen -> "terminalOpen"
      is TerminalInput -> "terminalInput"
      is TerminalResize -> "terminalResize"
      is TerminalAck -> "terminalAck"
      is TerminalClose -> "terminalClose"
      is TerminalKill -> "terminalKill"
      Connections -> "connections"
      is LocalDir -> "localDir"
      is AddProject -> "categoryOp"
      Providers -> "providers"
      is ProviderAction -> "providerAction"
      is ProviderProbe -> "providerProbe"
    }

  /** The id the engine dedupes this action by; null for actions that carry none */
  val clientId: String? get() = null

  /** The action with a fresh clientId where it can carry one and has none yet; a resend keeps the id it was given */
  fun stamped(id: () -> String): WebviewMsg = when (this) {
    is Send -> if (clientId == null) copy(clientId = id()) else this
    is Stop -> if (clientId == null) copy(clientId = id()) else this
    is Permission -> if (clientId == null) copy(clientId = id()) else this
    is Answer -> if (clientId == null) copy(clientId = id()) else this
    is SteerQueued -> if (clientId == null) copy(clientId = id()) else this
    else -> this
  }

  /**
   * Worth sending again on the next wire: what the user did to a session. Reads, seq requests, terminals and view
   * switches belong to the wire that asked (the screens ask again)
   */
  val replayable: Boolean
    get() = this is Send || this is Stop || this is Permission || this is Answer || this is SteerQueued || this is SendQueued ||
      this is SetMode || this is SetConfig

  fun toJson(): JsonObject = buildJsonObject {
    put("type", type)
    when (val m = this@WebviewMsg) {
      is Ready -> { put("patches", m.patches); m.listPatches?.let { put("listPatches", it) } }
      Resync, ResyncSessions, MarkAllRead, Connections, Providers -> {}
      is Send -> {
        opt("sessionId", m.sessionId); put("text", m.text)
        m.attachments?.let { put("attachments", drafts(it)) }
        m.steer?.let { put("steer", it) }; opt("clientId", m.clientId)
      }
      is Stop -> { opt("sessionId", m.sessionId); opt("clientId", m.clientId) }
      is Permission -> { put("sessionId", m.sessionId); put("blockId", m.blockId); put("optionId", m.optionId); opt("clientId", m.clientId) }
      is Answer -> {
        put("sessionId", m.sessionId); put("blockId", m.blockId); put("answers", JsonObject(m.answers))
        m.images?.let { images -> put("images", buildJsonObject { images.forEach { (k, v) -> put(k, drafts(v)) } }) }
        m.skip?.let { put("skip", it) }; opt("clientId", m.clientId)
      }
      is BuildPlan -> {
        put("sessionId", m.sessionId); put("planId", m.planId); opt("optionId", m.optionId)
        m.model?.let { put("model", WireJson.encodeToJsonElement(ModelPick.serializer(), it)) }
      }
      is SetMode -> { opt("sessionId", m.sessionId); put("id", m.id) }
      is SetConfig -> { opt("sessionId", m.sessionId); put("configId", m.configId); put("value", m.value) }
      is SelectSession -> put("id", m.id)
      is DeselectSession -> put("id", m.id)
      is NewSession -> { opt("agent", m.agent); opt("category", m.category); opt("cwd", m.cwd); m.chat?.let { put("chat", it) } }
      is RenameSession -> { put("id", m.id); put("title", m.title) }
      is DeleteSession -> { put("id", m.id); opt("requestId", m.requestId) }
      is RestoreSession -> put("id", m.id)
      is PinSession -> { put("id", m.id); put("pinned", m.pinned) }
      is ArchiveSession -> { put("id", m.id); put("archived", m.archived) }
      is SetSessionUnread -> { put("id", m.id); put("unread", m.unread) }
      is SearchSessions -> { put("query", m.query); put("seq", m.seq) }
      is SelectAccount -> { opt("sessionId", m.sessionId); put("id", m.id) }
      is RefreshQuota -> { put("agent", m.agent); opt("sessionId", m.sessionId) }
      is Compact -> opt("sessionId", m.sessionId)
      is TakeOverSession -> opt("sessionId", m.sessionId)
      is Dequeue -> { put("sessionId", m.sessionId); put("id", m.id) }
      is SendQueued -> { put("sessionId", m.sessionId); put("id", m.id) }
      is SteerQueued -> { put("sessionId", m.sessionId); put("id", m.id); opt("clientId", m.clientId) }
      is EditQueued -> {
        put("sessionId", m.sessionId); put("id", m.id); put("text", m.text)
        put("retainedAttachments", buildJsonArray { m.retainedAttachments.forEach { add(it) } })
        put("attachments", drafts(m.attachments))
      }
      // categoryOp carries the op's fields flat (protocol.ts `{ type: 'categoryOp'; file? } & CategoryOp`)
      is CategoryOp -> { m.op.fields(this); opt("file", m.file) }
      is MoveSession -> { put("id", m.id); opt("cwd", m.cwd); opt("category", m.category) }
      is SessionMarkdown -> { put("id", m.id); put("seq", m.seq); m.format?.let { put("format", it.name) } }
      is AddAccount -> { put("agent", m.agent); put("via", m.via.name); opt("sessionId", m.sessionId) }
      is RemoveAccount -> { put("id", m.id); opt("sessionId", m.sessionId) }
      is UnlockCredentials -> { put("agent", m.agent); opt("sessionId", m.sessionId) }
      is Login -> { opt("sessionId", m.sessionId); opt("methodId", m.methodId) }
      is Controls -> { put("agent", m.agent); m.fresh?.let { put("fresh", it) }; opt("cwd", m.cwd) }
      is Retry -> opt("sessionId", m.sessionId)
      is RetryTurn -> opt("sessionId", m.sessionId)
      is EditTurn -> { put("requestId", m.requestId); put("edit", m.edit.toJson()) }
      is ForkSession -> { put("sessionId", m.sessionId); put("turnIndex", m.turnIndex); opt("agent", m.agent) }
      is Reconnect -> opt("sessionId", m.sessionId)
      is ObserveSubagent -> { put("sessionId", m.sessionId); put("subagentId", m.subagentId) }
      is UnobserveSubagent -> { put("sessionId", m.sessionId); put("subagentId", m.subagentId) }
      is CancelSubagent -> { put("sessionId", m.sessionId); put("subagentId", m.subagentId) }
      is StopAsyncTask -> { put("sessionId", m.sessionId); put("taskId", m.taskId) }
      is Goal -> { put("sessionId", m.sessionId); put("action", m.action.name); opt("objective", m.objective) }
      is SearchFiles -> { put("query", m.query); put("seq", m.seq) }
      is SetSetting -> { put("key", m.key); put("value", m.value) }
      is ProjectDir -> { put("sessionId", m.sessionId); put("dir", m.dir); put("seq", m.seq) }
      is ProjectStatus -> { put("sessionId", m.sessionId); put("seq", m.seq) }
      is ProjectFile -> { put("sessionId", m.sessionId); put("path", m.path); m.base?.let { put("base", it.name) }; put("seq", m.seq) }
      is SessionChanges -> { put("sessionId", m.sessionId); put("seq", m.seq) }
      is DiffSource -> {
        put("sessionId", m.sessionId); put("toolCallId", m.toolCallId); put("nth", m.nth); opt("subagentId", m.subagentId); put("seq", m.seq)
      }
      is ImageBlob -> { put("sessionId", m.sessionId); put("path", m.path); put("seq", m.seq); if (m.shown) put("shown", true) }
      is StageFile -> { put("sessionId", m.sessionId); put("path", m.path); put("seq", m.seq) }
      is OpenPlan -> { put("sessionId", m.sessionId); put("planId", m.planId) }
      is TerminalOpen -> { put("id", m.id); opt("cwd", m.cwd); m.cols?.let { put("cols", it) }; m.rows?.let { put("rows", it) } }
      is TerminalInput -> { put("id", m.id); put("data", m.data); m.binary?.let { put("binary", it) } }
      is TerminalResize -> { put("id", m.id); put("cols", m.cols); put("rows", m.rows) }
      is TerminalAck -> { put("id", m.id); put("chars", m.chars) }
      is TerminalClose -> put("id", m.id)
      is TerminalKill -> put("id", m.id)
      is LocalDir -> { opt("path", m.path); put("seq", m.seq) }
      is AddProject -> cc.hotic.acpira.model.CategoryOp.AddProject(m.cwd).fields(this)
      is ProviderAction -> put("action", m.action.toJson())
      is ProviderProbe -> { put("id", m.id); put("probe", m.probe.toJson()) }
    }
  }

  companion object {
    /** The `type` literals this build sends */
    val types = listOf(
      "ready", "resync", "resyncSessions", "send", "stop", "permission", "answer", "buildPlan", "setMode", "setConfig",
      "selectSession", "deselectSession", "newSession", "renameSession", "deleteSession", "restoreSession", "pinSession",
      "archiveSession", "setSessionUnread", "markAllRead", "searchSessions", "forkSession", "selectAccount", "refreshQuota",
      "compact", "retry", "takeOverSession", "dequeue", "sendQueued", "steerQueued", "editQueued", "categoryOp",
      "moveSession", "sessionMarkdown", "addAccount", "removeAccount", "unlockCredentials", "login", "reconnect",
      "retryTurn", "editTurn", "observeSubagent", "unobserveSubagent", "cancelSubagent", "stopAsyncTask", "goal",
      "searchFiles", "setSetting", "controls", "projectDir", "projectStatus", "projectFile", "sessionChanges", "diffSource",
      "imageBlob", "stageFile", "openPlan", "terminalOpen", "terminalInput", "terminalResize", "terminalAck",
      "terminalClose", "terminalKill", "connections", "localDir", "providers", "providerAction", "providerProbe",
    )
  }
}

private fun drafts(list: List<Draft>): JsonArray = buildJsonArray { list.forEach { add(it.toJson()) } }

private fun JsonObjectBuilder.opt(key: String, value: String?) {
  if (value != null) put(key, JsonPrimitive(value))
}
