package cc.hotic.acpira.logic

import cc.hotic.acpira.model.ConfigControl
import cc.hotic.acpira.model.SessionControls
import cc.hotic.acpira.model.SessionOption
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.JsonPrimitive

// apps/ios Logic/ModelCatalog.swift (shared/models.ts + shared/composerControls.ts): the model list rebuilt from flat
// option names (Devin flattens model × effort × Fast × 1M into one select), the native reasoning / Fast controls. The
// wire ids stay untouched; only the presentation is derived.

/**
 * One option of a family: the wire id plus the dimensions parsed from its name. Fusion only: [lead] is the lead family
 * (effort / fast describe it) and [sidekick] the sidekick's label ("SWE-2 High")
 */
data class ModelVariant(
  val id: String,
  val name: String,
  val effort: String,
  val fast: Boolean,
  val long: Boolean,
  val lead: String? = null,
  val sidekick: String? = null,
)

/** A Fusion family's lead families and sidekick labels, in first-seen order */
data class Fusion(val leads: List<String>, val sidekicks: List<String>)

/** A model family: one row of the model panel, its variants the params underneath */
data class ModelFamily(
  val key: String,
  val name: String,
  val brand: String?,
  val variants: List<ModelVariant>,
  /** Effort labels by strength, deduplicated; "" is the level-less (Standard) variant */
  val efforts: List<String>,
  val hasFast: Boolean,
  val hasLong: Boolean = false,
  /** The provider the family comes from, when the options name one */
  val source: String? = null,
  /** `official` / `custom`, from the agent adapter's source metadata */
  val sourceKind: String? = null,
  /** Fusion only: its pairs' leads and sidekicks */
  val fusion: Fusion? = null,
)

object ModelCatalog {
  const val fusion = "Fusion"
  private val fusionName = Regex("""^Fusion\s*\((.+?)\s+\+\s+(.+)\)$""", RegexOption.IGNORE_CASE)
  private val levels = mapOf(
    "none" to "None", "minimal" to "Minimal", "low" to "Low", "medium" to "Medium", "high" to "High",
    "xhigh" to "XHigh", "x-high" to "XHigh", "max" to "Max",
  )
  private val effortOrder = listOf("", "Thinking", "None", "Minimal", "Low", "Medium", "High", "XHigh", "Max")
  fun rank(effort: String): Int = effortOrder.indexOf(effort).takeIf { it >= 0 } ?: effortOrder.size

  data class Parsed(
    val family: String,
    val effort: String,
    val fast: Boolean,
    val long: Boolean,
    val lead: String? = null,
    val sidekick: String? = null,
  )

  /**
   * "Fusion (GPT-6 Astra High Thinking Fast + GPT-5.6 Luna High Thinking Fast)": lead GPT-6 Astra at High, sidekick
   * "GPT-5.6 Luna High", Fast from either side (one pair-level switch on the wire)
   */
  fun parseFusion(name: String): Parsed? {
    val m = fusionName.find(name.trim()) ?: return null
    val lead = parse(m.groupValues[1])
    val sk = parse(m.groupValues[2])
    return Parsed(
      fusion, lead.effort, lead.fast || sk.fast, lead.long, lead = lead.family,
      sidekick = listOf(sk.family, sk.effort).filter { it.isNotEmpty() }.joinToString(" "),
    )
  }

  private fun effortTokens(tokens: List<String>): Pair<List<String>, String> {
    val w = tokens.toMutableList()
    var thinking = false
    var effort = ""
    if (w.lastOrNull()?.lowercase() == "thinking") { thinking = true; w.removeAt(w.lastIndex) }
    val last = w.lastOrNull()?.lowercase().orEmpty()
    if (last == "high" && w.size >= 2 && w[w.size - 2].lowercase() == "extra") {
      effort = "XHigh"; w.removeAt(w.lastIndex); w.removeAt(w.lastIndex)
    } else if (levels[last] != null) {
      effort = levels.getValue(last); w.removeAt(w.lastIndex)
    } else if (last == "no" && thinking) {
      effort = "None"; thinking = false; w.removeAt(w.lastIndex)
    }
    if (effort.isEmpty() && thinking) effort = "Thinking"
    return w to effort
  }

