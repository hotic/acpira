@file:Suppress("EnumEntryName")

package cc.hotic.acpira.model

import kotlinx.serialization.KSerializer
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject

// Mirror of packages/shared/transcript.ts: the session view the engine pushes and the page renders. Field names are the
// TypeScript ones; ContractTest pins the tagged unions' literals to the sources.

typealias AgentId = String

// Tolerant string enums: an unknown literal reads as the listed fallback

@Serializable(with = SessionStatus.S::class)
enum class SessionStatus { starting, ready, auth_required, readonly, error, closed, unknown;
  object S : TolerantEnumSerializer<SessionStatus>("SessionStatus", entries, unknown)
}

@Serializable(with = ToolKind.S::class)
enum class ToolKind { read, edit, delete, move, search, execute, think, fetch, switch_mode, other;
  object S : TolerantEnumSerializer<ToolKind>("ToolKind", entries, other)
}

@Serializable(with = ToolStatus.S::class)
enum class ToolStatus { pending, in_progress, completed, failed, cancelled;
  object S : TolerantEnumSerializer<ToolStatus>("ToolStatus", entries, completed)
  /** Still running: pending or in progress */
  val live: Boolean get() = this == pending || this == in_progress
}

@Serializable(with = PlanStatus.S::class)
enum class PlanStatus { pending, in_progress, completed;
  object S : TolerantEnumSerializer<PlanStatus>("PlanStatus", entries, pending)
}

@Serializable(with = PermissionKind.S::class)
enum class PermissionKind { allow_once, allow_always, reject_once, reject_always, unknown;
  object S : TolerantEnumSerializer<PermissionKind>("PermissionKind", entries, unknown)
}

@Serializable(with = CompactionStatus.S::class)
enum class CompactionStatus { in_progress, completed, failed, cancelled;
  object S : TolerantEnumSerializer<CompactionStatus>("CompactionStatus", entries, completed)
}

@Serializable(with = QuestionKind.S::class)
enum class QuestionKind { single, multiple, text;
  object S : TolerantEnumSerializer<QuestionKind>("QuestionKind", entries, text)
}

@Serializable(with = QuestionOutcome.S::class)
enum class QuestionOutcome { answered, skipped, cancelled;
  object S : TolerantEnumSerializer<QuestionOutcome>("QuestionOutcome", entries, cancelled)
}

@Serializable(with = TurnStop.S::class)
enum class TurnStop { end_turn, max_tokens, max_turn_requests, refusal, cancelled, error, unknown;
  object S : TolerantEnumSerializer<TurnStop>("TurnStop", entries, unknown)
}

@Serializable(with = FailureAction.S::class)
enum class FailureAction { retry, login, new_session, unknown;
  object S : TolerantEnumSerializer<FailureAction>("FailureAction", entries, unknown)
}

/** A list row's state mark; background: the turn ended but a background task it started still runs */
@Serializable(with = SessionState.S::class)
enum class SessionState { working, waiting, unread, error, background, unknown;
  object S : TolerantEnumSerializer<SessionState>("SessionState", entries, unknown)
}

@Serializable(with = GoalStatus.S::class)
enum class GoalStatus { active, paused, blocked, limited, complete, unknown;
  object S : TolerantEnumSerializer<GoalStatus>("GoalStatus", entries, unknown)
}

/** A goal control the agent advertises at initialize; `set` is also what `/goal <objective>` does */
@Serializable(with = GoalAction.S::class)
enum class GoalAction { set, pause, resume, clear, unknown;
  object S : TolerantEnumSerializer<GoalAction>("GoalAction", entries, unknown)
}

@Serializable(with = GoalEvent.S::class)
enum class GoalEvent { set, paused, resumed, blocked, limited, complete, cleared, unknown;
  object S : TolerantEnumSerializer<GoalEvent>("GoalEvent", entries, unknown)
}

@Serializable(with = SubagentState.S::class)
enum class SubagentState { running, completed, failed, cancelled, disconnected, unknown;
  object S : TolerantEnumSerializer<SubagentState>("SubagentState", entries, unknown)
}

/**
 * What Acpira observes of a child: `session` its own updates under its own session id, `nested` its tool calls on the
 * parent session with a parent link, `receipt` only the delegation call and its result
 */
