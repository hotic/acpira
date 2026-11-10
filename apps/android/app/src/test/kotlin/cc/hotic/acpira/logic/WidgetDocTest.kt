package cc.hotic.acpira.logic

import cc.hotic.acpira.ui.WidgetDoc
import cc.hotic.acpira.ui.WidgetPromptGate
import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue

/** Widget document head, theme swap and prompt rate-limit gate (apps/ios WidgetDocTests) */
class WidgetDocTest {
  @Test fun headDropsWidgetCspAndBaseAndExposesSendPrompt() {
    val doc = WidgetDoc.build(
      """<meta http-equiv="Content-Security-Policy" content="default-src *"><base href="https://x.test/"><p class="text-muted-foreground">hi</p>""",
      dark = true,
      tailwind = "const s = '</script>';",
      token = "tok-1",
    )
    assertTrue(doc.contains("default-src 'none'"))
    assertFalse(doc.contains("default-src *"))
    assertFalse(doc.contains("<base"))
    assertTrue(doc.contains("""<html class="dark">"""))
    assertTrue(doc.contains("sendPrompt: function (text)"))
    assertTrue(doc.contains("document.currentScript.remove()"))
    assertTrue(doc.contains("""var TOKEN = "tok-1""""))
    assertTrue(doc.contains("""<\/script>"""))
    assertTrue(doc.endsWith("""<p class="text-muted-foreground">hi</p>"""))
  }

  @Test fun themeSwitchSetsAllVariables() {
    val light = WidgetDoc.theme(dark = false)
    assertTrue(light.contains("classList.toggle('dark', false)"))
    assertTrue(light.contains("colorScheme = 'light'"))
    for ((name, _) in WidgetDoc.vars(dark = false)) {
      assertTrue(light.contains("\"$name\""))
    }
  }

  @Test fun promptGateTrimsCapsAndThrottles() {
    val gate = WidgetPromptGate()
    assertNull(gate.accept("   ", nowMs = 1_000L))
    assertNull(gate.accept("a".repeat(WidgetDoc.PROMPT_MAX + 1), nowMs = 1_000L))
    assertEquals("Picked D", gate.accept("  Picked D \n", nowMs = 1_000L))
    assertNull(gate.accept("Again", nowMs = 1_400L))
    assertEquals("Again", gate.accept("Again", nowMs = 2_100L))
  }
}