  /** `<family> [effort] [Thinking] [Fast] [1M]`, the effort also in parentheses */
  fun parse(name: String): Parsed {
    parseFusion(name)?.let { return it }
    val t = name.trim().split(Regex("\\s+")).filter { it.isNotEmpty() }.toMutableList()
    var fast = false
    var long = false
    var effort = ""
    // Fast and 1M may both trail, in either order
    fun stripTrailingFlags() {
      while (t.size > 1) {
        when (t.last().lowercase()) {
          "fast" -> { fast = true; t.removeAt(t.lastIndex) }
          "1m" -> { long = true; t.removeAt(t.lastIndex) }
          else -> return
        }
      }
    }
    stripTrailingFlags()
    if (t.size > 1 && t.last().endsWith(")")) {
      val open = (t.size - 1 downTo 1).firstOrNull { t[it].startsWith("(") }
      if (open != null) {
        val group = t.subList(open, t.size).toMutableList()
        group[0] = group[0].drop(1)
        group[group.lastIndex] = group.last().dropLast(1)
        val inner = group.filter { it.isNotEmpty() }
        if (inner.isNotEmpty() && inner.all { !it.contains('(') && !it.contains(')') }) {
          val (rest, e) = effortTokens(inner)
          if (e.isNotEmpty() && rest.isEmpty()) {
            effort = e
            while (t.size > open) t.removeAt(t.lastIndex)
            stripTrailingFlags()
          }
        }
      }
    }
    if (effort.isEmpty() && t.size > 1) {
      val (rest, e) = effortTokens(t.drop(1))
      if (e.isNotEmpty()) {
        effort = e
        val head = t[0]
        t.clear(); t.add(head); t.addAll(rest)
        stripTrailingFlags()
      }
    }
    return Parsed(t.joinToString(" "), effort, fast, long)
  }

