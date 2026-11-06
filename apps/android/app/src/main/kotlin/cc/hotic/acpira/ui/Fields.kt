package cc.hotic.acpira.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import cc.hotic.acpira.design.Metrics
import cc.hotic.acpira.design.Palette
import cc.hotic.acpira.design.Type

/**
 * One line of text in a form: a card row (as the system's form text field sits), or [boxed] on its own fill (inside a
 * dialog). Paths, URLs and keys go in as typed: no autocorrect or capitals. mono: URLs and paths; secret: a key
 */
@Composable
internal fun TextInput(
  value: String,
  onValue: (String) -> Unit,
  placeholder: String,
  tag: String,
  modifier: Modifier = Modifier,
  boxed: Boolean = false,
  mono: Boolean = false,
  secret: Boolean = false,
  keyboard: KeyboardType = KeyboardType.Text,
  onDone: (() -> Unit)? = null,
) {
  val style = if (mono) TextStyle(color = Palette.fg1, fontSize = Type.mono, fontFamily = FontFamily.Monospace)
  else TextStyle(color = Palette.fg1, fontSize = Type.text1)
  val shell = if (boxed) Modifier.background(Palette.chip, RoundedCornerShape(Metrics.rLg)).heightIn(min = Metrics.ctl).padding(horizontal = Metrics.gap)
  else Modifier.heightIn(min = Metrics.formRowHeight).padding(horizontal = Metrics.pad)
  Box(modifier.fillMaxWidth().then(shell), contentAlignment = Alignment.CenterStart) {
    if (value.isEmpty()) Text(placeholder, style = style.copy(color = Palette.fg3), maxLines = 1)
    BasicTextField(
      value = value, onValueChange = onValue, singleLine = true, textStyle = style,
      cursorBrush = SolidColor(Palette.fg1),
      visualTransformation = if (secret) PasswordVisualTransformation() else VisualTransformation.None,
      keyboardOptions = KeyboardOptions(
        capitalization = KeyboardCapitalization.None, autoCorrectEnabled = false,
        keyboardType = if (secret) KeyboardType.Password else keyboard,
        imeAction = if (onDone != null) ImeAction.Done else ImeAction.Default,
      ),
      keyboardActions = KeyboardActions(onDone = { onDone?.invoke() }),
      modifier = Modifier.fillMaxWidth().testTag(tag),
    )
  }
}
