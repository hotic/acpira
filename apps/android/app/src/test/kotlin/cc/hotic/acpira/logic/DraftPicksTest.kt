package cc.hotic.acpira.logic

import cc.hotic.acpira.model.ConfigControl
import cc.hotic.acpira.model.SessionControls
import cc.hotic.acpira.model.SessionOption
import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

/** The new session page's picks replayed on the session the engine made (apps/ios AppModel+Engine.swift `apply`) */
class DraftPicksTest {
  private fun opts(vararg ids: String) = ids.map { SessionOption(it, it) }
  private val modes = opts("default", "plan")
  private fun controls(model: String, effort: String, mode: String = "default") = SessionControls(
    modes = modes,
    modeId = mode,
    options = listOf(
      ConfigControl("model", "Model", category = "model", options = opts("a", "b"), value = model),
      ConfigControl("effort", "Effort", category = "thought_level", options = opts("low", "high"), value = effort),
    ),
  )

  @Test fun onlyDifferencesWhileTheModelStays() {
    val replay = replayPicks(controls("a", "high", mode = "plan"), controls("a", "low"))
    assertEquals("plan", replay.mode)
    assertEquals(listOf("effort" to "high"), replay.config)
  }

  @Test fun aModelSwitchReplaysEveryPickAfterIt() {
    // The switch may reset an equal effort, so it goes again after the model
    val replay = replayPicks(controls("b", "low"), controls("a", "low"))
    assertNull(replay.mode)
    assertEquals(listOf("model" to "b", "effort" to "low"), replay.config)
  }

  @Test fun aModeTheSessionLacksStaysOut() {
    val made = controls("a", "low").copy(modes = opts("default"))
    assertNull(replayPicks(controls("a", "low", mode = "plan"), made).mode)
    assertEquals(emptyList(), replayPicks(controls("a", "low"), controls("a", "low")).config)
  }
}
