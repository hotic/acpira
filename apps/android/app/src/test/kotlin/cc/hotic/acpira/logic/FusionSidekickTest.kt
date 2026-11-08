package cc.hotic.acpira.logic

import cc.hotic.acpira.model.ConfigControl
import cc.hotic.acpira.model.ModelShapes
import cc.hotic.acpira.model.SessionControls
import cc.hotic.acpira.model.SessionOption
import cc.hotic.acpira.model.WireJson
import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue

/**
 * Fusion pairs and the built-in agent's sidekick in logic/ModelCatalog.kt and logic/TurnEdit.kt. Mirrors the apps/ios
 * LogicTests (ModelCatalogTests' fusion cases), ComposerTests (fusion choices, the sidekick split) and TurnActionTests
 * (a sidekick picked in a draft)
 */
class FusionSidekickTest {
  private fun options(vararg pairs: Pair<String, String>) = pairs.map { SessionOption(it.first, it.second) }

  /** Real Fusion samples from Devin: Fast is one pair-level switch, marked on whichever side supports it */
  private val devinFusion = options(
    "fusion-claude-fable-5-1-high-sidekick-swe-2-medium" to "Fusion (Claude Fable 5.1 High + SWE-2 Medium)",
    "fusion-claude-fable-5-1-high-sidekick-gpt-5-6-luna-high" to "Fusion (Claude Fable 5.1 High + GPT-5.6 Luna High Thinking)",
    "fusion-claude-fable-5-1-high-fast-sidekick-gpt-5-6-luna-high-priority" to "Fusion (Claude Fable 5.1 High + GPT-5.6 Luna High Thinking Fast)",
    "fusion-claude-fable-5-1-max-sidekick-swe-2-high" to "Fusion (Claude Fable 5.1 Max + SWE-2 High)",
    "fusion-gpt-6-astra-high-sidekick-swe-2-medium" to "Fusion (GPT-6 Astra High Thinking + SWE-2 Medium)",
    "fusion-gpt-6-astra-high-fast-sidekick-swe-2-medium" to "Fusion (GPT-6 Astra High Thinking Fast + SWE-2 Medium)",
    "fusion-gpt-6-astra-high-sidekick-gpt-5-6-luna-high" to "Fusion (GPT-6 Astra High Thinking + GPT-5.6 Luna High Thinking)",
    "fusion-gpt-6-astra-high-fast-sidekick-gpt-5-6-luna-high-priority" to "Fusion (GPT-6 Astra High Thinking Fast + GPT-5.6 Luna High Thinking Fast)",
    "fusion-gpt-6-astra-low-sidekick-swe-2-medium" to "Fusion (GPT-6 Astra Low Thinking + SWE-2 Medium)",
    "fusion-claude-opus-5-medium-sidekick-glm-5-2-high" to "Fusion (Claude Opus 5 Medium + GLM-5.2 High)",
  )

  private val composerFusion = options(
    "plain" to "GPT-6 Astra High Thinking",
    "gpt-low-swe" to "Fusion (GPT-6 Astra Low Thinking + SWE-2 Medium)",
    "gpt-high-swe" to "Fusion (GPT-6 Astra High Thinking + SWE-2 Medium)",
    "gpt-high-luna" to "Fusion (GPT-6 Astra High Thinking + GPT-5.6 Luna High Thinking)",
    "gpt-high-luna-fast" to "Fusion (GPT-6 Astra High Thinking Fast + GPT-5.6 Luna High Thinking Fast)",
    "opus-medium-glm" to "Fusion (Claude Opus 5 Medium + GLM-5.2 High)",
  )

  private fun choices(controls: List<ConfigControl>, set: (String, String) -> Unit = { _, _ -> }) =
    ModelChoices(SessionControls(options = controls), { null }, "Standard", set)

  private fun fusionChoices(value: String, set: (String, String) -> Unit = { _, _ -> }) =
    choices(listOf(ConfigControl("model", "Model", "model", options = composerFusion, value = value)), set)

  private fun control(json: String) = WireJson.decodeFromString(ConfigControl.serializer(), json)