  /** Options grouped into families in first-seen order, variants sorted by effort, Fast, 1M */
  fun group(options: List<SessionOption>, brandOf: (SessionOption) -> String?): List<ModelFamily> {
    data class Tuple(
      val ns: String?, val family: String, val effort: String, val fast: Boolean, val long: Boolean, val lead: String?, val sidekick: String?,
    )
    // With no provider metadata, equal parameter tuples are ambiguous, not interchangeable
    val tuples = HashSet<Tuple>()
    val ambiguous = HashSet<Pair<String?, String>>()
    for (o in options) {
      val p = parse(o.name)
      val ns = o.source?.id ?: o.group?.id
      val tuple = Tuple(ns, p.family, p.effort, p.fast, p.long, p.lead, p.sidekick)
      if (!tuples.add(tuple)) ambiguous.add(ns to p.family)
    }
    val order = mutableListOf<String>()
    val names = HashMap<String, String>()
    val brands = HashMap<String, String?>()
    val sources = HashMap<String, Pair<String?, String?>>()
    val variants = HashMap<String, MutableList<ModelVariant>>()
    val leads = HashMap<String, MutableList<String>>()
    val sidekicks = HashMap<String, MutableList<String>>()
    for (o in options) {
      val p = parse(o.name)
      val ns = o.source?.id ?: o.group?.id
      val separate = (ns to p.family) in ambiguous
      // The TS keys verbatim (JSON arrays): settings `hiddenOptions` stores them, shared with the desktop
      val key = if (separate) jsonKey(listOf(ns, p.family, o.id)) else ns?.let { jsonKey(listOf(it, p.family)) } ?: p.family
      if (key !in variants) {
        order.add(key)
        names[key] = p.family
        sources[key] = (o.source?.name ?: o.group?.name ?: if (separate) o.id else null) to o.source?.kind
        variants[key] = mutableListOf()
      }
      // Fusion pairs are Devin's own routing, like Adaptive
      if (brands[key] == null) brands[key] = if (p.lead != null) "devin" else brandOf(o)
      p.lead?.let { lead ->
        val ls = leads.getOrPut(key) { mutableListOf() }
        if (lead !in ls) ls.add(lead)
        val ss = sidekicks.getOrPut(key) { mutableListOf() }
        p.sidekick?.let { if (it !in ss) ss.add(it) }
      }
      variants.getValue(key).add(ModelVariant(o.id, o.name, p.effort, p.fast, p.long, p.lead, if (p.lead != null) p.sidekick else null))
    }
    return order.map { key ->
      val fusion = leads[key]?.let { Fusion(it, sidekicks[key].orEmpty()) }
      // Pairs sort by lead, effort, sidekick, then Fast and 1M
      val leadRank = { v: ModelVariant -> fusion?.leads?.indexOf(v.lead.orEmpty())?.coerceAtLeast(0) ?: 0 }
      val skRank = { v: ModelVariant -> fusion?.sidekicks?.indexOf(v.sidekick.orEmpty())?.coerceAtLeast(0) ?: 0 }
      val vs = variants.getValue(key).sortedWith(
        compareBy<ModelVariant> { leadRank(it) }.thenBy { rank(it.effort) }.thenBy { skRank(it) }.thenBy { it.fast }.thenBy { it.long },
      )
      ModelFamily(
        key = key,
        name = names.getValue(key),
        brand = brands[key],
        variants = vs,
        efforts = vs.map { it.effort }.distinct().sortedBy { rank(it) },
        hasFast = vs.any { it.fast },
        hasLong = vs.any { it.long },
        source = sources[key]?.first,
        sourceKind = sources[key]?.second,
        fusion = fusion,
      )
    }
  }

  /** `JSON.stringify` of a short array of strings and nulls, as models.ts builds a family's key */
  fun jsonKey(parts: List<String?>): String = JsonArray(parts.map { it?.let(::JsonPrimitive) ?: JsonNull }).toString()

  /** Exact effort / Fast / 1M first, then dropping 1M, Fast, both; finally any variant at that effort */
  fun findVariant(f: ModelFamily, effort: String, fast: Boolean, long: Boolean): ModelVariant? {
    fun hit(fa: Boolean, lo: Boolean) = f.variants.firstOrNull { it.effort == effort && it.fast == fa && it.long == lo }
    return hit(fast, long) ?: hit(fast, false) ?: hit(false, long) ?: hit(false, false) ?: f.variants.firstOrNull { it.effort == effort }
  }

  /** The offered Fusion pair nearest to lead × effort × sidekick × fast */
  fun findFusionVariant(f: ModelFamily, lead: String, effort: String, sidekick: String?, fast: Boolean): ModelVariant? {
    val ofLead = f.variants.filter { it.lead == lead }
    val pool = ofLead.ifEmpty { f.variants }
    fun hit(e: String, sk: String?, fa: Boolean) = pool.firstOrNull { it.effort == e && (sk == null || it.sidekick == sk) && it.fast == fa }
    fun at(e: String) = hit(e, sidekick, fast) ?: hit(e, sidekick, false) ?: hit(e, null, fast) ?: hit(e, null, false)
    // The other levels, nearest first, the stronger of two equally near
    val nearest = pool.map { it.effort }.filter { it != effort }.distinct().sortedWith(
      compareBy<String> { kotlin.math.abs(rank(it) - rank(effort)) }.thenByDescending { rank(it) },
    )
    return at(effort) ?: nearest.firstNotNullOfOrNull { at(it) } ?: pool.firstOrNull()
  }

