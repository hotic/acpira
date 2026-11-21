package cc.hotic.acpira.logic

import cc.hotic.acpira.i18n.L10n
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.model.UsageModel
import cc.hotic.acpira.model.UsageRow
import cc.hotic.acpira.model.UsageSession
import cc.hotic.acpira.model.UsageView
import java.math.BigDecimal
import java.math.MathContext
import java.math.RoundingMode
import java.text.DecimalFormat
import java.text.DecimalFormatSymbols
import java.text.NumberFormat
import java.time.DayOfWeek
import java.time.Instant
import java.time.LocalDate
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.TextStyle
import java.util.Locale
import kotlin.math.abs
import kotlin.math.floor
import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToInt
import kotlin.math.roundToLong

// Mirror of packages/ui/usage/summary.ts and formatting helpers from packages/ui/usage/UsagePage.tsx: folds the
// engine's hour-bucketed rows into local days and hours, summary figures, trend columns and model / agent / project
// breakdowns.

enum class UsagePeriod(val wire: String) {
  Days7("7"),
  Days30("30"),
  All("all");

  fun label(): String = when (this) {
    Days7 -> t("usagePage.period.days", "n" to 7)
    Days30 -> t("usagePage.period.days", "n" to 30)
    All -> t("usagePage.period.all")
  }
}

enum class UsageDim(val wire: String) {
  Model("model"),
  Agent("agent"),
  Project("project");

  fun label(): String = t("usagePage.dim.$wire")
  fun moreKey(): String = "usagePage.more.$wire"
}

enum class UsageStep { Hours, Day, Week, Month }

data class UsageModelLabel(val name: String, val brand: String?)

data class UsageModelLabels(
  val keys: List<String>,
  val label: Map<String, UsageModelLabel>,
  val projects: List<String>,
)

data class UsageDay(
  val day: Long,
  val tokens: Long,
  val byModel: Map<String, Long>,
  val byAgent: Map<String, Long>,
  val byProject: Map<String, Long>,
) {
  fun tokensFor(dim: UsageDim, key: String): Long = when (dim) {
    UsageDim.Model -> byModel[key] ?: 0L
    UsageDim.Agent -> byAgent[key] ?: 0L
    UsageDim.Project -> byProject[key] ?: 0L
  }
}

data class UsagePart(val key: String, val tokens: Long, val share: Double)

data class UsageSummary(
  val tokens: Long = 0L,
  val workMs: Long = 0L,
  val longestMs: Long = 0L,
  val prompts: Int = 0,
  val turns: Int = 0,
  val tools: Int = 0,
  val added: Int = 0,
  val removed: Int = 0,
  val sessions: Int = 0,
  val modelParts: List<UsagePart> = emptyList(),
  val agentParts: List<UsagePart> = emptyList(),
  val projectParts: List<UsagePart> = emptyList(),
) {
  fun parts(dim: UsageDim): List<UsagePart> = when (dim) {
    UsageDim.Model -> modelParts
    UsageDim.Agent -> agentParts
    UsageDim.Project -> projectParts
  }
}

data class UsageTrendSpan(val start: Long, val end: Long)
data class UsageTrendColumns(val step: UsageStep, val cols: List<UsageTrendSpan>)
data class UsageFigure(val value: String, val unit: String?)

object UsageStats {
  const val CHATS_KEY = "\u0000chats"
  const val TREND_SPAN_HOURS = 4
  const val TREND_DAYS_MAX = 31
  const val TREND_WEEKS_MAX = 26
  const val DOT_STEPS = 4
  const val KEEP_ROWS = 10
  private const val HOUR_MS = 3_600_000L
  private const val DAY_MS = 86_400_000L

