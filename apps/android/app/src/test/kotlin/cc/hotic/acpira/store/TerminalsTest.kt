package cc.hotic.acpira.store

import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.model.HostMsg
import cc.hotic.acpira.model.TerminalExit
import cc.hotic.acpira.model.WebviewMsg
import org.junit.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

/**
 * The phone's terminals (store/Terminals.kt) against panel/terminals.ts and xterm.ts: what the page posts as it opens,
 * types, resizes, acknowledges and lets go, and what the engine's state does to the view. Mirrors apps/ios TerminalTests.swift
 */
class TerminalsTest {
  /** A view that records what the session hands it */
  private class RecordingScreen : TerminalScreen {
    val shown = ArrayList<String>()
    override fun write(text: String) { shown += text }
    override fun replay(text: String) { shown.clear(); shown += "replay:$text" }
    override fun note(text: String) { shown += "note:$text" }
  }

  private val id = "phone:/w"
  private val sent = ArrayList<WebviewMsg>()
  private val s = TerminalSession(id, "/w") { sent += it }
  private val screen = RecordingScreen()

  @Test fun opensAtTheLaidOutSizeAndAcknowledgesRenderedOutputInSteps() {
    // Laid out before the open: nothing goes to the engine yet
    s.resize(50, 30)
    assertTrue(sent.isEmpty())
    s.open(screen, 50, 30)
    assertEquals(listOf<WebviewMsg>(WebviewMsg.TerminalOpen(id, "/w", 50, 30)), sent)
    s.receive(HostMsg.TerminalState(id, title = "zsh", cols = 50, rows = 30, replay = ""))
    assertEquals("zsh", s.title)
    // UTF-16 units, as the engine counts: 4 999 units, then one 2-unit emoji crosses the step
    s.receive(HostMsg.TerminalOutput(id, "a".repeat(4_999)))
    assertEquals(1, sent.size)
    s.receive(HostMsg.TerminalOutput(id, "😀"))
    assertEquals(WebviewMsg.TerminalAck(id, 5_001), sent.last())
    assertEquals(0, s.unacked)
    assertEquals(3, screen.shown.size)
  }

  @Test fun aReplayStartsTheViewOverAndTheCountWithIt() {
    s.open(screen, 80, 24)
    s.receive(HostMsg.TerminalOutput(id, "x".repeat(4_000)))
    s.receive(HostMsg.TerminalState(id, cols = 60, rows = 20, replay = "so far"))
    assertEquals(listOf("replay:so far"), screen.shown)
    assertEquals(60 to 20, s.size)
    s.receive(HostMsg.TerminalOutput(id, "y".repeat(4_000)))
    // 4 000 after the replay, not 8 000: no ack yet
    assertTrue(sent.none { it is WebviewMsg.TerminalAck })
  }

  @Test fun resizesOnlyWhenTheGridChanges() {
    s.open(screen, 80, 24)
    s.resize(80, 24)
    s.resize(80, 12)
    assertEquals(listOf<WebviewMsg>(WebviewMsg.TerminalResize(id, 80, 12)), sent.drop(1))
  }

  @Test fun keysGoAsTextOrAsBytes() {
    s.input("ls\r".toByteArray())
    assertTrue(sent.isEmpty())
    s.open(screen, 80, 24)
    s.input("ls é\r".toByteArray())
    // A legacy mouse report: bytes past 0x7f that are not UTF-8
    s.input(byteArrayOf(0x1b, 0x5b, 0x4d, 0x20, 0xa1.toByte(), 0x21))
    assertEquals(
      listOf<WebviewMsg>(WebviewMsg.TerminalInput(id, "ls é\r"), WebviewMsg.TerminalInput(id, "\u001b[M ¡!", binary = true)),
      sent.drop(1),
    )
  }

  @Test fun anEndedShellNotesWhyAndReturnStartsANewOne() {
    s.open(screen, 80, 24)
    s.receive(HostMsg.TerminalState(id, exit = TerminalExit(0)))
    assertTrue(s.ended)
    assertEquals(listOf("note:${t("terminal.exited", "code" to 0)}"), screen.shown)
    s.input("x".toByteArray())
    assertEquals(1, sent.size)
    s.input(byteArrayOf(0x0d))
    assertEquals(listOf(WebviewMsg.TerminalKill(id), WebviewMsg.TerminalOpen(id, "/w", 80, 24)), sent.drop(1))
    assertFalse(s.ended)
    s.receive(HostMsg.TerminalState(id, error = "no shell"))
    assertEquals("no shell", s.error)
    assertEquals("note:${t("terminal.failed", "error" to "no shell")}", screen.shown.last())
  }

  @Test fun leavingLetsGoOnceAndOnlyFromTheViewThatShowsIt() {
    val second = RecordingScreen()
    s.open(screen, 80, 24)
    // The page came back (a rebuilt view) before the old view went: the old view lets go of nothing
    s.open(second, 80, 24)
    s.close(screen)
    assertEquals(WebviewMsg.TerminalOpen(id, "/w", 80, 24), sent.last())
    s.close(second)
    s.close(second)
    assertEquals(1, sent.count { it == WebviewMsg.TerminalClose(id) })
    // Output still on its way is neither shown nor acknowledged
    s.receive(HostMsg.TerminalOutput(id, "z".repeat(6_000)))
    assertTrue(second.shown.isEmpty())
    assertEquals(WebviewMsg.TerminalClose(id), sent.last())
  }

  @Test fun theHubRoutesByIdAndKeepsThePhonesTerminalsApart() {
    val out = ArrayList<WebviewMsg>()
    val hub = TerminalHub { out += it }
    val a = hub.session("/w")
    assertTrue(a === hub.session("/w"))
    assertEquals("phone:/w", a.id)
    a.open(screen, 80, 24)
    assertTrue(hub.receive(HostMsg.TerminalState(id, title = "vim")))
    assertEquals("vim", a.title)
    // Another client's terminal is the hub's to route but no session of the phone's
    assertTrue(hub.receive(HostMsg.TerminalOutput("terminal-1", "x")))
    assertEquals(listOf<WebviewMsg>(WebviewMsg.TerminalOpen(id, "/w", 80, 24)), out)
  }
}