  /** "GPT-6 Astra High + SWE-2 High Fast" */
  fun fusionLabel(v: ModelVariant): String {
    val lead = listOf(v.lead.orEmpty(), v.effort).filter { it.isNotEmpty() }.joinToString(" ")
    val sk = listOf(v.sidekick.orEmpty(), if (v.fast) "Fast" else "").filter { it.isNotEmpty() }.joinToString(" ")
    return listOf(lead, sk).filter { it.isNotEmpty() }.joinToString(" + ")
  }

  /** The levels the pair's lead is offered at */
  fun fusionEfforts(f: ModelFamily, v: ModelVariant): List<String> {
    val lead = v.lead ?: f.fusion?.leads?.firstOrNull().orEmpty()
    return f.efforts.filter { e -> f.variants.any { it.lead == lead && it.effort == e } }
  }

  /** A Standard / Thinking pair is a Thinking switch rather than two levels */
  fun thinkingOnly(f: ModelFamily) = f.efforts.size == 2 && "" in f.efforts && "Thinking" in f.efforts
  fun variantScale(f: ModelFamily) = f.efforts.size > 1 && !thinkingOnly(f)
}

/** composerControls.ts: reasoning levels, Fast and the split of an agent's controls, whatever the wire shape */
object ComposerControls {
  private val levels = listOf("None", "Minimal", "Low", "Medium", "High", "XHigh", "Max", "Ultra", "Thinking")
  const val permissionCategory = "_permission_mode"
  /** The built-in agent's sidekick select (custom category), its level and its Fast, the model chip's last bean */
  const val sidekickCategory = "_sidekick"
  const val sidekickEffortCategory = "_sidekick_effort"
  const val sidekickFastCategory = "_sidekick_fast"
  /** The sidekick's value for none */
  const val noSidekick = "none"

  private fun compact(value: String) =
    value.lowercase().replace(Regex("reasoning|thinking|effort|level"), "").replace(Regex("[\\s_-]"), "")

  /** Presentation only: Low / Medium / High / XHigh / Max / Ultra; the wire ids stay */
  fun effortLabel(option: SessionOption): String {
    fun canonical(value: String): String? {
      val key = compact(value)
      return if (key == "extrahigh") "XHigh" else levels.firstOrNull { it.lowercase() == key }
    }
    return canonical(option.id) ?: canonical(option.name) ?: option.name
  }

  fun isReasoning(c: ConfigControl) =
    c.category == "thought_level" || (c.category == null && c.id.matches(Regex("reasoning_effort|thought_level|thinking|thinking_level")))

  /** Kimi folds thinking on / off into the effort select */
  private fun toggleSide(o: SessionOption): String? {
    val key = compact(o.id).ifEmpty { compact(o.name) }
    return if (key == "on" || key == "off") key else null
  }

  /** Kimi keeps the previous model's leftover value: drop the stranger, keep the native set */
  private fun nativeThoughtOptions(options: List<SessionOption>): List<SessionOption> {
    val toggles = options.filter { toggleSide(it) != null }
    val efforts = options.filter { toggleSide(it) == null }
    val ons = toggles.filter { toggleSide(it) == "on" }
    if (toggles.any { toggleSide(it) == "off" }) return options
    if (ons.isNotEmpty() && efforts.size > 1) return efforts
    if (ons.isNotEmpty() && efforts.size == 1) return ons
    return options
  }

  data class Reasoning(val efforts: List<SessionOption>, val offId: String?, val onId: String?, val value: String?, val off: Boolean)

  fun presentReasoning(c: ConfigControl): Reasoning {
    val native = nativeThoughtOptions(c.options)
    val efforts = native.filter { toggleSide(it) == null }
      .map { it.copy(name = effortLabel(it)) }
      .withIndex().sortedWith(compareBy({ levels.indexOf(it.value.name).let { r -> if (r < 0) levels.size else r } }, { it.index }))
      .map { it.value }
    val offId = native.firstOrNull { toggleSide(it) == "off" }?.id
    val onId = native.firstOrNull { toggleSide(it) == "on" }?.id
    val value = if (native.any { it.id == c.value }) c.value else {
      val plain = native.filter { toggleSide(it) == null }
      plain.firstOrNull { it.id == "high" }?.id ?: plain.firstOrNull()?.id ?: onId ?: native.firstOrNull()?.id
    }
    return Reasoning(efforts, offId, onId, value, offId != null && value == offId)
  }

