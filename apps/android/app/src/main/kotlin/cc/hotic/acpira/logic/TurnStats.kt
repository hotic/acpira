package cc.hotic.acpira.logic

import cc.hotic.acpira.i18n.L10n
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.model.AgentTurn
import cc.hotic.acpira.model.SessionControls
import cc.hotic.acpira.model.ToolCallBlock
import cc.hotic.acpira.model.TurnSettings
import cc.hotic.acpira.model.TurnStop
import cc.hotic.acpira.model.TurnUsage
import java.text.NumberFormat
import java.util.Locale
import kotlin.math.abs
import kotlin.math.floor
import kotlin.math.roundToInt

// apps/ios Logic/TurnStats.swift (chat/turnActionHelpers.ts and TurnActions.tsx StatsCard): what a reply's statistics
// card lists, only the rows the peer reported: the timing tiles, the token lines, the context it left.

/**
 * modelLabel: the model that answered. The peer's own id when it reported one, else the selection captured on the
 * prompt, named through the model control's options as its family (effort, Thinking, Fast and 1M left out; a Fusion
 * pair keeps its whole name); an unknown id shows as is
 */
fun modelLabel(usage: TurnUsage?, settings: TurnSettings?, controls: SessionControls): String? {
  val control = controls.options.firstOrNull { it.category == "model" }
  val id = usage?.model ?: control?.let { settings?.config?.get(it.id) } ?: return null
  val name = control?.options?.firstOrNull { it.id == id }?.name ?: return id
  return if (isFusion(name)) name else ModelCatalog.parse(name).family
}

/** turnTtft: from the prompt going out to the first streamed block; imported history has neither stamp */
fun turnTtft(turn: AgentTurn): String? {
  val started = turn.startedAt ?: return null
  val responded = turn.respondedAt ?: return null
  return if (responded >= started) commandDuration(responded - started) else null
}

/**
 * The turn's generated tokens: `output` already holds `reasoning` on OpenAI-style peers (input + output = total), while
 * peers that count thoughts apart report reasoning above output, or input + output + reasoning = total
 */
private fun generatedTokens(u: TurnUsage?): Int? {
  val output = u?.output ?: return null
  val r = u.reasoning ?: 0
  if (r <= 0) return output
  if (r > output) return output + r
  val total = u.total
  val input = u.input
  if (total != null && input != null && abs(input + output + r - total) < abs(input + output - total)) return output + r
  return output
}

/** The foreground tool runs inside `[from, to]`, as spans; a background call runs beside the model, not instead of it */
private fun foregroundSpans(turn: AgentTurn, from: Double, to: Double): List<Pair<Double, Double>> =
  turn.blocks.mapNotNull { b ->
    if (b !is ToolCallBlock || b.background == true) return@mapNotNull null
    val start = maxOf(from, b.startedAt ?: return@mapNotNull null)
    val end = minOf(to, b.endedAt ?: return@mapNotNull null)
    if (end > start) start to end else null
  }

/** Foreground tool time inside the window, overlapping parallel calls merged so they count once */
private fun foregroundToolMs(turn: AgentTurn, from: Double, to: Double): Double {
  val spans = foregroundSpans(turn, from, to).sortedBy { it.first }
  var current = spans.firstOrNull() ?: return 0.0
  var total = 0.0
  for (span in spans.drop(1)) {
    if (span.first <= current.second) current = current.first to maxOf(current.second, span.second)
    else { total += current.second - current.first; current = span }
  }
  return total + current.second - current.first
}

/**
 * tallyIsLastCall: the tally covers only the turn's last model call (Devin reports the last request's own usage: its
 * input + output matches the end-of-turn context), unless the peer said how many calls it counted
 */
private fun tallyIsLastCall(u: TurnUsage): Boolean {
  u.modelCalls?.let { return it == 1 }
  val context = u.context ?: return false
  val input = u.input ?: return false
  val output = u.output ?: return false
  if (context.used <= 0) return false
  val total = u.total ?: (input + output)
  return abs((total - context.used).toDouble()) <= context.used * 0.02
}

/**
 * turnOutputTps: decode throughput, `(tokens - 1) / seconds`. An agent that timed its own streaming (`decodeMs`) is
 * taken at its word, one first token per call left out. Otherwise the window runs from the first streamed block to the
 * end, foreground tool time and the engine's background waits (`idleMs`) taken out; a tally of the last call alone is
 * timed from the last foreground tool's end. Left out below 5 tokens or a 200 ms window (a buffered dump)
 */
