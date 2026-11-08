package cc.hotic.acpira.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.systemBarsPadding
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTagsAsResourceId
import androidx.compose.runtime.SideEffect
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogWindowProvider
import androidx.core.view.WindowCompat
import cc.hotic.acpira.design.LocalDark
import androidx.compose.ui.window.DialogProperties
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.i18n.t

// Ported from apps/ios Chat/ExpandedComposer.swift: the composer's whole-screen editor, as ChatGPT's expand corner opens
// one. A long draft is written and read over the whole page instead of inside the card's few lines. It edits the same
// draft, so collapsing returns to the card with the text as it stands; the staged attachments ride over the text as in
// the card, and the pickers stay in the card.

/** The draft over the whole screen: its attachments over the text, collapse in the bar, send above the keyboard */
@OptIn(ExperimentalComposeUiApi::class)
@Composable
fun ExpandedComposer(
  text: String,
  onText: (String) -> Unit,
  staged: List<Staged>,
  onStaged: (List<Staged>) -> Unit,
  assist: ComposerAssist,
  motion: Boolean,
  onSend: () -> Unit,
  onCollapse: () -> Unit,
  onToast: (String) -> Unit,
) {
  val prompt = rememberPromptState(text, onText)
  val longPaste = rememberLongPaste(staged, onStaged, onToast)
  val focus = remember { FocusRequester() }
  val filled = text.isNotBlank() || staged.isNotEmpty()
  Dialog(onDismissRequest = onCollapse, properties = DialogProperties(usePlatformDefaultWidth = false, decorFitsSystemWindows = false)) {
    // The dialog's own window takes the page's bar ink, or the clock reads white on a light page
    val window = (LocalView.current.parent as? DialogWindowProvider)?.window
    val view = LocalView.current
    val dark = LocalDark.current
    SideEffect {
      window?.let { WindowCompat.getInsetsController(it, view).apply { isAppearanceLightStatusBars = !dark; isAppearanceLightNavigationBars = !dark } }
    }
    // A dialog is a window of its own: its test tags need the resource-id switch again
    Column(
      Modifier.fillMaxSize().background(Palette.bg0).semantics { testTagsAsResourceId = true }
        .systemBarsPadding().imePadding().testTag("expanded"),
      verticalArrangement = Arrangement.spacedBy(Metrics.s2),
    ) {
      Row(Modifier.fillMaxWidth().height(Metrics.barCtl).padding(horizontal = Metrics.s1), verticalAlignment = Alignment.CenterVertically) {
        Box(Modifier.weight(1f))
        Box(
          Modifier.size(Metrics.barCtl).clip(CircleShape).clickable(onClick = onCollapse).testTag("expanded.collapse"),
          contentAlignment = Alignment.Center,
        ) { Lucide("minimize2", Palette.fg1, size = Metrics.iconCtl, contentDescription = t("mobile.composer.collapse")) }
      }
      if (staged.isNotEmpty()) StagedStrip(staged, onStaged, Modifier.padding(start = Metrics.pad), onNavigate = onCollapse)
      PromptField(
        prompt,
        commands = assist.commands.map { it.name }, summons = assist.personas.map { it.name },
        onLongPaste = longPaste, maxLines = Int.MAX_VALUE, fill = true,
        modifier = Modifier.weight(1f).padding(horizontal = Metrics.pad),
        fieldModifier = Modifier.focusRequester(focus).testTag("expanded.field"),
      )
      // Send rides the keyboard's top edge, at the corner it holds in the card
      Row(Modifier.fillMaxWidth().padding(horizontal = Metrics.pad, vertical = Metrics.s2)) {
        Box(Modifier.weight(1f))
        // A staged video still compressing holds the draft back, as in the card
        val waiting = VideoJobs.pending(staged)
        SendButton(running = false, filled = filled && !waiting, motion = motion) { if (!waiting) onSend() }
      }
    }
    // Typing goes on at once, as the editor opens on the draft
    LaunchedEffect(Unit) { runCatching { focus.requestFocus() } }
  }
}
