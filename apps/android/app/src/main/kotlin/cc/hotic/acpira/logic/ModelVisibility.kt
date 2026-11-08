package cc.hotic.acpira.logic

import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.model.ConfigControl
import cc.hotic.acpira.model.SessionOption
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull

// apps/ios Logic/ModelVisibility.swift (shared/models.ts familyHidden, setFamilyVisible, visibleOptions and
// shared/modelCatalog.ts): which model families an agent's menus offer. The hidden lists live in the engine's settings
// (`hiddenOptions`, agent → control id → family keys), shared with the desktop, so the keys and the legacy forms are
// the TS ones.

object ModelVisibility {
  private val noBrand: (SessionOption) -> String? = { null }

  /** Preferences stored before the family had a source: its bare name, or the `Provider/Name` Pi and OpenCode showed */
  private fun nameLegacyKeys(f: ModelFamily, name: String): List<String> {
    val keys = f.source?.let { mutableListOf(name, "$it/$name") } ?: mutableListOf(name)
    if (f.key.startsWith("[")) {
      val parts = runCatching { (Json.parseToJsonElement(f.key) as JsonArray).map { (it as? JsonPrimitive)?.contentOrNull } }.getOrNull()
      val first = parts?.firstOrNull()
      if (parts != null && parts.size == 2 && first != null && parts[1] != null) keys.add(ModelCatalog.jsonKey(listOf(first, name)))
    }
    return keys
  }

  private fun legacyKeys(f: ModelFamily) = nameLegacyKeys(f, f.name)

  private fun allVariantsHidden(f: ModelFamily, hidden: List<String>) =
    f.variants.size > 1 && f.variants.all { v -> nameLegacyKeys(f, v.name).any { it in hidden } }

  fun hidden(f: ModelFamily, hidden: List<String>): Boolean =
    f.key in hidden || legacyKeys(f).any { it in hidden } || allVariantsHidden(f, hidden)

  /** One family switched; legacy name-only entries become the family's key first, so its siblings keep their state */
  fun setVisible(options: List<SessionOption>, hidden: List<String>, key: String, show: Boolean): List<String> {
    val next = hidden.toMutableList()
    for (f in ModelCatalog.group(options, noBrand)) {
      val wasHidden = hidden(f, hidden)
      for (k in legacyKeys(f)) if (k in hidden) next.removeAll { it == k }
      if (f.variants.size > 1) for (v in f.variants) for (k in nameLegacyKeys(f, v.name)) next.removeAll { it == k }
      if (wasHidden && f.key !in next) next.add(f.key)
    }
    next.removeAll { it == key }
    if (!show) next.add(key)
    return next
  }

  /** The options a menu offers: hidden families dropped, the current value always kept, and a list that would hide
   *  everything shows everything */
  fun visibleOptions(options: List<SessionOption>, hidden: List<String>?, current: String?): List<SessionOption> {
    if (hidden.isNullOrEmpty()) return options
    val off = ModelCatalog.group(options, noBrand).filter { hidden(it, hidden) }.flatMap { f -> f.variants.map { it.id } }.toSet()
    val kept = options.filter { it.id == current || it.id !in off }
    return kept.ifEmpty { options }
  }

  /** Every listed family switched at once, rows a search leaves out included */
  fun setAllVisible(families: List<ModelFamily>, hidden: List<String>, visible: Boolean): List<String> {
    val next = hidden.toMutableList()
    for (f in families) {
      next.removeAll { it == f.name || it == f.key }
      if (!visible) next.add(f.key)
    }
    return next
  }

  fun filter(families: List<ModelFamily>, query: String): List<ModelFamily> {
    val terms = query.lowercase().split(Regex("\\s+")).filter { it.isNotEmpty() }
    if (terms.isEmpty()) return families
    return families.filter { f ->
      val text = (listOf(f.name, f.source.orEmpty()) + f.variants.flatMap { listOf(it.id, it.name) }).joinToString(" ").lowercase()
      terms.all { text.contains(it) }
    }
  }

  /** Each series' newest release first, then the older ones; unrelated series keep the agent's order */
  fun prioritize(families: List<ModelFamily>): List<ModelFamily> {
    data class Entry(val family: ModelFamily, val version: List<Int>, val at: Int)
    val order = mutableListOf<String>()
    val buckets = HashMap<String, MutableList<Entry>>()
    families.forEachIndexed { at, f ->
      val (series, version) = release(f)
      if (series !in buckets) order.add(series)
      buckets.getOrPut(series) { mutableListOf() }.add(Entry(f, version, at))
    }
    val sorted = order.map { series ->
      buckets.getValue(series).sortedWith { l, r ->
        val n = maxOf(l.version.size, r.version.size)
        (0 until n).map { (r.version.getOrNull(it) ?: 0) - (l.version.getOrNull(it) ?: 0) }.firstOrNull { it != 0 } ?: (l.at - r.at)
      }
    }
    return sorted.mapNotNull { it.firstOrNull()?.family } + sorted.flatMap { s -> s.drop(1).map { it.family } }
  }

