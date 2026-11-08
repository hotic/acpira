package cc.hotic.acpira.store

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.model.HostMsg
import cc.hotic.acpira.model.TerminalExit
import cc.hotic.acpira.model.WebviewMsg

// The phone's side of the engine's terminals (rust/crates/acpira-host/src/terminal.rs), ported from apps/ios
// Terminal/TerminalSession.swift (the desktop's packages/ui/panel/terminals.ts): the shell runs on the computer, the
// phone opens it at the size its view lays out, shows what it prints and acknowledges what it rendered, so a flood waits
// on the phone instead of piling up. Leaving the page only stops viewing it: the engine keeps the shell a minute without
// a viewer, and opening the same id within it replays what the shell printed. No views here, so the flow is unit tested;
// ui/terminal/TerminalPage.kt draws it.

/** What the page's terminal view does with the engine's output: the emulator on screen, a recorder in tests */
interface TerminalScreen {
  /** Output to parse and show, as it came */
  fun write(text: String)
  /** The engine's output so far, in place of what the view had (the page came back, or the shell restarted) */
  fun replay(text: String)
  /** A dim line of the app's own on a line of its own: the exit, a failed start */
  fun note(text: String)
}

/** One terminal of the engine as the phone sees it: one per project folder, kept for the model's life */
class TerminalSession(val id: String, val cwd: String, private val post: (WebviewMsg) -> Unit) {
  /** The engine's title: the foreground program's name, null until its first state */
  var title by mutableStateOf<String?>(null)
    private set
  var exit by mutableStateOf<TerminalExit?>(null)
    private set
  /** The shell could not start */
  var error by mutableStateOf<String?>(null)
    private set
  /** Viewed through this connection since the last open, and not let go of since */
  var opened = false
    private set
  /** The pty's size as far as the phone knows (cols to rows) */
  var size: Pair<Int, Int>? = null
    private set
  /** Output written to the screen and not acknowledged yet, in UTF-16 units (what the engine counts) */
  var unacked = 0
    private set
  var screen: TerminalScreen? = null
    private set

  /** The shell ended or never started: Return starts a new one */
  val ended: Boolean get() = exit != null || error != null

  /**
   * A view shows the terminal, laid out at [cols] x [rows]: the engine starts the shell there, or replays a running one
   * to it. A second view takes over from the first
   */
  fun open(on: TerminalScreen, cols: Int, rows: Int) {
    screen = on
    size = cols to rows
    opened = true
    unacked = 0
    post(WebviewMsg.TerminalOpen(id, cwd, cols, rows))
  }

  /** The view went with its page: the engine stops sending, the shell runs on for a return */
  fun close(from: TerminalScreen) {
    if (screen !== from) return
    screen = null
    if (!opened) return
    opened = false
    unacked = 0
    post(WebviewMsg.TerminalClose(id))
  }

  /**
   * Bytes the view sends for keys and pastes. UTF-8 goes as text; anything else (legacy mouse reports) as `binary`, one
   * character per byte. Once the shell ended, Return starts a new one and other keys do nothing
   */
  fun input(bytes: ByteArray) {
    if (!opened) return
    if (ended) {
      if (bytes.size == 1 && bytes[0] == CR) restart()
      return
    }
    val text = utf8(bytes)
    if (text != null) post(WebviewMsg.TerminalInput(id, text))
    else post(WebviewMsg.TerminalInput(id, String(CharArray(bytes.size) { (bytes[it].toInt() and 0xff).toChar() }), binary = true))
  }

  /** The view laid out another grid (rotation, the keyboard coming or going) */
  fun resize(cols: Int, rows: Int) {
    if (size == cols to rows) return
    size = cols to rows
    if (opened) post(WebviewMsg.TerminalResize(id, cols, rows))
  }

