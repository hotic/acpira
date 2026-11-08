package cc.hotic.acpira.ui.terminal

import android.content.ClipboardManager
import android.content.Context
import android.os.Looper
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.isImeVisible
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.IconButton
import androidx.compose.material3.Scaffold
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalUriHandler
import androidx.compose.ui.platform.testTag
import cc.hotic.acpira.design.LocalDark
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.TerminalMetrics
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.store.AppModel
import cc.hotic.acpira.store.TerminalScreen
import cc.hotic.acpira.store.TerminalSession
import cc.hotic.acpira.store.closeTerminal
import cc.hotic.acpira.store.fileName
import cc.hotic.acpira.store.terminal
import cc.hotic.acpira.ui.AppMenu
import cc.hotic.acpira.ui.AppMenuItem
import cc.hotic.acpira.ui.MenuGlyph
import cc.hotic.acpira.ui.ToolPageBar
import org.connectbot.terminal.ModifierManager
import org.connectbot.terminal.Terminal
import org.connectbot.terminal.TerminalEmulator
import org.connectbot.terminal.TerminalEmulatorFactory
import org.connectbot.terminal.VTermKey

// Ported from apps/ios Terminal/TerminalPage.swift + TerminalSurface.swift: the project's shell on the engine's
// computer, the desktop panel's terminal tab (panel/TerminalFrame.tsx) at phone size. The bar names it after the program
// in the foreground with the project under it; its menu restarts or closes the shell. The emulator is ConnectBot's
// termlib (libvterm under a Compose canvas, Apache-2.0), the counterpart of SwiftTerm on iOS: a native view, no WebView.
// Copy is the emulator's long-press selection; the keys the software keyboard lacks sit on a bar over it (TerminalKeys).

@OptIn(ExperimentalLayoutApi::class)
@Composable
fun TerminalPage(model: AppModel, cwd: String) {
  val session = model.terminal(cwd)
  val dark = LocalDark.current
  val look = remember(dark) { TerminalLook.of(dark) }
  val context = LocalContext.current
  val uri = LocalUriHandler.current
  val ctrl = remember { StickyCtrl() }
  var keyboard by remember { mutableStateOf(true) }
  var menu by remember { mutableStateOf(false) }
  val focus = remember { FocusRequester() }
  val surface = remember(session) { EmulatorSurface(session, look) }
  // Leaving the page only stops viewing: the engine keeps the shell a while for a return, which replays it
  DisposableEffect(surface) { onDispose { surface.dispose() } }
  LaunchedEffect(look) { surface.apply(look) }
  Scaffold(
    containerColor = look.background,
    contentWindowInsets = WindowInsets(0),
    topBar = {
      ToolPageBar(session.title ?: t("panel.terminal"), "terminal", model::pop, subtitle = fileName(cwd), ground = look.background) {
        Box {
          IconButton(onClick = { menu = true }, modifier = Modifier.testTag("terminal.more")) {
            Lucide("ellipsis", Palette.fg1, size = Metrics.iconCtl, contentDescription = t("session.more"))
          }
          AppMenu(expanded = menu, onDismiss = { menu = false }) {
            AppMenuItem(
              t("terminal.restart"), onClick = { menu = false; session.restart() },
              icon = { MenuGlyph("rotateCcw") }, modifier = Modifier.testTag("terminal.restart"),
            )
            AppMenuItem(
              t("mobile.terminal.close"), danger = true, onClick = { menu = false; model.closeTerminal(cwd) },
              icon = { MenuGlyph("x", danger = true) }, modifier = Modifier.testTag("terminal.close"),
            )
          }
        }
      }
    },
  ) { padding ->
    Column(Modifier.fillMaxSize().padding(padding).navigationBarsPadding().imePadding()) {
      Terminal(
        terminalEmulator = surface.emulator,
        // The text set in from the edges like the desktop's (pt-gap pb-gap-half pl-pad)
        modifier = Modifier.weight(1f).fillMaxWidth()
          .padding(top = Metrics.gap, bottom = Metrics.gapHalf, start = Metrics.pad).testTag("terminal.surface"),
        initialFontSize = TerminalMetrics.fontSize,
        backgroundColor = look.background,
        foregroundColor = look.foreground,
        selectionBackgroundColor = look.selection,
        keyboardEnabled = true,
        showSoftKeyboard = keyboard,
        focusRequester = focus,
        onTerminalTap = { keyboard = true },
        modifierManager = ctrl,
        onHyperlinkClick = { runCatching { uri.openUri(it) } },
        onPasteRequest = { clipboardText(context)?.let(surface.emulator::pasteText) },
      )
      // The bar rides on the keyboard, as iOS's input accessory does
      if (WindowInsets.isImeVisible) {
        TerminalKeys(
          ctrl = ctrl.on,
          press = { key -> surface.press(key, ctrl); if (key == TerminalKey.Hide) keyboard = false },
          modifier = Modifier.background(look.background),
        )
      }
    }
  }
}