  private val effortSuffix = Regex("""[\s._-]+(minimal|low|medium|high|xhigh|max)$""")
  private val separators = Regex("""[\s._-]+""")
  private val leadingVendor = Regex("""^(claude|gpt|kimi)""")
  private val hanThenAscii = Regex("""([\u4e00-\u9fff])(?=[0-9A-Za-z])""")
  private val asciiThenHan = Regex("""([0-9A-Za-z])(?=[\u4e00-\u9fff])""")
  private val trailingSlashes = Regex("""/+$""")

  // Calendar slots

  fun dayOf(ms: Long, zone: ZoneId = ZoneId.systemDefault()): Long =
    Instant.ofEpochMilli(ms).atZone(zone).toLocalDate().atStartOfDay(zone).toInstant().toEpochMilli()

  fun addDays(day: Long, n: Int, zone: ZoneId = ZoneId.systemDefault()): Long =
    Instant.ofEpochMilli(day).atZone(zone).toLocalDate().plusDays(n.toLong()).atStartOfDay(zone).toInstant().toEpochMilli()

  fun hourOf(ms: Long, zone: ZoneId = ZoneId.systemDefault()): Long {
    val z = Instant.ofEpochMilli(ms).atZone(zone)
    return z.withMinute(0).withSecond(0).withNano(0).toInstant().toEpochMilli()
  }

  fun addHours(hour: Long, n: Int, zone: ZoneId = ZoneId.systemDefault()): Long =
    Instant.ofEpochMilli(hour).atZone(zone).toLocalDateTime().plusHours(n.toLong()).atZone(zone).toInstant().toEpochMilli()

  /** Monday = 0 … Sunday = 6 (matching `(getDay() + 6) % 7` in summary.ts) */
  fun mondayOffset(day: Long, zone: ZoneId = ZoneId.systemDefault()): Int =
    Instant.ofEpochMilli(day).atZone(zone).dayOfWeek.value - 1

  // Model and project grouping

  fun modelKey(name: String): String {
    val bare = name.substringAfterLast('/').lowercase()
      .replace(effortSuffix, "")
      .replace(separators, "")
    return bare.replace(leadingVendor, "").ifEmpty { bare }
  }

  private fun nameRank(n: String): Int {
    val hasSpace = n.any { it.isWhitespace() }
    val hasUpper = n.any { it in 'A'..'Z' }
    return (if (hasSpace && hasUpper) 2 else 0) + (if ('/' in n) 0 else 1)
  }

  fun isChatFolder(chatRoot: String?, cwd: String): Boolean {
    val root = chatRoot?.takeIf { it.isNotEmpty() } ?: return false
    val cut = max(cwd.lastIndexOf('/'), cwd.lastIndexOf('\\'))
    if (cut < 0) return false
    fun clean(p: String) = p.replace('\\', '/').replace(trailingSlashes, "")
    return clean(cwd.substring(0, cut)) == clean(root)
  }

  fun modelLabels(view: UsageView, chats: String?, brandOf: (String) -> String?): UsageModelLabels {
    val projects = view.sessions.map { if (isChatFolder(chats, it.cwd)) CHATS_KEY else it.cwd }
    val keys = view.models.map { modelKey(it.name) }
    val label = LinkedHashMap<String, UsageModelLabel>()
    view.models.forEachIndexed { i, m ->
      val k = keys[i]
      val cur = label[k]
      if (cur != null) {
        val curRank = nameRank(cur.name)
        val nextRank = nameRank(m.name)
        if (curRank > nextRank || (curRank == nextRank && cur.name.length >= m.name.length)) return@forEachIndexed
      }
      label[k] = UsageModelLabel(
        name = m.name.substringAfterLast('/'),
        brand = brandOf(m.id) ?: brandOf(m.name),
      )
    }
    return UsageModelLabels(keys, label, projects)
  }

  fun keyOf(view: UsageView, models: UsageModelLabels, r: UsageRow, dim: UsageDim): String = when (dim) {
    UsageDim.Model -> r.model?.let { models.keys.getOrNull(it) }.orEmpty()
    UsageDim.Agent -> view.sessions.getOrNull(r.session)?.agent.orEmpty()
    UsageDim.Project -> models.projects.getOrNull(r.session).orEmpty()
  }

