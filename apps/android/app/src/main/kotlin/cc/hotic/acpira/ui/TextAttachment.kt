package cc.hotic.acpira.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.logic.PastedText

// Ported from apps/ios Chat/TextAttachment.swift: the same text attachment in the draft, the queue and the transcript
// opens its original contents in a sheet, whole, selectable and with Copy.

/** A text attachment's pill: a tap shows its whole text; [onRemove] adds a staged pill's ✕ */
@Composable
fun TextAttachmentPill(blob: String?, name: String, onRemove: (() -> Unit)? = null) {
  var open by remember { mutableStateOf(false) }
  AttachmentPill(name, "fileText", onOpen = { open = true }, onRemove = onRemove)
  if (open) TextAttachmentSheet(name, text = null, blob = blob) { open = false }
}

/**
 * The attachment's text in the code sheet: [text] when the app holds it (a paste still staged), else the base64 the
 * attachment carries, else its stored blob read back through the chat's loader
 */
@Composable
fun TextAttachmentSheet(name: String, text: String?, blob: String?, onDismiss: () -> Unit) {
  val media = LocalMedia.current
  // null while loading; a failed read says so in place of the text
  var shown by remember(text, blob) { mutableStateOf(text ?: PastedText.inline(blob)) }
  var failed by remember(text, blob) { mutableStateOf(false) }
  LaunchedEffect(text, blob, media) {
    if (shown != null) return@LaunchedEffect
    val session = media.session
    if (blob == null || session == null) { failed = true; return@LaunchedEffect }
    runCatching { media.loader.text(session, blob) }.onSuccess { shown = it }.onFailure { failed = true }
  }
  CodeSheet(name, shown.orEmpty(), t("common.copy"), onDismiss) {
    val body = shown
    when {
      body != null -> SelectionContainer {
        Text(body, color = Palette.fgStrong, fontSize = Type.body, lineHeight = Type.bodyLine, modifier = Modifier.fillMaxWidth().testTag("text.attachment"))
      }
      failed -> Text(t("attach.loadFailed", "name" to name), color = Palette.fg3, fontSize = Type.text2)
      else -> Column(Modifier.fillMaxWidth(), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(Metrics.s2)) {
        CircularProgressIndicator(color = Palette.fg2)
        Text(t("attach.loading"), color = Palette.fg3, fontSize = Type.text2)
      }
    }
  }
}