@Serializable(with = SubagentVisibility.S::class)
enum class SubagentVisibility { session, nested, receipt, unknown;
  object S : TolerantEnumSerializer<SubagentVisibility>("SubagentVisibility", entries, unknown)
}

/** A summoned child's access: `bypass` works without approvals; `inherit` (a persona setting only) takes the caller's */
@Serializable(with = RelayMode.S::class)
enum class RelayMode { consult, work, bypass, inherit, unknown;
  object S : TolerantEnumSerializer<RelayMode>("RelayMode", entries, unknown)
}

/** Why an account shows no quota bars: the last read failed and the next one waits */
@Serializable(with = AccountQuotaIssue.S::class)
enum class AccountQuotaIssue { rate_limited, expired, unavailable, unknown;
  object S : TolerantEnumSerializer<AccountQuotaIssue>("AccountQuotaIssue", entries, unknown)
}

/** Agent → option id → the option values hidden from its pickers (settings.ts HiddenMap) */
typealias HiddenMap = Map<AgentId, Map<String, List<String>>>

/** A model option id → the dependent controls (effort, Fast …) that model came with (modelShapes.ts) */
typealias ModelShapes = Map<String, List<ConfigControl>>

// Agents and accounts

@Serializable
data class AgentInstall(val command: String? = null, val docs: String? = null)

@Serializable
data class QuotaWindow(val id: String, val remaining: Double, val resetsAt: String? = null)

@Serializable
data class AccountQuota(val windows: List<QuotaWindow> = emptyList(), val onDemandBalanceUsd: Double? = null, val fetchedAt: String = "")

@Serializable
data class LocalAccountInfo(val label: String, val detail: String? = null, val status: String = "", val quota: AccountQuota? = null)

@Serializable
data class AgentInfo(
  val id: AgentId,
  val name: String,
  val accounts: Boolean? = null,
  val credentialsLocked: Boolean? = null,
  val localAccount: LocalAccountInfo? = null,
  val available: Boolean? = null,
  val disabled: Boolean? = null,
  val missing: List<String>? = null,
  val searched: List<String>? = null,
  val install: AgentInstall? = null,
  val update: String? = null,
  /** Who makes the CLI, for an agent the app knows no vendor of (an agents.json entry or a plugin's) */
  val vendor: String? = null,
  /** The agent's own mark as SVG path data on a 24 x 24 box, for an agent the app has no mark of its own for */
  val mark: List<String>? = null,
)

@Serializable
data class AccountInfo(
  val id: String,
  val agent: AgentId,
  val label: String,
  val detail: String? = null,
  val addedAt: String? = null,
  val lastUsedAt: String? = null,
  val quota: AccountQuota? = null,
  /** Absent while `quota` is set */
  val quotaIssue: AccountQuotaIssue? = null,
)

// Controls

@Serializable
data class SessionOption(
  val id: String,
  val name: String,
  val description: String? = null,
  val group: Group? = null,
  val kind: String? = null,
  /** Where the model comes from; kind `official` / `custom` splits the settings' model page */
  val source: Source? = null,
) {
  @Serializable
  data class Group(val id: String, val name: String)

  @Serializable
  data class Source(val id: String, val name: String, val kind: String? = null)
}

@Serializable
data class ConfigControl(
  val id: String,
  val name: String,
  val category: String? = null,
  val type: String? = null,
  val options: List<SessionOption> = emptyList(),
  val value: String? = null,
)

@Serializable
data class SessionControls(
  val modes: List<SessionOption> = emptyList(),
  val modeId: String? = null,
  val modeConfigId: String? = null,
  val options: List<ConfigControl> = emptyList(),
)

@Serializable
data class SlashCommand(val name: String, val description: String = "", val input: Input? = null) {
  @Serializable
  data class Input(val hint: String = "")
}

// Blocks

@Serializable
data class ImageRef(val blob: String? = null, val mimeType: String = "", val uri: String? = null)

@Serializable
data class DiffLine(val kind: Kind = Kind.ctx, val text: String = "", val oldLine: Int? = null, val newLine: Int? = null) {
  @Serializable(with = Kind.S::class)
  enum class Kind { hunk, add, del, ctx;
    object S : TolerantEnumSerializer<Kind>("DiffLineKind", entries, ctx)
  }
}