  fun reasoningVisible(c: ConfigControl): Boolean = presentReasoning(c).let { it.efforts.size > 1 || it.offId != null }

  fun isFast(c: ConfigControl): Boolean {
    if (c.type == "boolean") {
      return Regex("(^|[\\s_-])fast($|[\\s_-])", RegexOption.IGNORE_CASE).containsMatchIn(c.id) ||
        c.name.matches(Regex("fast( mode)?", RegexOption.IGNORE_CASE))
    }
    return c.options.size == 2 && listOf("standard", "fast").all { id -> c.options.any { it.id == id } }
  }

  fun fastOn(c: ConfigControl) = c.value == (if (c.type == "boolean") "true" else "fast")
  fun fastValue(c: ConfigControl, on: Boolean) = if (c.type == "boolean") on.toString() else if (on) "fast" else "standard"

  /** The toolbar's split; the controls that fit nowhere have no place on the phone */
  data class Split(
    val models: List<ConfigControl>, val reasoning: List<ConfigControl>, val modelConfig: List<ConfigControl>,
    val collaboration: List<ConfigControl>, val permission: List<ConfigControl>,
    val sidekick: List<ConfigControl> = emptyList(), val sidekickEffort: List<ConfigControl> = emptyList(),
    val sidekickFast: List<ConfigControl> = emptyList(),
  )

  fun split(options: List<ConfigControl>, brandOf: (SessionOption) -> String?): Split {
    val models = mutableListOf<ConfigControl>()
    val reasoning = mutableListOf<ConfigControl>()
    val modelConfig = mutableListOf<ConfigControl>()
    val collaboration = mutableListOf<ConfigControl>()
    val permission = mutableListOf<ConfigControl>()
    val sidekick = mutableListOf<ConfigControl>()
    val sidekickEffort = mutableListOf<ConfigControl>()
    val sidekickFast = mutableListOf<ConfigControl>()
    for (c in options) {
      when {
        isReasoning(c) -> reasoning.add(c)
        c.category == "collaboration_mode" && c.type != "boolean" -> collaboration.add(c)
        c.category == permissionCategory && c.type != "boolean" -> permission.add(c)
        c.category == sidekickCategory && c.type != "boolean" -> sidekick.add(c)
        c.category == sidekickEffortCategory && c.type != "boolean" -> sidekickEffort.add(c)
        c.category == sidekickFastCategory -> sidekickFast.add(c)
        c.category == "model_config" -> modelConfig.add(c)
        c.type != "boolean" && (c.category == "model" ||
          (c.category == null && (c.id == "model" || ModelCatalog.group(c.options, brandOf).size < c.options.size))) -> models.add(c)
      }
    }
    return Split(models, reasoning, modelConfig, collaboration, permission, sidekick, sidekickEffort, sidekickFast)
  }
}

/** One switch whatever its source (an embedded Fast variant, a native control, Thinking): its state and how to flip it */
class Toggle(val on: Boolean, val disabled: Boolean, val set: (Boolean) -> Unit)

/** The reasoning levels on offer (the family's own scale, else the agent's thought_level), Fast beside them, and the
 *  Thinking switch where thinking can be turned off */
class Effort(val levels: List<Pair<String, String>>, val value: String, val pick: (String) -> Unit, val fast: Toggle?, val thinking: Toggle?)

/**
 * What the composer's model chip offers, read from the session's controls (Goo.swift ModelChoices): the model
 * families, the reasoning levels with Fast, the modes, the working mode, the approval level, a Fusion pair's lead and
 * sidekick, and the built-in agent's sidekick. [standard] names the level-less variant (composer.standard)
 */