  @Test fun fusionPairsCollapseIntoOneFamily() {
    val f = ModelCatalog.group(devinFusion) { null }
    assertEquals(1, f.size)
    assertTrue(f[0].name == "Fusion" && f[0].brand == "devin" && f[0].hasFast)
    assertEquals(listOf("Low", "Medium", "High", "Max"), f[0].efforts)
    assertEquals(listOf("Claude Fable 5.1", "GPT-6 Astra", "Claude Opus 5"), f[0].fusion?.leads)
    assertEquals(listOf("SWE-2 Medium", "GPT-5.6 Luna High", "SWE-2 High", "GLM-5.2 High"), f[0].fusion?.sidekicks)
    assertEquals(devinFusion.take(4).map { it.id }, f[0].variants.take(4).map { it.id })
  }

  @Test fun findFusionVariantFallsBackStepByStep() {
    val f = ModelCatalog.group(devinFusion) { null }[0]
    fun find(lead: String, effort: String, sidekick: String?, fast: Boolean) = ModelCatalog.findFusionVariant(f, lead, effort, sidekick, fast)?.id
    assertEquals("fusion-gpt-6-astra-high-fast-sidekick-gpt-5-6-luna-high-priority", find("GPT-6 Astra", "High", "GPT-5.6 Luna High", true))
    assertEquals("fusion-claude-fable-5-1-high-sidekick-swe-2-medium", find("Claude Fable 5.1", "High", "SWE-2 Medium", true))
    assertEquals("fusion-gpt-6-astra-high-sidekick-swe-2-medium", find("GPT-6 Astra", "High", "SWE-2 High", false))
    assertEquals("fusion-claude-opus-5-medium-sidekick-glm-5-2-high", find("Claude Opus 5", "High", "SWE-2 Medium", false))
    assertEquals("fusion-claude-fable-5-1-max-sidekick-swe-2-high", find("SWE-2", "Max", null, false))
    val pair = f.variants.first { it.id == "fusion-claude-fable-5-1-high-fast-sidekick-gpt-5-6-luna-high-priority" }
    assertEquals("Claude Fable 5.1 High + GPT-5.6 Luna High Fast", ModelCatalog.fusionLabel(pair))
  }

  @Test fun fusionPicksKeepThePairAndResolveOnlyOfferedCombinations() {
    val sets = mutableListOf<String>()
    val c = fusionChoices("gpt-high-luna") { id, v -> sets += "$id=$v" }
    assertEquals(listOf("GPT-6 Astra", "Claude Opus 5"), c.fusion?.leads)
    assertEquals(listOf("SWE-2 Medium", "GPT-5.6 Luna High", "GLM-5.2 High"), c.fusion?.sidekicks)
    assertTrue(c.offersSidekick("SWE-2 Medium") && !c.offersSidekick("GLM-5.2 High"))
    c.pickFusion(sidekick = "SWE-2 Medium")
    c.pickFusion(lead = "Claude Opus 5")
    assertEquals(listOf("model=gpt-high-swe", "model=opus-medium-glm"), sets)
    assertFalse(fusionChoices("gpt-low-swe").offersSidekick("GPT-5.6 Luna High"))
    assertNull(fusionChoices("plain").fusion)
  }

  @Test fun fusionEmbeddedEffortAndFastChangeTheModelNotTheNativeControls() {
    val sets = mutableListOf<String>()
    val c = fusionChoices("gpt-high-luna") { id, v -> sets += "$id=$v" }
    assertTrue(c.hasEffort)
    assertEquals(listOf("Low", "High"), c.effort?.levels?.map { it.first })
    assertEquals("High", c.effort?.value)
    c.fast?.set(true)
    c.effort?.pick("Low")
    assertEquals(listOf("model=gpt-high-luna-fast", "model=gpt-low-swe"), sets)
    // No Fast for this exact pair: never jump to a different sidekick just to enable it
    assertNull(fusionChoices("gpt-high-swe").fast)
    assertFalse(fusionChoices("opus-medium-glm").hasEffort)
  }

  @Test fun modernFusionKeepsNativeReasoningAndSpeedIndependentOfThePair() {
    val sets = mutableListOf<String>()
    val model = ConfigControl("model", "Model", "model", options = composerFusion.filter { it.id in setOf("gpt-high-swe", "gpt-high-luna") }, value = "gpt-high-luna")
    val thought = ConfigControl("thought_level", "Thinking", "thought_level", options = options("low" to "Low", "high" to "High"), value = "high")
    val speed = ConfigControl("speed", "Speed", "model_config", options = options("standard" to "Standard", "fast" to "Fast"), value = "standard")
    val c = choices(listOf(model, thought, speed)) { id, v -> sets += "$id=$v" }
    assertEquals(listOf("low", "high"), c.effort?.levels?.map { it.first })
    c.effort?.pick("low")
    c.fast?.set(true)
    c.pickFusion(sidekick = "SWE-2 Medium")
    assertEquals(listOf("thought_level=low", "speed=fast", "model=gpt-high-swe"), sets)
    val fastOnly = choices(listOf(model, speed))
    assertTrue(!fastOnly.hasEffort && fastOnly.fast?.disabled == false)
  }