/** The whole files behind a diff, as the `diffSource` push answers with them */
@Serializable
data class FullDiffSource(val path: String, val oldText: String = "", val newText: String = "")

/**
 * What a diff was taken from; a page view carries the path only (`omitted`), and the page asks for the texts when it
 * needs them (WebviewMsg `diffSource`)
 */
@Serializable
data class DiffSource(val path: String, val oldText: String? = null, val newText: String? = null, val omitted: Boolean? = null)

/** A tool call's output, tagged by `type` */
@Serializable(with = ToolContent.S::class)
sealed interface ToolContent {
  @Serializable data class Text(val text: String) : ToolContent
  @Serializable data class Diff(val lines: List<DiffLine> = emptyList(), val source: DiffSource? = null) : ToolContent
  @Serializable data class ListItems(val items: List<String> = emptyList()) : ToolContent
  @Serializable data class Image(val blob: String? = null, val mimeType: String = "", val uri: String? = null) : ToolContent
  data class Unknown(val type: String) : ToolContent

  object S : TaggedSerializer<ToolContent>("ToolContent", "type") {
    override fun decode(json: Json, tag: String, obj: JsonObject): ToolContent? = when (tag) {
      "text" -> json.decodeFromJsonElement(Text.serializer(), obj)
      "diff" -> json.decodeFromJsonElement(Diff.serializer(), obj)
      "list" -> json.decodeFromJsonElement(ListItems.serializer(), obj)
      "image" -> json.decodeFromJsonElement(Image.serializer(), obj)
      else -> null
    }
    override fun unknown(tag: String) = Unknown(tag)
    override fun encode(json: Json, value: ToolContent): JsonObject = when (value) {
      is Text -> tagged("type", "text", json.encodeToJsonElement(Text.serializer(), value))
      is Diff -> tagged("type", "diff", json.encodeToJsonElement(Diff.serializer(), value))
      is ListItems -> tagged("type", "list", json.encodeToJsonElement(ListItems.serializer(), value))
      is Image -> tagged("type", "image", json.encodeToJsonElement(Image.serializer(), value))
      is Unknown -> buildJsonObject { put("type", JsonPrimitive(value.type)) }
    }
  }
}

@Serializable
data class DiffStat(val add: Int = 0, val del: Int = 0)

/**
 * An AIR async task: background work the adapter reports apart from the prompt lifecycle. state: running | paused |
 * completed | failed | stopped, the last three final
 */
@Serializable
data class AsyncTaskInfo(
  val id: String,
  val state: String = "",
  val canStop: Boolean? = null,
  val name: String? = null,
  val description: String? = null,
  val summary: String? = null,
  val taskType: String? = null,
  val lastToolName: String? = null,
  val outputFilePath: String? = null,
  val usage: Usage? = null,
  /** A stop the engine asked for that the adapter's own state has not confirmed yet */
  val stopRequested: Boolean? = null,
) {
  @Serializable
  data class Usage(val totalTokens: Int? = null, val toolUses: Int? = null, val durationMs: Double? = null)
}

@Serializable
data class ToolLocation(val path: String, val line: Int? = null)

@Serializable
data class ReadRange(val path: String, val start: Int = 0, val end: Int? = null)

@Serializable
data class PlanEntry(val title: String = "", val status: PlanStatus = PlanStatus.pending, val priority: String? = null)

@Serializable
data class ThoughtBlock(val text: String = "", val startedAt: Double? = null, val durationSec: Double? = null, val streaming: Boolean? = null) : AgentBlock

@Serializable
data class PlanBlock(val entries: List<PlanEntry> = emptyList(), val changed: Boolean? = null) : AgentBlock