class ModelChoices(
  controls: SessionControls,
  brandOf: (SessionOption) -> String?,
  standard: String,
  private val onSetConfig: (String, String) -> Unit,
) {
  val families: List<ModelFamily>
  val cur: ModelFamily?
  val curVar: ModelVariant?
  val modelId: String?
  val effort: Effort?
  val mode: SessionOption?
  val modes: List<SessionOption>
  val collaboration: ConfigControl?
  val access: ConfigControl?
  /** The built-in agent's sidekick select, its level and its Fast (the last bean opens them as beans of their own) */
  val sidekick: ConfigControl?
  val sidekickEffort: ConfigControl?
  val sidekickFast: Toggle?
  private val nativeFast: Toggle?

  init {
    val split = ComposerControls.split(controls.options, brandOf)
    val model = split.models.firstOrNull()
    families = model?.let { ModelCatalog.group(it.options, brandOf) }.orEmpty()
    cur = families.firstOrNull { f -> f.variants.any { it.id == model?.value } }
    curVar = cur?.variants?.firstOrNull { it.id == model?.value }
    modelId = model?.id
    nativeFast = split.modelConfig.firstOrNull { ComposerControls.isFast(it) }?.let { nativeToggle(it) }
    effort = effortOf(split, standard)
    mode = controls.modes.firstOrNull { it.id == controls.modeId } ?: controls.modes.firstOrNull()
    modes = if (controls.modes.size > 1) controls.modes else emptyList()
    collaboration = split.collaboration.firstOrNull { it.options.size > 1 }
    access = split.permission.firstOrNull { it.options.size > 1 }
    sidekick = split.sidekick.firstOrNull()
    sidekickEffort = split.sidekickEffort.firstOrNull()
    sidekickFast = split.sidekickFast.firstOrNull()?.let { nativeToggle(it) }
  }

  /** Devin's Standard / Fast select or an adapter boolean as one switch */
  private fun nativeToggle(c: ConfigControl): Toggle {
    val on = ComposerControls.fastOn(c)
    val next = ComposerControls.fastValue(c, !on)
    return Toggle(on, c.options.isNotEmpty() && c.options.none { it.id == next }) { onSetConfig(c.id, next) }
  }

  private fun effortOf(split: ComposerControls.Split, standard: String): Effort? {
    val f = cur
    val v = curVar
    val id = modelId
    var fusionFast: Toggle? = null
    if (f?.fusion != null && v != null && id != null) {
      // A pair's level and Fast are the pair's own: each pick writes the offered pair nearest to it
      val pick = { e: String, fast: Boolean ->
        ModelCatalog.findFusionVariant(f, v.lead.orEmpty(), e, v.sidekick, fast)?.let { onSetConfig(id, it.id) }; Unit
      }
      if (f.hasFast) {
        // No Fast for this exact pair: never jump to another sidekick just to offer it
        val can = f.variants.any { it.lead == v.lead && it.effort == v.effort && it.sidekick == v.sidekick && it.fast != v.fast }
        fusionFast = Toggle(v.fast, !can) { pick(v.effort, it) }
      }
      val levels = ModelCatalog.fusionEfforts(f, v)
      if (levels.size > 1) return Effort(levels.map { it to it.ifEmpty { standard } }, v.effort, { pick(it, v.fast) }, fusionFast ?: nativeFast, null)
    }
    if (f != null && v != null && id != null && f.fusion == null && (ModelCatalog.variantScale(f) || ModelCatalog.thinkingOnly(f))) {
      val fast = if (f.hasFast) {
        fun at(long: Boolean) = f.variants.firstOrNull { it.effort == v.effort && it.fast == !v.fast && it.long == long }
        val hit = at(false) ?: at(v.long)
        Toggle(v.fast, hit == null) { hit?.let { onSetConfig(id, it.id) } }
      } else nativeFast
      val pick = { e: String -> ModelCatalog.findVariant(f, e, v.fast, false)?.let { onSetConfig(id, it.id) }; Unit }
      if (ModelCatalog.thinkingOnly(f)) {
        val on = v.effort == "Thinking"
        val thinking = Toggle(on, ModelCatalog.findVariant(f, if (on) "" else "Thinking", v.fast, false) == null) {
          pick(if (it) "Thinking" else "")
        }
        return Effort(emptyList(), v.effort, pick, fast, thinking)
      }
      return Effort(f.efforts.map { it to it.ifEmpty { standard } }, v.effort, pick, fast, null)
    }
    val c = split.reasoning.firstOrNull { ComposerControls.reasoningVisible(it) }
    if (c != null) {
      val p = ComposerControls.presentReasoning(c)
      // Turned back on at High, else the first level, else the agent's own "on"
      val turnOn = p.efforts.firstOrNull { it.id == "high" }?.id ?: p.efforts.firstOrNull()?.id ?: p.onId
      val thinking = p.offId?.let { off -> Toggle(!p.off, false) { on -> if (!on) onSetConfig(c.id, off) else turnOn?.let { onSetConfig(c.id, it) } } }
      if (p.efforts.size <= 1 && thinking == null) return null
      return Effort(p.efforts.map { it.id to it.name }, if (p.off) "" else p.value.orEmpty(), { onSetConfig(c.id, it) }, fusionFast ?: nativeFast, thinking)
    }
    // A fixed lead level can still have pair-level Fast, without a reasoning bean to own it
    return fusionFast?.let { Effort(emptyList(), v?.effort.orEmpty(), {}, it, null) }
  }

  val hasModel get() = cur != null && modelId != null
  val hasEffort get() = effort?.let { it.levels.size > 1 || it.thinking != null } ?: false

  /** Fast, when the model in use can switch it at the level in force, or while it is on */
  val fast: Toggle? get() = (effort?.fast ?: nativeFast)?.takeUnless { it.disabled && !it.on }

  /** The Fusion family's leads and sidekicks while a pair is in use */
  val fusion: Fusion? get() = cur?.fusion

  /** Keeps the lead's effort and the other side where possible; the shared resolver picks the nearest offered pair */
  fun pickFusion(lead: String? = null, effort: String? = null, sidekick: String? = null, fast: Boolean? = null) {
    val f = cur ?: return
    val v = curVar ?: return
    val id = modelId ?: return
    if (f.fusion == null) return
    val hit = ModelCatalog.findFusionVariant(f, lead ?: v.lead.orEmpty(), effort ?: v.effort, sidekick ?: v.sidekick, fast ?: v.fast) ?: return
    onSetConfig(id, hit.id)
  }

  /** Whether a pair with this sidekick exists for the lead at its level (the rest stay listed, disabled) */
  fun offersSidekick(name: String): Boolean {
    val f = cur ?: return false
    val v = curVar ?: return false
    return f.variants.any { it.lead == v.lead && it.effort == v.effort && it.sidekick == name }
  }

  /** ModelPanel.pickFamily: into Fusion the model in use becomes the lead; out of it the lead's effort carries over */
  fun pickFamily(f: ModelFamily) {
    val id = modelId ?: return
    val v = curVar
    val next = if (f.fusion != null) {
      ModelCatalog.findFusionVariant(f, v?.lead ?: cur?.name.orEmpty(), v?.effort.orEmpty(), v?.sidekick, v?.fast ?: false)
    } else v?.let { ModelCatalog.findVariant(f, it.effort, it.fast, false) }
    (next ?: f.variants.firstOrNull())?.let { onSetConfig(id, it.id) }
  }

  /** The sidekick picked, null while the select reads None */
  val sidekickPicked: SessionOption?
    get() = sidekick?.let { sk ->
      val current = sk.value ?: ComposerControls.noSidekick
      sk.options.firstOrNull { it.id == current && it.id != ComposerControls.noSidekick }
    }
}