  // Folding and summaries

  fun daysOf(view: UsageView, models: UsageModelLabels, zone: ZoneId = ZoneId.systemDefault()): Map<Long, UsageDay> =
    foldBy(view, models) { dayOf(it, zone) }

  fun hoursOf(view: UsageView, models: UsageModelLabels, zone: ZoneId = ZoneId.systemDefault()): Map<Long, UsageDay> =
    foldBy(view, models) { hourOf(it, zone) }

  private fun foldBy(view: UsageView, models: UsageModelLabels, slot: (Long) -> Long): Map<Long, UsageDay> {
    class Acc(
      val day: Long,
      var tokens: Long = 0L,
      val byModel: HashMap<String, Long> = HashMap(),
      val byAgent: HashMap<String, Long> = HashMap(),
      val byProject: HashMap<String, Long> = HashMap(),
    )
    val map = LinkedHashMap<Long, Acc>()
    for (r in view.rows) {
      val bucket = slot(r.at)
      val acc = map.getOrPut(bucket) { Acc(bucket) }
      val tokens = r.tokens ?: 0L
      acc.tokens += tokens
      val mk = keyOf(view, models, r, UsageDim.Model)
      acc.byModel[mk] = (acc.byModel[mk] ?: 0L) + tokens
      val ak = keyOf(view, models, r, UsageDim.Agent)
      acc.byAgent[ak] = (acc.byAgent[ak] ?: 0L) + tokens
      val pk = keyOf(view, models, r, UsageDim.Project)
      acc.byProject[pk] = (acc.byProject[pk] ?: 0L) + tokens
    }
    return map.mapValues { (_, a) -> UsageDay(a.day, a.tokens, a.byModel, a.byAgent, a.byProject) }
  }

  fun periodStart(period: UsagePeriod, today: Long, zone: ZoneId = ZoneId.systemDefault()): Long = when (period) {
    UsagePeriod.All -> 0L
    UsagePeriod.Days7 -> addDays(today, -6, zone)
    UsagePeriod.Days30 -> addDays(today, -29, zone)
  }

  fun summarize(view: UsageView, models: UsageModelLabels, from: Long): UsageSummary {
    var tokensTotal = 0L
    var workMs = 0L
    var longestMs = 0L
    var prompts = 0
    var turns = 0
    var tools = 0
    var added = 0
    var removed = 0
    val sessions = HashSet<Int>()
    val byModel = HashMap<String, Long>()
    val byAgent = HashMap<String, Long>()
    val byProject = HashMap<String, Long>()
    for (r in view.rows) {
      if (r.at + HOUR_MS <= from) continue
      val tokens = r.tokens ?: 0L
      tokensTotal += tokens
      workMs += r.workMs ?: 0L
      longestMs = max(longestMs, r.longestMs ?: 0L)
      prompts += r.prompts ?: 0
      turns += r.turns ?: 0
      tools += r.tools ?: 0
      added += r.added ?: 0
      removed += r.removed ?: 0
      sessions += r.session
      val mk = keyOf(view, models, r, UsageDim.Model)
      byModel[mk] = (byModel[mk] ?: 0L) + tokens
      val ak = keyOf(view, models, r, UsageDim.Agent)
      byAgent[ak] = (byAgent[ak] ?: 0L) + tokens
      val pk = keyOf(view, models, r, UsageDim.Project)
      byProject[pk] = (byProject[pk] ?: 0L) + tokens
    }
    fun rank(totals: Map<String, Long>): List<UsagePart> = totals.entries
      .filter { (k, v) -> v > 0L && k.isNotEmpty() }
      .map { (k, v) -> UsagePart(k, v, if (tokensTotal > 0L) v.toDouble() / tokensTotal.toDouble() else 0.0) }
      .sortedWith(compareByDescending<UsagePart> { it.tokens }.thenBy { it.key })
    return UsageSummary(
      tokens = tokensTotal, workMs = workMs, longestMs = longestMs, prompts = prompts, turns = turns,
      tools = tools, added = added, removed = removed, sessions = sessions.size,
      modelParts = rank(byModel), agentParts = rank(byAgent), projectParts = rank(byProject),
    )
  }