@Serializable
data class ToolCallBlock(
  val observation: String? = null,
  val id: String,
  val kind: ToolKind = ToolKind.other,
  val verb: String = "",
  val verbKey: String? = null,
  val mcpServer: String? = null,
  val target: String? = null,
  val targetMono: Boolean? = null,
  val locations: List<ToolLocation>? = null,
  val readRange: ReadRange? = null,
  val status: ToolStatus = ToolStatus.completed,
  val background: Boolean? = null,
  val asyncTask: AsyncTaskInfo? = null,
  val startedAt: Double? = null,
  val endedAt: Double? = null,
  val meta: String? = null,
  val diffStat: DiffStat? = null,
  val todoEntries: List<PlanEntry>? = null,
  val content: ToolContent? = null,
  val contents: List<ToolContent>? = null,
  val subagentId: String? = null,
) : AgentBlock {
  /** Every content item, `content` first (the contract carries either or both) */
  val allContents: List<ToolContent> get() = listOfNotNull(content) + contents.orEmpty()
}

@Serializable
data class TextBlock(
  val id: String? = null,
  val phase: String? = null,
  val markdown: String = "",
  val streaming: Boolean? = null,
  val images: List<ImageRef>? = null,
) : AgentBlock

@Serializable
data class PermissionOption(val id: String, val label: String = "", val kind: PermissionKind = PermissionKind.unknown, val detail: String? = null)

@Serializable
data class PermissionBlock(
  val id: String,
  val title: String = "",
  val command: String? = null,
  val description: String? = null,
  val planId: String? = null,
  val defaultToNo: Boolean? = null,
  val options: List<PermissionOption> = emptyList(),
) : AgentBlock

@Serializable
data class PlanDocumentBlock(
  val id: String,
  val title: String = "",
  val markdown: String = "",
  val path: String? = null,
  val toolCallId: String = "",
  val approvalToolCallId: String? = null,
  val status: String = "",
) : AgentBlock

@Serializable
data class CompactionBlock(val id: String, val status: CompactionStatus = CompactionStatus.completed, val error: String? = null) : AgentBlock

@Serializable
data class QuestionOption(val id: String, val label: String = "", val description: String? = null)

@Serializable
data class Question(
  val id: String,
  val title: String? = null,
  val text: String = "",
  val kind: QuestionKind = QuestionKind.text,
  val options: List<QuestionOption> = emptyList(),
  val other: Boolean? = null,
  val numeric: Boolean? = null,
  val required: Boolean? = null,
)

@Serializable
data class QuestionBlock(
  val id: String,
  val toolCallId: String? = null,
  val message: String? = null,
  val questions: List<Question> = emptyList(),
  val outcome: QuestionOutcome? = null,
  /** By question id: a string, or a list of option ids */
  val answers: Map<String, JsonElement>? = null,
  /** The agent takes images with the answers (the built-in agent) */
  val images: Boolean? = null,
  /** Images sent with the answers, by question id */
  val answerImages: Map<String, List<ImageRef>>? = null,
) : AgentBlock

@Serializable
data class ImageBlock(val id: String, val blob: String? = null, val mimeType: String = "", val uri: String? = null) : AgentBlock

@Serializable
data class NoticeBlock(
  val id: String,
  val revision: Int? = null,
  val category: String? = null,
  val severity: String = "warning",
  val info: Boolean? = null,
  val title: String = "",
  val details: String? = null,
  val actions: List<FailureAction>? = null,
  /** Set on the row that logs one run of provider retries */
  val retry: RetryLog? = null,
) : AgentBlock

/** One run of provider retries: every failed attempt with the provider's own error text */
@Serializable
data class RetryLog(
  /** A tool status: in_progress, completed, failed, cancelled */
  val status: String = "in_progress",
  val max: Int? = null,
  /** When the next attempt starts (epoch ms) */
  val nextAt: Double? = null,
  val reconnect: Boolean? = null,
  val attempts: List<RetryAttempt> = emptyList(),
)

@Serializable
data class RetryAttempt(
  /** Epoch ms */
  val at: Double = 0.0,
  val attempt: Int? = null,
  val text: String = "",
)

@Serializable
data class SteerBlock(val id: String, val text: String = "", val attachments: List<Attachment>? = null) : AgentBlock

@Serializable
data class GoalBlock(
  val event: GoalEvent = GoalEvent.unknown,
  val objective: String? = null,
  val iterations: Int? = null,
  val tokensUsed: Int? = null,
  val timeUsedSeconds: Double? = null,
) : AgentBlock

data class UnknownBlock(val type: String) : AgentBlock

