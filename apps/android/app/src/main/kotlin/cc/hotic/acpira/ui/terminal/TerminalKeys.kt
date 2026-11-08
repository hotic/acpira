package cc.hotic.acpira.ui.terminal

import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.waitForUpOrCancellation
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.TerminalMetrics
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.i18n.t
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

// Ported from apps/ios Terminal/TerminalKeys.swift: the keys a shell wants that the software keyboard lacks, on a bar
// over it. Esc, a sticky Ctrl, Tab, the arrows (repeating while held), symbols buried on the keyboard's symbol pages,
// and a key that puts the keyboard away. The row scrolls sideways where it does not fit; the keyboard key stays at its end.

/** A key on the bar */
sealed interface TerminalKey {
  data object Esc : TerminalKey
  data object Ctrl : TerminalKey
  data object Tab : TerminalKey
  data object Left : TerminalKey
  data object Down : TerminalKey
  data object Up : TerminalKey
  data object Right : TerminalKey
  data object Hide : TerminalKey
  data class Text(val text: String) : TerminalKey

  companion object {
    /** Symbols the phone keyboard keeps a page or two away */
    val symbols = listOf("~", "|", "/", "\\", "-", "_", "`")
  }
}

/** The bar: one scrolling row of capsules, the keyboard key fixed at the end; [ctrl] fills Ctrl while it waits */
@Composable
fun TerminalKeys(ctrl: Boolean, press: (TerminalKey) -> Unit, modifier: Modifier = Modifier) {
  Row(
    modifier.fillMaxWidth().height(TerminalMetrics.keyBar).testTag("terminal.keys"),
    verticalAlignment = Alignment.CenterVertically,
    horizontalArrangement = Arrangement.spacedBy(Metrics.s1),
  ) {
    Row(
      Modifier.weight(1f).horizontalScroll(rememberScrollState()).padding(horizontal = Metrics.s2),
      horizontalArrangement = Arrangement.spacedBy(Metrics.s1),
      verticalAlignment = Alignment.CenterVertically,
    ) {
      Key(TerminalKey.Esc, press, tag = "esc") { Word("esc") }
      Key(TerminalKey.Ctrl, press, tag = "ctrl", on = ctrl) { Word("ctrl", on = ctrl) }
      Key(TerminalKey.Tab, press, tag = "tab") { Word("tab") }
      Key(TerminalKey.Left, press, tag = "left", repeats = true) { Glyph("arrowLeft") }
      Key(TerminalKey.Down, press, tag = "down", repeats = true) { Glyph("arrowDown") }
      Key(TerminalKey.Up, press, tag = "up", repeats = true) { Glyph("arrowUp") }
      Key(TerminalKey.Right, press, tag = "right", repeats = true) { Glyph("arrowRight") }
      for (symbol in TerminalKey.symbols) Key(TerminalKey.Text(symbol), press, tag = "sym") { Word(symbol) }
    }
    Key(TerminalKey.Hide, press, tag = "hide", label = t("mobile.terminal.hideKeyboard"), modifier = Modifier.padding(end = Metrics.s2)) {
      Glyph("chevronDown")
    }
  }
}

/**
 * A key at the control height, round for a glyph and a capsule for a word; one that is [on] fills with the ink. A
 * [repeats] key fires again while held, as the keyboard's own arrows do
 */
@Composable
private fun Key(
  key: TerminalKey,
  press: (TerminalKey) -> Unit,
  tag: String,
  modifier: Modifier = Modifier,
  on: Boolean = false,
  repeats: Boolean = false,
  label: String? = null,
  content: @Composable () -> Unit,
) {
  val haptics = LocalHapticFeedback.current
  val fire by rememberUpdatedState {
    haptics.performHapticFeedback(HapticFeedbackType.KeyboardTap)
    press(key)
  }
  Box(
    modifier
      .heightIn(min = Metrics.ctl).widthIn(min = Metrics.ctl)
      .clip(CircleShape)
      .background(if (on) Palette.fg1 else Palette.chip)
      .pointerInput(repeats) {
        coroutineScope {
          awaitEachGesture {
            awaitFirstDown()
            if (!repeats) {
              if (waitForUpOrCancellation() != null) fire()
              return@awaitEachGesture
            }
            // Fires on the press, then again after a beat while still held
            fire()
            val again = launch {
              delay(TerminalMetrics.repeatAfterMs)
              while (true) { fire(); delay(TerminalMetrics.repeatEveryMs) }
            }
            waitForUpOrCancellation()
            again.cancel()
          }
        }
      }
      .semantics {
        role = Role.Button
        if (key == TerminalKey.Ctrl) selected = on
        label?.let { contentDescription = it }
      }
      .padding(horizontal = Metrics.s2)
      .testTag("terminal.key.$tag"),
    contentAlignment = Alignment.Center,
  ) { content() }
}

@Composable
private fun Word(text: String, on: Boolean = false) {
  Text(text, fontSize = Type.text3, fontWeight = FontWeight.Medium, color = if (on) Palette.bg0 else Palette.fg1, maxLines = 1)
}

@Composable
private fun Glyph(name: String) = Lucide(name, Palette.fg1, size = Metrics.icon)