  @Test fun switchingIntoFusionKeepsTheLeadAndLeavingKeepsItsEffort() {
    val sets = mutableListOf<String>()
    val plain = fusionChoices("plain") { id, v -> sets += "$id=$v" }
    plain.pickFamily(plain.families.first { it.fusion != null })
    val fusion = fusionChoices("gpt-high-luna") { id, v -> sets += "$id=$v" }
    fusion.pickFamily(fusion.families.first { it.fusion == null })
    assertEquals(listOf("model=gpt-high-swe", "model=plain"), sets)
  }

  @Test fun theSidekickHasItsOwnPlace() {
    val sidekick = control("""{"id":"sidekick","name":"Sidekick","category":"_sidekick","value":"none","options":[{"id":"none","name":"None"},{"id":"gpt-6","name":"GPT-6"}]}""")
    val level = control("""{"id":"sidekick_effort","name":"Sidekick effort","category":"_sidekick_effort","value":"high","options":[{"id":"low","name":"low"},{"id":"high","name":"high"}]}""")
    val split = ComposerControls.split(listOf(sidekick, level)) { null }
    assertEquals(listOf("sidekick"), split.sidekick.map { it.id })
    assertEquals(listOf("sidekick_effort"), split.sidekickEffort.map { it.id })
    assertTrue(split.reasoning.isEmpty() && split.models.isEmpty())
    val c = choices(listOf(sidekick, level))
    assertEquals(ComposerControls.noSidekick, c.sidekick?.value)
    assertEquals("sidekick_effort", c.sidekickEffort?.id)
    assertNull(c.sidekickPicked)
    // The sidekick's level is never the lead's
    assertNull(c.effort)
  }

  @Test fun aSidekickPickedInADraftBringsItsModelsLevels() {
    val controls = SessionControls(
      options = listOf(
        control("""{"id":"model","name":"Model","category":"model","value":"a/sol","options":[{"id":"a/sol","name":"Sol"},{"id":"a/terra","name":"Terra"}]}"""),
        control("""{"id":"sidekick","name":"Sidekick","category":"_sidekick","value":"none","options":[{"id":"none","name":"None"},{"id":"a/sol","name":"Sol"},{"id":"a/terra","name":"Terra"}]}"""),
        control("""{"id":"approval","name":"Approval","category":"_permission_mode","value":"ask","options":[{"id":"ask","name":"Ask"},{"id":"full","name":"Full"}]}"""),
      ),
    )
    val effort = control("""{"id":"effort","name":"Effort","category":"thought_level","options":[{"id":"low","name":"Low"},{"id":"xhigh","name":"XHigh"}],"value":"xhigh"}""")
    val shapes: ModelShapes = mapOf("a/terra" to listOf(effort))
    // Nothing answers a draft's pick: the level select comes from the picked model's shape, next to the sidekick
    val picked = editTurnConfig(controls, controls, "sidekick", "a/terra", shapes)
    assertEquals(listOf("model", "sidekick", "sidekick_effort", "approval"), picked.options.map { it.id })
    assertEquals("xhigh", picked.options[2].value)
    assertEquals(listOf("low", "xhigh"), picked.options[2].options.map { it.id })
    // A level picked meanwhile holds while the next model offers it; none, or a model never seen, drops the select
    val low = editTurnConfig(picked, picked, "sidekick_effort", "low", shapes)
    assertEquals("low", editTurnConfig(low, low, "sidekick", "a/terra", shapes).options[2].value)
    assertEquals(listOf("model", "sidekick", "approval"), editTurnConfig(low, low, "sidekick", "none", shapes).options.map { it.id })
    assertEquals(listOf("model", "sidekick", "approval"), editTurnConfig(low, low, "sidekick", "a/sol", shapes).options.map { it.id })
  }
}