  fun trendColumns(
    period: UsagePeriod,
    today: Long,
    first: Long,
    now: Long,
    zone: ZoneId = ZoneId.systemDefault(),
  ): UsageTrendColumns {
    fun every(step: UsageStep, from: Long, next: (Long) -> Long, until: Long): UsageTrendColumns {
      val cols = ArrayList<UsageTrendSpan>()
      var d = from
      while (d <= until) {
        val end = next(d)
        cols.add(UsageTrendSpan(d, end))
        if (end <= d) break
        d = end
      }
      if (step != UsageStep.Hours && cols.isNotEmpty()) {
        val last = cols.last()
        cols[cols.lastIndex] = last.copy(end = min(last.end, addDays(today, 1, zone)))
      }
      return UsageTrendColumns(step, cols)
    }

    return when (period) {
      UsagePeriod.Days7 -> every(UsageStep.Hours, periodStart(period, today, zone), { addHours(it, TREND_SPAN_HOURS, zone) }, now)
      UsagePeriod.Days30 -> every(UsageStep.Day, periodStart(period, today, zone), { addDays(it, 1, zone) }, today)
      UsagePeriod.All -> {
        val span = ((today - first).toDouble() / DAY_MS.toDouble()).roundToInt() + 1
        when {
          span <= TREND_DAYS_MAX -> every(UsageStep.Day, first, { addDays(it, 1, zone) }, today)
          span <= TREND_WEEKS_MAX * 7 -> every(
            UsageStep.Week,
            addDays(first, -mondayOffset(first, zone), zone),
            { addDays(it, 7, zone) },
            today,
          )
          else -> {
            val firstDate = Instant.ofEpochMilli(first).atZone(zone).toLocalDate().withDayOfMonth(1)
            val start = firstDate.atStartOfDay(zone).toInstant().toEpochMilli()
            every(
              UsageStep.Month,
              start,
              { d -> Instant.ofEpochMilli(d).atZone(zone).toLocalDate().plusMonths(1).withDayOfMonth(1).atStartOfDay(zone).toInstant().toEpochMilli() },
              today,
            )
          }
        }
      }
    }
  }

  fun firstDay(days: Map<Long, UsageDay>, today: Long): Long {
    var first = today
    for (d in days.values) if (d.tokens > 0L && d.day < first) first = d.day
    return first
  }

  fun currentRun(days: Map<Long, UsageDay>, today: Long, zone: ZoneId = ZoneId.systemDefault()): Int {
    var day = if ((days[today]?.tokens ?: 0L) > 0L) today else addDays(today, -1, zone)
    var n = 0
    while ((days[day]?.tokens ?: 0L) > 0L) {
      n++
      day = addDays(day, -1, zone)
    }
    return n
  }

  // Formatting

  private fun javaLocale(localeId: String): Locale = Locale.forLanguageTag(localeId)

