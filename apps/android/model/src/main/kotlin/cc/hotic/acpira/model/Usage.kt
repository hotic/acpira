package cc.hotic.acpira.model

import kotlinx.serialization.Serializable

// Mirror of packages/shared/usage.ts and rust/crates/acpira-shared/src/usage.rs: hour-bucketed usage across every
// session, folded on the client into local days, summary tiles, trend columns and model / agent / project breakdowns.

/** A session the usage rows index into; [cwd] is its project folder */
@Serializable
data class UsageSession(
  val agent: AgentId = "",
  val cwd: String = "",
)

/** A model the usage rows index into: the wire id and the family name shown in pickers */
@Serializable
data class UsageModel(
  val id: String = "",
  val name: String = "",
)

/** One session's turns in one model within one UTC clock hour; zero fields are omitted on the wire */
@Serializable
data class UsageRow(
  /** The hour's start, Unix milliseconds (whole UTC hours) */
  val at: Long = 0,
  /** Index into [UsageView.sessions] */
  val session: Int = 0,
  /** Index into [UsageView.models]; null when nothing named the model */
  val model: Int? = null,
  val tokens: Long? = null,
  val input: Long? = null,
  val output: Long? = null,
  val cachedRead: Long? = null,
  val turns: Int? = null,
  val prompts: Int? = null,
  val workMs: Long? = null,
  val longestMs: Long? = null,
  val tools: Int? = null,
  val added: Int? = null,
  val removed: Int? = null,
)

/** Usage across every session on the engine (`HostMsg.usage`) */
@Serializable
data class UsageView(
  val sessions: List<UsageSession> = emptyList(),
  val models: List<UsageModel> = emptyList(),
  /** Ordered by [UsageRow.at] */
  val rows: List<UsageRow> = emptyList(),
)