/** One block of an agent turn, tagged by `type` */
@Serializable(with = AgentBlock.S::class)
sealed interface AgentBlock {
  object S : TaggedSerializer<AgentBlock>("AgentBlock", "type") {
    /** The `type` literals this build mirrors */
    val types = listOf(
      "thought", "plan", "tool_call", "text", "permission", "compaction", "plan_document", "question", "image", "notice",
      "steer", "goal",
    )

    override fun decode(json: Json, tag: String, obj: JsonObject): AgentBlock? = when (tag) {
      "thought" -> json.decodeFromJsonElement(ThoughtBlock.serializer(), obj)
      "plan" -> json.decodeFromJsonElement(PlanBlock.serializer(), obj)
      "tool_call" -> json.decodeFromJsonElement(ToolCallBlock.serializer(), obj)
      "text" -> json.decodeFromJsonElement(TextBlock.serializer(), obj)
      "permission" -> json.decodeFromJsonElement(PermissionBlock.serializer(), obj)
      "compaction" -> json.decodeFromJsonElement(CompactionBlock.serializer(), obj)
      "plan_document" -> json.decodeFromJsonElement(PlanDocumentBlock.serializer(), obj)
      "question" -> json.decodeFromJsonElement(QuestionBlock.serializer(), obj)
      "image" -> json.decodeFromJsonElement(ImageBlock.serializer(), obj)
      "notice" -> json.decodeFromJsonElement(NoticeBlock.serializer(), obj)
      "steer" -> json.decodeFromJsonElement(SteerBlock.serializer(), obj)
      "goal" -> json.decodeFromJsonElement(GoalBlock.serializer(), obj)
      else -> null
    }

    override fun unknown(tag: String) = UnknownBlock(tag)

    override fun encode(json: Json, value: AgentBlock): JsonObject = when (value) {
      is ThoughtBlock -> tagged("type", "thought", json.encodeToJsonElement(ThoughtBlock.serializer(), value))
      is PlanBlock -> tagged("type", "plan", json.encodeToJsonElement(PlanBlock.serializer(), value))
      is ToolCallBlock -> tagged("type", "tool_call", json.encodeToJsonElement(ToolCallBlock.serializer(), value))
      is TextBlock -> tagged("type", "text", json.encodeToJsonElement(TextBlock.serializer(), value))
      is PermissionBlock -> tagged("type", "permission", json.encodeToJsonElement(PermissionBlock.serializer(), value))
      is CompactionBlock -> tagged("type", "compaction", json.encodeToJsonElement(CompactionBlock.serializer(), value))
      is PlanDocumentBlock -> tagged("type", "plan_document", json.encodeToJsonElement(PlanDocumentBlock.serializer(), value))
      is QuestionBlock -> tagged("type", "question", json.encodeToJsonElement(QuestionBlock.serializer(), value))
      is ImageBlock -> tagged("type", "image", json.encodeToJsonElement(ImageBlock.serializer(), value))
      is NoticeBlock -> tagged("type", "notice", json.encodeToJsonElement(NoticeBlock.serializer(), value))
      is SteerBlock -> tagged("type", "steer", json.encodeToJsonElement(SteerBlock.serializer(), value))
      is GoalBlock -> tagged("type", "goal", json.encodeToJsonElement(GoalBlock.serializer(), value))
      is UnknownBlock -> buildJsonObject { put("type", JsonPrimitive(value.type)) }
    }
  }
}

/** The block's own id when the contract gives it one */
val AgentBlock.blockId: String?
  get() = when (this) {
    is ToolCallBlock -> id
    is TextBlock -> id
    is PermissionBlock -> id
    is CompactionBlock -> id
    is PlanDocumentBlock -> id
    is QuestionBlock -> id
    is ImageBlock -> id
    is NoticeBlock -> id
    is SteerBlock -> id
    else -> null
  }

// Turns

/** Where a shared text came from (another tool's selection, a picked web page) */
@Serializable
data class TextContext(val sourceId: String = "", val sourceName: String = "", val uri: String? = null, val comment: String? = null)