fun turnOutputTps(turn: AgentTurn): Int? {
  val usage = turn.usage ?: return null
  val tokens = generatedTokens(usage)?.takeIf { it >= 5 } ?: return null
  usage.decodeMs?.let { decode ->
    if (decode < 200) return null
    val tps = ((tokens - (usage.modelCalls ?: 1)) * 1000.0 / decode).roundToInt()
    return tps.takeIf { it > 0 }
  }
  val responded = turn.respondedAt ?: return null
  val ended = turn.endedAt ?: return null
  val lastToolEnd = if (tallyIsLastCall(usage)) foregroundSpans(turn, responded, ended).maxOfOrNull { it.second } else null
  val active = lastToolEnd?.let { ended - it }
    ?: (ended - responded - foregroundToolMs(turn, responded, ended) - (turn.idleMs ?: 0.0))
  if (active < 200) return null
  val tps = ((tokens - 1) * 1000.0 / active).roundToInt()
  return tps.takeIf { it > 0 }
}

/** One timing tile (duration, first token, output speed) or one token line; `chip` is the share riding beside the value */
data class StatLine(val label: String, val value: String, val chip: String? = null)

/** The context the turn left: its share as the chip, then used over size */
data class StatContext(val share: String, val used: String, val size: String)

/**
 * What StatsCard shows, only what the peer reported: the timing tiles, the token lines with the cache share as a chip on
 * the input line, then the post-turn context
 */
data class TurnStatsContent(
  val tiles: List<StatLine> = emptyList(),
  val tokens: List<StatLine> = emptyList(),
  val context: StatContext? = null,
)

fun turnStatsContent(turn: AgentTurn): TurnStatsContent {
  val usage = turn.usage
  val locale = Locale.forLanguageTag(L10n.locale)
  val numbers = NumberFormat.getIntegerInstance(locale)
  val percents = NumberFormat.getPercentInstance(locale).apply { maximumFractionDigits = 0 }
  fun number(n: Int) = numbers.format(n)
  // Floored, so a 99.6% hit never reads as a full 100%
  fun share(part: Int, whole: Int) = percents.format(floor(part.toDouble() / whole * 100) / 100)
  val tiles = buildList {
    if (turn.startedAt != null && turn.endedAt != null) add(StatLine(t("stats.duration"), elapsedDuration(turn)))
    turnTtft(turn)?.let { add(StatLine(t("stats.firstToken"), it)) }
    turnOutputTps(turn)?.let { add(StatLine(t("stats.outputShort"), "${number(it)} tok/s")) }
  }
  // A stopped turn whose counts are all zero reported nothing (claude-agent-acp answers a cancel with zeros)
  val counts = listOf(usage?.input, usage?.output, usage?.cachedRead, usage?.cachedWrite, usage?.reasoning)
  val unreported = turn.stop == TurnStop.cancelled && counts.none { (it ?: 0) > 0 }
  // input / output whenever reported, 0 included; cache write / reasoning only above zero (Grok reports a 0 write on
  // every turn); the cache read is the input line's chip, not a line of its own
  val tokens = if (unreported) emptyList() else listOf(
    Triple(usage?.input, "stats.input", true), Triple(usage?.cachedWrite, "stats.cachedWrite", false),
    Triple(usage?.output, "stats.output", true), Triple(usage?.reasoning, "stats.reasoning", false),
  ).mapNotNull { (value, key, always) ->
    if (value == null || !(always || value > 0)) return@mapNotNull null
    val read = usage?.cachedRead
    val chip = if (key == "stats.input" && read != null && read > 0 && value > 0) t("stats.cacheHit", "pct" to share(read, value)) else null
    StatLine(t(key), number(value), chip)
  }
  val context = usage?.context?.takeIf { it.size > 0 }?.let { StatContext(share(it.used, it.size), number(it.used), number(it.size)) }
  return TurnStatsContent(tiles, tokens, context)
}

/** A Fusion pair's name ("Fusion (A High + B)"), which the label keeps whole; `ModelCatalog.parse` leaves pairs out */
private val fusionName = Regex("""^Fusion\s*\((.+?)\s+\+\s+(.+)\)$""", RegexOption.IGNORE_CASE)

private fun isFusion(name: String) = fusionName.matches(name.trim())
