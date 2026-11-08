package cc.hotic.acpira.store

import android.content.Context
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.model.AgentInfo
import cc.hotic.acpira.model.AgentTurn
import cc.hotic.acpira.model.SessionSummary
import cc.hotic.acpira.model.SessionView
import cc.hotic.acpira.model.TextBlock
import cc.hotic.acpira.model.ThoughtBlock
import cc.hotic.acpira.model.ToolCallBlock
import cc.hotic.acpira.model.ToolKind
import cc.hotic.acpira.model.ToolStatus
import cc.hotic.acpira.model.TurnStop
import cc.hotic.acpira.model.WireJson
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import java.time.Instant

/**
 * The synthetic sessions the iOS app's demo shows (apps/ios AcpiraFixtures/synthetic, synced into assets/fixtures by
 * the build): the same files, so a screenshot of a fixture on either phone compares like for like
 */
object Fixtures {
  /** Fixture names that hold a SessionView (AcpiraFixtures Fixtures.sessionNames) */
  val sessionNames = listOf(
    "showcase", "running", "streaming", "question", "queued", "stopped", "empty", "diagrams", "media", "plan", "notices",
    "records", "context", "failure", "login", "readonly", "closed", "error", "connecting", "goal", "subagents",
    "builtin", "fusion",
  )

  private fun read(context: Context, name: String): String? =
    runCatching { context.assets.open("fixtures/$name.json").bufferedReader().use { it.readText() } }.getOrNull()

  fun session(context: Context, name: String): SessionView? =
    read(context, name)?.let { runCatching { WireJson.decodeFromString(SessionView.serializer(), it) }.getOrNull() }

  fun agents(context: Context): List<AgentInfo> =
    read(context, "agents")?.let { WireJson.decodeFromString(ListSerializer(AgentInfo.serializer()), it) }.orEmpty()

  fun sessions(context: Context): List<SessionSummary> =
    read(context, "sessions")?.let { WireJson.decodeFromString(ListSerializer(SessionSummary.serializer()), it) }.orEmpty()

  /** The fixtures' "now": timestamps are written relative to it */
  fun now(context: Context): Instant = read(context, "clock")
    ?.let { runCatching { Instant.parse(WireJson.parseToJsonElement(it).jsonObject["now"]!!.jsonPrimitive.content) }.getOrNull() }
    ?: Instant.now()
}

/**
 * The reply a prompt gets while no engine is connected (apps/ios Logic/DemoReply.swift): a short thought, one read in
 * the project, and a paragraph saying where the reply comes from and echoing the prompt
 */
object DemoReply {
  fun turn(prompt: String, cwd: String, startedMs: Double): AgentTurn {
    val quoted = prompt.lines().joinToString("\n") { "> $it" }
    val project = cwd.split('/').lastOrNull { it.isNotEmpty() } ?: cwd
    return AgentTurn(
      blocks = listOf(
        ThoughtBlock(text = t("mobile.demo.thought"), startedAt = startedMs, durationSec = 2.0),
        ToolCallBlock(id = "demo-read-${startedMs.toLong()}", kind = ToolKind.read, verb = t("verb.read"), target = "README.md", targetMono = true, status = ToolStatus.completed),
        TextBlock(markdown = t("mobile.demo.reply", "project" to project) + if (quoted.isBlank()) "" else "\n\n$quoted"),
      ),
      startedAt = startedMs,
      endedAt = startedMs + 3000,
      stop = TurnStop.end_turn,
    )
  }
}