  private val versionRe = Regex("\\d+(?:[.-]\\d+)*")
  private val sizeRe = Regex("^[bkmt]\\b", RegexOption.IGNORE_CASE)

  /** A family's series (its name with the version cut out, per source) and version; a date or a size suffix is no
   *  version, so such a family is a series of its own */
  private fun release(f: ModelFamily): Pair<String, List<Int>> {
    val own = f.key to emptyList<Int>()
    val match = versionRe.find(f.name) ?: return own
    val number = match.value
    val rest = f.name.substring(match.range.last + 1)
    if (number.matches(Regex("\\d{6,}")) || sizeRe.containsMatchIn(rest)) return own
    val series = (f.name.substring(0, match.range.first) + " " + rest).lowercase().replace(Regex("[\\s._-]+"), " ").trim()
    return ModelCatalog.jsonKey(listOf(f.sourceKind, f.source, series)) to number.split('.', '-').mapNotNull { it.toIntOrNull() }
  }
}

/** The settings' models page data (Settings/EngineSettings.swift ModelSettings, AgentPage.tsx ModelsSection) */
object ModelSettings {
  private val sidekick = setOf(ComposerControls.sidekickCategory, ComposerControls.sidekickEffortCategory, ComposerControls.sidekickFastCategory)

  data class Catalog(val control: ConfigControl, val families: List<ModelFamily>)
  data class Group(val control: ConfigControl, val key: String, val title: String, val rows: List<ModelFamily>, val sources: List<String>)

  /** The controls whose options are models (no reasoning levels, no switches, not the built-in agent's sidekick pick,
   *  its levels and its Fast), their families newest first; null before the engine answered */
  fun catalogs(controls: List<ConfigControl>?): List<Catalog>? =
    controls?.filter { !ComposerControls.isReasoning(it) && it.type != "boolean" && it.category !in sidekick }
      ?.map { Catalog(it, ModelVisibility.prioritize(ModelCatalog.group(it.options) { null })) }

  /** Families shown and listed over every catalog */
  fun count(catalogs: List<Catalog>, hidden: Map<String, List<String>>): Pair<Int, Int> {
    val total = catalogs.sumOf { it.families.size }
    val shown = catalogs.sumOf { c -> c.families.count { !ModelVisibility.hidden(it, hidden[c.control.id].orEmpty()) } }
    return shown to total
  }

  /** One group per control, the model control split into official / custom / other where the adapter classified its
   *  sources */
  fun groups(catalogs: List<Catalog>, query: String): List<Group> = catalogs.flatMap { c ->
    val families = ModelVisibility.filter(c.families, query)
    if (families.isEmpty()) return@flatMap emptyList()
    val title = when (c.control.category) {
      "model" -> t("settings.models.selection")
      "collaboration_mode" -> t("settings.models.collaboration")
      else -> c.control.name
    }
    val split = c.control.category == "model" && families.any { it.sourceKind != null }
    // (key, title, kind): a null kind takes every row; "" takes the unclassified ones
    val kinds: List<Triple<String, String, String?>> = if (split) {
      listOf(Triple("official", t("settings.models.official"), "official"), Triple("custom", t("settings.models.custom"), "custom"), Triple("other", title, ""))
    } else listOf(Triple(c.control.id, title, null))
    kinds.mapNotNull { (key, groupTitle, kind) ->
      val rows = families.filter { f -> kind == null || (f.sourceKind ?: "") == kind }
      if (rows.isEmpty()) null
      else Group(c.control, "${c.control.id}:$key", groupTitle, rows, rows.mapNotNull { it.source }.distinct())
    }
  }

  /** What a family spans, its row's second line: its effort levels, Fast, 1M; a single variant names itself */
  fun summary(f: ModelFamily): String? {
    if (f.variants.size == 1) {
      val v = f.variants.first()
      if (v.name == f.name) return null
      if (v.lead != null) return ModelCatalog.fusionLabel(v)
      val parts = listOf(v.effort, if (v.fast) "Fast" else "", if (v.long) "1M" else "").filter { it.isNotEmpty() }
      return if (parts.isEmpty()) t("composer.standard") else parts.joinToString(" ")
    }
    val fusion = f.fusion?.let { t("composer.fusionSummary", "leads" to it.leads.size, "sidekicks" to it.sidekicks.size) }
    val levels = f.efforts.filter { it.isNotEmpty() }
    val parts = listOfNotNull(
      fusion,
      if (levels.isEmpty()) null else "${t("settings.providers.model.thinking")} ${levels.joinToString(" / ")}",
      if (f.hasFast) "Fast" else null,
      if (f.hasLong) t("settings.providers.model.ctx", "n" to "1M") else null,
    )
    return if (parts.isEmpty()) null else parts.joinToString(t("common.metaSep"))
  }
}