/** An attachment as a sent turn keeps it, tagged by `kind` */
@Serializable(with = Attachment.S::class)
sealed interface Attachment {
  @Serializable data class Image(val blob: String? = null, val mimeType: String = "", val name: String? = null) : Attachment
  /** context: where a shared text came from (absent on dropped text) */
  @Serializable data class Text(val blob: String? = null, val name: String = "", val context: TextContext? = null) : Attachment
  @Serializable data class File(val uri: String = "", val name: String = "") : Attachment
  @Serializable data class Selection(val uri: String = "", val name: String = "", val startLine: Int = 0, val endLine: Int = 0, val blob: String? = null) : Attachment
  @Serializable data class Quote(val text: String = "", val comment: String? = null) : Attachment
  data class Unknown(val kind: String) : Attachment

  object S : TaggedSerializer<Attachment>("Attachment", "kind") {
    override fun decode(json: Json, tag: String, obj: JsonObject): Attachment? = when (tag) {
      "image" -> json.decodeFromJsonElement(Image.serializer(), obj)
      "text" -> json.decodeFromJsonElement(Text.serializer(), obj)
      "file" -> json.decodeFromJsonElement(File.serializer(), obj)
      "selection" -> json.decodeFromJsonElement(Selection.serializer(), obj)
      "quote" -> json.decodeFromJsonElement(Quote.serializer(), obj)
      else -> null
    }
    override fun unknown(tag: String) = Unknown(tag)
    override fun encode(json: Json, value: Attachment): JsonObject = when (value) {
      is Image -> tagged("kind", "image", json.encodeToJsonElement(Image.serializer(), value))
      is Text -> tagged("kind", "text", json.encodeToJsonElement(Text.serializer(), value))
      is File -> tagged("kind", "file", json.encodeToJsonElement(File.serializer(), value))
      is Selection -> tagged("kind", "selection", json.encodeToJsonElement(Selection.serializer(), value))
      is Quote -> tagged("kind", "quote", json.encodeToJsonElement(Quote.serializer(), value))
      is Unknown -> buildJsonObject { put("kind", JsonPrimitive(value.kind)) }
    }
  }
}

/** The ACP selections a prompt was sent with */
@Serializable
data class TurnSettings(val modeId: String? = null, val config: Map<String, String> = emptyMap())

@Serializable
data class UserTurn(
  val id: String? = null,
  val text: String = "",
  val command: String? = null,
  val attachments: List<Attachment>? = null,
  val settings: TurnSettings? = null,
  val edited: Boolean? = null,
  val planId: String? = null,
  /** Sent by Acpira itself (compaction, a retry …); never a bubble */
  val auto: Boolean? = null,
  val autoReason: String? = null,
  val goalContinue: Boolean? = null,
) : Turn

@Serializable
data class TurnError(
  val message: String = "",
  val code: Int? = null,
  val kind: String? = null,
  val retryable: Boolean? = null,
  val failureId: String? = null,
  val actions: List<FailureAction>? = null,
)

@Serializable
data class TurnUsage(
  val input: Int? = null,
  val output: Int? = null,
  val cachedRead: Int? = null,
  val cachedWrite: Int? = null,
  val reasoning: Int? = null,
  val total: Int? = null,
  val modelCalls: Int? = null,
  val decodeMs: Int? = null,
  val model: String? = null,
  val requestId: String? = null,
  val context: Context? = null,
) {
  @Serializable
  data class Context(val used: Int = 0, val size: Int = 0)
}

@Serializable
data class TurnRetry(val attempt: Int? = null, val max: Int? = null, val detail: String = "")

@Serializable
data class TurnActivity(val kind: ToolKind = ToolKind.other, val label: String = "")

@Serializable
data class TurnCommand(val name: String = "", val mode: String? = null, val options: List<Option>? = null) {
  @Serializable
  data class Option(val name: String = "", val value: String = "")
}

@Serializable
data class AgentTurn(
  val observation: String? = null,
  val blocks: List<AgentBlock> = emptyList(),
  val usage: TurnUsage? = null,
  val startedAt: Double? = null,
  val respondedAt: Double? = null,
  val endedAt: Double? = null,
  val idleMs: Double? = null,
  val activity: TurnActivity? = null,
  val stop: TurnStop? = null,
  val error: TurnError? = null,
  val command: TurnCommand? = null,
  val retry: TurnRetry? = null,
) : Turn