  /** Plain up to 9,999, else locale compact notation with at most 3 significant digits split into `(value, unit)` */
  fun figure(n: Long, localeId: String = L10n.locale): UsageFigure {
    val locale = javaLocale(localeId)
    if (abs(n) < 10_000L) {
      return UsageFigure(NumberFormat.getIntegerInstance(locale).format(n), null)
    }
    val absN = abs(n).toDouble()
    val (scale, unit) = when {
      localeId.startsWith("zh-TW") || localeId.startsWith("zh-Hant") -> when {
        absN >= 1e12 -> 1e12 to "兆"
        absN >= 1e8 -> 1e8 to "億"
        else -> 1e4 to "萬"
      }
      localeId.startsWith("zh") -> when {
        absN >= 1e12 -> 1e12 to "万亿"
        absN >= 1e8 -> 1e8 to "亿"
        else -> 1e4 to "万"
      }
      localeId.startsWith("ja") -> when {
        absN >= 1e12 -> 1e12 to "兆"
        absN >= 1e8 -> 1e8 to "億"
        else -> 1e4 to "万"
      }
      localeId.startsWith("ko") -> when {
        absN >= 1e12 -> 1e12 to "조"
        absN >= 1e8 -> 1e8 to "억"
        else -> 1e4 to "만"
      }
      else -> when {
        absN >= 1e12 -> 1e12 to "T"
        absN >= 1e9 -> 1e9 to "B"
        absN >= 1e6 -> 1e6 to "M"
        else -> 1e3 to "K"
      }
    }
    val rounded = BigDecimal.valueOf(n.toDouble() / scale).round(MathContext(3, RoundingMode.HALF_UP)).stripTrailingZeros()
    val symbols = DecimalFormatSymbols(locale)
    val plain = rounded.toPlainString().replace('.', symbols.decimalSeparator)
    return UsageFigure(plain, unit)
  }

  /** Pangu spacing between Han characters and ASCII letters/digits for Chinese locales */
  fun spaced(text: String, localeId: String = L10n.locale): String {
    if (!localeId.startsWith("zh")) return text
    return text.replace(hanThenAscii, "$1 ").replace(asciiThenHan, "$1 ")
  }

  fun duration(ms: Long, localeId: String = L10n.locale): Pair<String, String> {
    val minutes = (ms.toDouble() / 60_000.0).roundToInt()
    if (minutes < 60) return "$minutes" to t("usagePage.unit.minutes")
    val hours = minutes.toDouble() / 60.0
    val fmt = DecimalFormat(if (hours >= 100.0) "#,##0" else "#,##0.#", DecimalFormatSymbols(javaLocale(localeId)))
    return fmt.format(hours) to t("usagePage.unit.hours")
  }

  fun percent(share: Double, localeId: String = L10n.locale): String {
    val locale = javaLocale(localeId)
    val fmt = NumberFormat.getPercentInstance(locale).apply {
      maximumFractionDigits = if (share > 0.0 && share < 0.01) 1 else 0
      minimumFractionDigits = 0
    }
    if (share > 0.0 && share < 0.001) return "<${fmt.format(0.001)}"
    return fmt.format(floor(share * 1000.0) / 1000.0)
  }

  fun monthShort(ms: Long, localeId: String = L10n.locale, zone: ZoneId = ZoneId.systemDefault()): String {
    val date = Instant.ofEpochMilli(ms).atZone(zone).toLocalDate()
    if (date.monthValue == 1) return date.year.toString()
    val raw = date.month.getDisplayName(TextStyle.SHORT, javaLocale(localeId))
    return spaced(raw, localeId)
  }

  fun numericDate(ms: Long, zone: ZoneId = ZoneId.systemDefault()): String {
    val date = Instant.ofEpochMilli(ms).atZone(zone).toLocalDate()
    return "${date.monthValue}/${date.dayOfMonth}"
  }

  fun dayLabel(ms: Long, localeId: String = L10n.locale, zone: ZoneId = ZoneId.systemDefault()): String {
    val date = Instant.ofEpochMilli(ms).atZone(zone).toLocalDate()
    val weekday = date.dayOfWeek.getDisplayName(TextStyle.SHORT, javaLocale(localeId))
    return "${date.monthValue}/${date.dayOfMonth} $weekday"
  }

  fun monthLong(ms: Long, localeId: String = L10n.locale, zone: ZoneId = ZoneId.systemDefault()): String {
    val date = Instant.ofEpochMilli(ms).atZone(zone).toLocalDate()
    val loc = javaLocale(localeId)
    val raw = if (localeId.startsWith("zh") || localeId.startsWith("ja")) {
      "${date.year}年${date.monthValue}月"
    } else if (localeId.startsWith("ko")) {
      "${date.year}년 ${date.monthValue}월"
    } else {
      DateTimeFormatter.ofPattern("MMMM yyyy", loc).format(date)
    }
    return spaced(raw, localeId)
  }