  /** A new shell in the same place, the old output cleared: the engine answers the open with an empty replay */
  fun restart() {
    exit = null
    error = null
    val s = screen ?: return
    val (cols, rows) = size ?: return
    if (opened) post(WebviewMsg.TerminalKill(id))
    open(s, cols, rows)
  }

  /** End the shell (the page's close); the same id opens a new one */
  fun kill() {
    if (opened) post(WebviewMsg.TerminalKill(id))
    opened = false
    unacked = 0
    title = null
    exit = null
    error = null
  }

  /** The engine's messages for this terminal (TerminalHub routes them) */
  fun receive(message: HostMsg) {
    when (message) {
      is HostMsg.TerminalOutput -> {
        val s = screen
        if (!opened || s == null) return
        s.write(message.data)
        // The emulator takes the output into its buffer on the call: what xterm's write callback waits for
        unacked += message.data.length
        if (unacked >= ACK_EVERY) {
          post(WebviewMsg.TerminalAck(id, unacked))
          unacked = 0
        }
      }
      is HostMsg.TerminalState -> {
        if (message.cols != null && message.rows != null) size = message.cols!! to message.rows!!
        message.replay?.let {
          // The engine counts from zero again for this view
          unacked = 0
          exit = null
          error = null
          screen?.replay(it)
        }
        message.title?.let { title = it }
        message.error?.let {
          error = it
          screen?.note(t("terminal.failed", "error" to it))
        }
        message.exit?.let { e ->
          exit = e
          screen?.note(e.signal?.let { t("terminal.signaled", "signal" to it) } ?: t("terminal.exited", "code" to e.code))
        }
      }
      else -> {}
    }
  }

  companion object {
    /** Rendered output is acknowledged in steps this size (xterm.ts ACK_EVERY), well under the engine's high water mark */
    const val ACK_EVERY = 5_000
    private const val CR: Byte = 0x0d

    private fun utf8(bytes: ByteArray): String? = runCatching {
      java.nio.charset.StandardCharsets.UTF_8.newDecoder()
        .onMalformedInput(java.nio.charset.CodingErrorAction.REPORT)
        .onUnmappableCharacter(java.nio.charset.CodingErrorAction.REPORT)
        .decode(java.nio.ByteBuffer.wrap(bytes)).toString()
    }.getOrNull()
  }
}

/** The phone's terminals by id, and the router for the engine's terminal messages */
class TerminalHub(private val post: (WebviewMsg) -> Unit) {
  private val sessions = HashMap<String, TerminalSession>()

  /** The project's terminal, made on first use */
  fun session(cwd: String): TerminalSession = sessions.getOrPut(id(cwd)) { TerminalSession(id(cwd), cwd, post) }

  /** terminalOutput / terminalState to the terminal they name; false for anything else */
  fun receive(message: HostMsg): Boolean {
    val id = when (message) {
      is HostMsg.TerminalOutput -> message.id
      is HostMsg.TerminalState -> message.id
      else -> return false
    }
    sessions[id]?.receive(message)
    return true
  }

  companion object {
    /**
     * A project's terminal id: stable, so the page reopens the same shell. The engine's terminals are shared by every
     * client of a persistent engine, so the prefix keeps the phone's apart from the desktop's `terminal-<uuid>` tabs
     */
    fun id(cwd: String) = "phone:$cwd"
  }
}

/** The project's terminal, made on first use; actions go to the engine */
fun AppModel.terminal(cwd: String): TerminalSession = terminals.session(cwd)

/**
 * The chat menu's "Terminal": the project's terminal page, or in the demo the note that it takes an engine (a shell
 * needs a computer to run on)
 */
fun AppModel.openTerminal(cwd: String) {
  if (!live) return showToast(t("mobile.engineNeeded"))
  push(Screen.Terminal(cwd))
}

/** The page's "close": the shell ends and the page goes */
fun AppModel.closeTerminal(cwd: String) {
  terminal(cwd).kill()
  if (path.lastOrNull() == Screen.Terminal(cwd)) pop()
}
