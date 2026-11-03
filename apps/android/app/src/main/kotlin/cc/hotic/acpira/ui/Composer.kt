package cc.hotic.acpira.ui

import androidx.compose.animation.animateColorAsState
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.style.TextOverflow
import cc.hotic.acpira.design.Lucide
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.design.VendorMark
import cc.hotic.acpira.i18n.t

/**
 * The phone composer (apps/ios Chat/Composer.swift): one rounded card at the bottom with the attach button, the agent's
 * mark (the current model's vendor, else the agent's), the field and the send button. While a turn runs an empty field's button stops it, and a typed prompt queues
 * behind the turn. Grows to [MAX_LINES] lines, then scrolls inside
 */
@Composable
fun Composer(
  text: String,
  onText: (String) -> Unit,
  mark: String,
  markName: String,
  running: Boolean,
  placeholder: String,
  onSend: () -> Unit,
  onStop: () -> Unit,
  modifier: Modifier = Modifier,
) {
  val canSend = text.isNotBlank()
  Box(modifier.fillMaxWidth().navigationBarsPadding().imePadding().padding(horizontal = Metrics.pad, vertical = Metrics.s2)) {
    Surface(
      color = Palette.bg1,
      shape = RoundedCornerShape(Metrics.rComposer),
      shadowElevation = Metrics.barShadow,
      modifier = Modifier.fillMaxWidth().border(Metrics.hairline, Palette.line, RoundedCornerShape(Metrics.rComposer)),
    ) {
      Row(
        verticalAlignment = Alignment.Bottom,
        horizontalArrangement = Arrangement.spacedBy(Metrics.s1),
        modifier = Modifier.padding(horizontal = Metrics.s2, vertical = Metrics.s1_5),
      ) {
        IconButton(onClick = {}, modifier = Modifier.size(Metrics.mCtl + Metrics.s1).testTag("composer.attach")) {
          Lucide("plus", Palette.fg1, size = Metrics.iconCtl, contentDescription = t("attachments"))
        }
        Box(Modifier.size(Metrics.mCtl + Metrics.s1), contentAlignment = Alignment.Center) {
          VendorMark(mark, markName, Palette.fg1, size = Metrics.iconCtl, fit = true)
        }
        Box(Modifier.weight(1f).heightIn(min = Metrics.mCtl + Metrics.s1), contentAlignment = Alignment.CenterStart) {
          if (text.isEmpty()) {
            Text(placeholder, color = Palette.fg3, fontSize = Type.text1, maxLines = 1, overflow = TextOverflow.Ellipsis)
          }
          BasicTextField(
            value = text,
            onValueChange = onText,
            textStyle = TextStyle(color = Palette.fg1, fontSize = Type.text1, lineHeight = Type.text2Line),
            cursorBrush = SolidColor(Palette.fg1),
            maxLines = MAX_LINES,
            keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Sentences),
            modifier = Modifier.fillMaxWidth().padding(vertical = Metrics.s2).testTag("composer.field"),
          )
        }
        val stop = running && !canSend
        val fill by animateColorAsState(if (canSend || stop) Palette.btn1Bg else Palette.sendBg, label = "send")
        FilledIconButton(
          onClick = { if (stop) onStop() else if (canSend) onSend() },
          enabled = canSend || stop,
          shape = CircleShape,
          colors = IconButtonDefaults.filledIconButtonColors(
            containerColor = fill, contentColor = Palette.btn1Fg,
            disabledContainerColor = fill, disabledContentColor = Palette.sendInk,
          ),
          modifier = Modifier.size(Metrics.mCtl + Metrics.s1).testTag(if (stop) "composer.stop" else "composer.send"),
        ) {
          if (stop) Box(Modifier.size(Metrics.gap).background(Palette.btn1Fg, RoundedCornerShape(Metrics.rXs)))
          else Lucide("arrowUp", if (canSend) Palette.btn1Fg else Palette.sendInk, size = Metrics.iconCtl, contentDescription = t("composer.send"))
        }
      }
    }
  }
}

private const val MAX_LINES = 6