/**
 * The page's emulator and the [TerminalScreen] the session writes to: opened at the grid the first layout measures,
 * resized after, let go of with the page
 */
private class EmulatorSurface(val session: TerminalSession, look: TerminalLook) : TerminalScreen {
  val emulator: TerminalEmulator = TerminalEmulatorFactory.create(
    looper = Looper.getMainLooper(),
    defaultForeground = look.foreground,
    defaultBackground = look.background,
    // What the keys and pastes send: to the engine's pty
    onKeyboardInput = { session.input(it) },
    onResize = { d ->
      if (d.rows > 0 && d.columns > 0) {
        if (session.screen === this) session.resize(d.columns, d.rows) else session.open(this, d.columns, d.rows)
      }
    },
  ).also { it.applyColorScheme(look.ansiArgb, look.foreground.toArgb(), look.background.toArgb()) }
  private var closed = false

  fun apply(look: TerminalLook) {
    if (!closed) emulator.applyColorScheme(look.ansiArgb, look.foreground.toArgb(), look.background.toArgb())
  }

  override fun write(text: String) {
    if (!closed) emulator.writeInput(text.toByteArray(Charsets.UTF_8))
  }

  override fun replay(text: String) {
    // RIS clears the screen (xterm's reset); the theme's colours stay installed
    write("\u001bc")
    if (text.isNotEmpty()) write(text)
  }

  override fun note(text: String) = write("\r\n\u001b[2m$text\u001b[22m\r\n")

  /** A key of the bar: the emulator encodes it, so the arrows follow the program's cursor mode (vim and less want the application form) */
  fun press(key: TerminalKey, ctrl: StickyCtrl) {
    if (closed) return
    val mods = if (ctrl.isCtrlActive()) CTRL else 0
    when (key) {
      TerminalKey.Esc -> emulator.dispatchKey(mods, VTermKey.ESCAPE)
      TerminalKey.Tab -> emulator.dispatchKey(mods, VTermKey.TAB)
      TerminalKey.Up -> emulator.dispatchKey(mods, VTermKey.UP)
      TerminalKey.Down -> emulator.dispatchKey(mods, VTermKey.DOWN)
      TerminalKey.Left -> emulator.dispatchKey(mods, VTermKey.LEFT)
      TerminalKey.Right -> emulator.dispatchKey(mods, VTermKey.RIGHT)
      // Typed like a key of the keyboard, so a waiting Ctrl applies (Ctrl+\ quits, Ctrl+_ undoes)
      is TerminalKey.Text -> emulator.dispatchCharacter(mods, key.text.codePointAt(0))
      TerminalKey.Ctrl -> { ctrl.on = !ctrl.on; return }
      TerminalKey.Hide -> return
    }
    ctrl.clearTransients()
  }

  fun dispose() {
    if (closed) return
    session.close(this)
    closed = true
    emulator.close()
  }

  private companion object {
    /** libvterm's VTermModifier bit for Ctrl (termlib keeps its constant internal) */
    const val CTRL = 4
  }
}

/** The bar's sticky Ctrl, which the emulator also reads for the keyboard's own keys and lets go of once a key used it */
internal class StickyCtrl : ModifierManager {
  var on by mutableStateOf(false)
  override fun isCtrlActive() = on
  override fun isAltActive() = false
  override fun isShiftActive() = false
  override fun clearTransients() { on = false }
}

private fun clipboardText(context: Context): String? {
  val clip = (context.getSystemService(Context.CLIPBOARD_SERVICE) as? ClipboardManager)?.primaryClip ?: return null
  return clip.takeIf { it.itemCount > 0 }?.getItemAt(0)?.coerceToText(context)?.toString()
}