/** One turn of a transcript, tagged by `role` */
@Serializable(with = Turn.S::class)
sealed interface Turn {
  object S : TaggedSerializer<Turn>("Turn", "role") {
    override fun decode(json: Json, tag: String, obj: JsonObject): Turn = when (tag) {
      "user" -> json.decodeFromJsonElement(UserTurn.serializer(), obj)
      // Anything but a user turn is the agent's, as the page reads it
      else -> json.decodeFromJsonElement(AgentTurn.serializer(), obj)
    }
    override fun unknown(tag: String): Turn = AgentTurn()
    override fun encode(json: Json, value: Turn): JsonObject = when (value) {
      is UserTurn -> tagged("role", "user", json.encodeToJsonElement(UserTurn.serializer(), value))
      is AgentTurn -> tagged("role", "agent", json.encodeToJsonElement(AgentTurn.serializer(), value))
    }
  }
}

// Sessions

@Serializable
data class SessionSummary(
  val id: String,
  val title: String = "",
  val agent: AgentId = "",
  val accountId: String? = null,
  val acpSessionId: String? = null,
  val cwd: String = "",
  val updatedAt: String = "",
  val createdAt: String? = null,
  val pinned: Boolean? = null,
  val archived: Boolean? = null,
  val category: String? = null,
  val state: SessionState? = null,
  /** Only beside `background`: a turn ended there that nobody has looked at yet */
  val unread: Boolean? = null,
  val model: Model? = null,
  /** Only beside `background`: when a pending scheduled wake-up resumes the session (epoch ms) */
  val wakeAt: Double? = null,
) {
  /** The model a session last ran: the wire id is its identity, the name its label */
  @Serializable
  data class Model(val id: String, val name: String = "")
}

@Serializable
data class Usage(val used: Int = 0, val size: Int = 0, val cost: Double? = null, val compactedFrom: Int? = null)

@Serializable
data class QueuedPrompt(val id: String, val text: String = "", val attachments: List<Attachment>? = null, val sending: Boolean? = null)

/**
 * A way to sign in (`login` methodId). `terminal`: the engine runs the agent binary with these args / env in a terminal
 * on its machine instead of calling `authenticate`
 */
@Serializable
data class AuthMethodInfo(val id: String, val name: String = "", val description: String? = null, val terminal: Terminal? = null) {
  @Serializable
  data class Terminal(val args: List<String> = emptyList(), val env: Map<String, String>? = null)
}

@Serializable
data class SessionGoal(
  val objective: String = "",
  val status: GoalStatus = GoalStatus.unknown,
  val iterations: Int? = null,
  val tokenBudget: Int? = null,
  val tokensUsed: Int? = null,
  val timeUsedSeconds: Double? = null,
  val updatedAt: Double? = null,
)

/**
 * A child Acpira itself runs in another CLI (a summoned persona, whose name is the node's `role`); every round of one
 * conversation with the same native session is a node of its own
 */
@Serializable
data class SubagentHarness(
  val agent: AgentId = "",
  val persona: String? = null,
  val mode: RelayMode = RelayMode.unknown,
  val thread: String = "",
  val round: Int = 0,
  /** The child CLI's own session id */
  val sessionId: String? = null,
  /** The reasoning effort the child session really runs; absent: none reported */
  val effort: String? = null,
  /** The group the round was summoned through */
  val group: String? = null,
)

/** A sidekick round's tool rows: a step this build cannot read is dropped, the node itself stays */
object ToolStepsSerializer : KSerializer<List<ToolCallBlock>> by TolerantListSerializer(ToolCallBlock.serializer())

/**
 * One delegated child node (subagents.ts SubagentSummary), whatever the wire dialect; its transcript comes with the
 * `subagent` push once observed
 */
