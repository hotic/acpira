package cc.hotic.acpira.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type
import cc.hotic.acpira.store.AppModel

/**
 * The toast's button (apps/ios ToastView's action): the capsule's last word after a hairline, for the toast it was
 * shown with only ([toast] its text)
 */
@Composable
fun ToastActionButton(model: AppModel, toast: String?) {
  val action = model.lists.toastAction?.takeIf { it.first == toast }?.second ?: return
  Row(verticalAlignment = Alignment.CenterVertically) {
    Box(Modifier.width(Metrics.hairline).height(Metrics.iconCtl).background(Palette.btn1Fg.copy(alpha = RULE_ALPHA)))
    Text(
      action.label, color = Palette.btn1Fg, fontSize = Type.text3, fontWeight = FontWeight.SemiBold,
      modifier = Modifier.clickable { model.lists.runToastAction() }.padding(horizontal = Metrics.pad, vertical = Metrics.s2 + Metrics.s1).testTag("toast.action"),
    )
  }
}

/** The hairline before the button, in the toast's ink */
private const val RULE_ALPHA = 0.25f