  fun weekRange(start: Long, endInclusive: Long, zone: ZoneId = ZoneId.systemDefault()): String =
    "${numericDate(start, zone)}–${numericDate(endInclusive, zone)}"

  // Deterministic demo sample (mirror of lab/usage.preview.tsx)

  fun sampleView(nowMs: Long, chats: String?, zone: ZoneId = ZoneId.systemDefault()): UsageView {
    var state = 20261121
    fun rnd(): Double {
      state += 0x6d2b79f5
      var z = (state xor (state ushr 15)) * (state or 1)
      z = z xor (z + ((z xor (z ushr 7)) * (z or 61)))
      return ((z xor (z ushr 14)).toLong() and 0xffffffffL).toDouble() / 4294967296.0
    }

    val root = "/Users/dev/projects/acpira"
    val work = "/Users/dev/projects/Work-Local"
    val pc = "ssh://asgard/srv/studio-pc"
    val keke = "/Users/dev/projects/keke"
    val chat = "${chats ?: "/Users/dev/.acpira/chats"}/quick-chat"

    val sessions = listOf(
      UsageSession("claude", root),
      UsageSession("claude", work),
      UsageSession("codex", root),
      UsageSession("acpira", root),
      UsageSession("codex", pc),
      UsageSession("kimi", keke),
      UsageSession("acpira", chat),
    )
    val models = listOf(
      UsageModel("opus", "Opus 5.5"),
      UsageModel("sonnet", "Sonnet 5.5"),
      UsageModel("gpt-6-astra", "6 Astra"),
      UsageModel("asgard/deepseek-v4.1-flash", "DeepSeek V4.1 Flash"),
      UsageModel("kimi-code/k3", "K3"),
    )
    val modelFor = intArrayOf(0, 1, 2, 3, 2, 4, 3)

    val todayMs = dayOf(nowMs, zone)
    val startMs = addDays(todayMs, -300, zone)
    val rows = ArrayList<UsageRow>()

    for (d in 0..300) {
      val dayMs = addDays(startMs, d, zone)
      val dow = Instant.ofEpochMilli(dayMs).atZone(zone).dayOfWeek
      val weekend = dow == DayOfWeek.SATURDAY || dow == DayOfWeek.SUNDAY
      if (d < 294 && rnd() < (if (weekend) 0.55 else 0.12)) continue
      val ramp = 0.3 + (d.toDouble() / 300.0) * 0.9
      for (h in 9 until 24) {
        if (rnd() < 0.55) continue
        val s = min(sessions.lastIndex, (rnd() * sessions.size).toInt())
        val peak = if (h in 14..17) 2.2 else 1.0
        val tokens = (peak * ramp * (0.3 + rnd()) * 6_000_000.0).roundToLong()
        val at = addHours(dayMs, h, zone)
        val turns = 1 + (rnd() * 6.0).toInt()
        val workMs = (turns * (2.0 + rnd() * 9.0) * 60_000.0).roundToLong()
        val longestMs = (workMs.toDouble() / turns * (1.0 + rnd())).roundToLong()
        val prompts = max(0, turns - (if (rnd() < 0.2) 1 else 0))
        rows.add(
          UsageRow(
            at = at, session = s, model = modelFor[s], tokens = tokens,
            input = (tokens * 0.97).roundToLong(), output = (tokens * 0.03).roundToLong(),
            turns = turns, prompts = prompts, workMs = workMs, longestMs = longestMs,
            tools = turns * 7, added = (rnd() * 300.0).roundToInt(), removed = (rnd() * 120.0).roundToInt(),
          ),
        )
      }
    }
    return UsageView(sessions, models, rows)
  }
}