@Serializable
data class SubagentSummary(
  /** Acpira's own stable id, never a peer id */
  val id: String,
  /** Another summary's id; absent: a child of the root session */
  val parentId: String? = null,
  /** The root agent turn (index in SessionView.turns) during which it was announced */
  val turnIndex: Int = 0,
  val title: String? = null,
  /** The delegated task text */
  val task: String? = null,
  /** The agent's profile / subagent type; absent when not structured */
  val role: String? = null,
  val state: SubagentState = SubagentState.unknown,
  /** The latest activity label, kept engine-side */
  val activity: String? = null,
  val toolCount: Int = 0,
  /** The final result text the parent received (for `session` visibility the child's own last text) */
  val result: String? = null,
  val visibility: SubagentVisibility = SubagentVisibility.unknown,
  val stateSource: StateSource = StateSource.unknown,
  val controls: Controls = Controls(),
  val cancelRequested: Boolean? = null,
  /** The agent said the child runs detached */
  val background: Boolean? = null,
  val announcedAt: Double = 0.0,
  val endedAt: Double? = null,
  /** Only when reported */
  val model: String? = null,
  /** Only when a usage update was attributed to this child */
  val usage: Usage? = null,
  val peer: Peer = Peer(),
  /** A receipt node whose transcript has content of its own (a Claude workflow agent's sidechain log) */
  val hasProcess: Boolean? = null,
  /** The child's pending permission cards and open question, mirrored so the root view can show them */
  val permissions: List<PermissionBlock>? = null,
  val question: QuestionBlock? = null,
  /** Set when Acpira itself runs the child in another CLI */
  val harness: SubagentHarness? = null,
  /** A sidekick round's latest foreground tool rows, without their output (the relay's window) */
  @Serializable(with = ToolStepsSerializer::class) val steps: List<ToolCallBlock>? = null,
) {
  /** local: Acpira synthesized the state (disconnected on connection end, the parent prompt returned first) */
  @Serializable(with = StateSource.S::class)
  enum class StateSource { agent, local, unknown;
    object S : TolerantEnumSerializer<StateSource>("SubagentStateSource", entries, unknown)
  }

  /** cancel: only when the agent said this child can be cancelled */
  @Serializable
  data class Controls(val cancel: Boolean = false)

  @Serializable
  data class Usage(val used: Int = 0, val size: Int = 0)

  /** The agent's own identifiers, for diagnostics and replay; sidechainId: a Claude workflow agent's own agentId */
  @Serializable
  data class Peer(val sessionId: String? = null, val agentId: String? = null, val toolCallId: String? = null, val sidechainId: String? = null)
}

/**
 * One child's transcript as the `subagent` push carries it; `rev` grows with every change, so an older push never
 * replaces a newer one
 */
data class SubagentTranscript(val rev: Int, val running: Boolean, val turns: List<Turn>)

/** Account metadata from the SSH engine owning this session; credentials never cross the peer wire */
@Serializable
data class RemoteAuth(val agent: AgentInfo, val accounts: List<AccountInfo> = emptyList(), val action: AccountAction? = null)

@Serializable
data class ForkOrigin(val sessionId: String, val turnIndex: Int = 0, val agent: AgentId? = null)

@Serializable
data class SessionView(
  val id: String,
  val agent: AgentId = "",
  val accountId: String? = null,
  val title: String = "",
  val cwd: String = "",
  val status: SessionStatus = SessionStatus.ready,
  val error: String? = null,
  val canTakeOver: Boolean? = null,
  val authMethods: List<AuthMethodInfo>? = null,
  val turns: List<Turn> = emptyList(),
  val running: Boolean = false,
  val rev: Int? = null,
  val controls: SessionControls = SessionControls(),
  val usage: Usage? = null,
  val commands: List<SlashCommand> = emptyList(),
  val queued: List<QueuedPrompt>? = null,
  val canSteer: Boolean? = null,
  /** A running command row can be stopped on its own (`stopCommand`) */
  val canStopCommands: Boolean? = null,
  val goal: SessionGoal? = null,
  val subagents: List<SubagentSummary>? = null,
  val forkedFrom: ForkOrigin? = null,
  val createdAt: String = "",
  val updatedAt: String = "",
  /** Account metadata of the SSH engine owning this session */
  val remoteAuth: RemoteAuth? = null,
  /** The agent's remembered per-model parameters, for switching the model of an edited prompt */
  val modelShapes: ModelShapes? = null,
  /** The goal controls the running agent advertises; absent without goal support (or no process yet) */
  val goalActions: List<GoalAction>? = null,
)
