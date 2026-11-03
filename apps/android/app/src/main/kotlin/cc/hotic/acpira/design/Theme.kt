package cc.hotic.acpira.design

import android.content.Context
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontVariation
import androidx.compose.ui.text.font.FontWeight

/** Inter, the webview's and the iOS app's UI face: the variable font from the iOS bundle, one instance per weight */
object Fonts {
  @Volatile private var family: FontFamily? = null

  fun inter(context: Context): FontFamily = family ?: synchronized(this) {
    family ?: build(context).also { family = it }
  }

  private fun build(context: Context): FontFamily {
    val assets = context.assets
    val weights = listOf(400, 500, 600, 700)
    return FontFamily(
      weights.map { w ->
        Font("fonts/InterVariable.ttf", assets, FontWeight(w), FontStyle.Normal, FontVariation.Settings(FontVariation.weight(w)))
      } + weights.map { w ->
        Font("fonts/InterVariable-Italic.ttf", assets, FontWeight(w), FontStyle.Italic, FontVariation.Settings(FontVariation.weight(w)))
      },
    )
  }
}

/**
 * The app theme: Material 3 with its scheme taken from the Acpira tokens, so the system components (bars, sheets,
 * menus, switches, dialogs) wear the same neutral palette the iOS app shows
 */
@Composable
fun AcpiraTheme(dark: Boolean, content: @Composable () -> Unit) {
  val context = LocalContext.current
  val inter = remember { Fonts.inter(context) }
  val s = Swatches
  fun pick(w: Swatch) = if (dark) w.dark else w.light
  val scheme = if (dark) {
    darkColorScheme(
      primary = pick(s.btn1Bg), onPrimary = pick(s.btn1Fg),
      secondary = pick(s.fg2), onSecondary = pick(s.bg0),
      background = pick(s.bg0), onBackground = pick(s.fg1),
      surface = pick(s.bg0), onSurface = pick(s.fg1), onSurfaceVariant = pick(s.fg2),
      surfaceContainerLowest = pick(s.bg0), surfaceContainerLow = pick(s.bg1), surfaceContainer = pick(s.bg1),
      surfaceContainerHigh = pick(s.bg2), surfaceContainerHighest = pick(s.bg2),
      secondaryContainer = pick(s.active), onSecondaryContainer = pick(s.fg1),
      outline = pick(s.lineStrong), outlineVariant = pick(s.conversationLine),
      error = pick(s.danger),
    )
  } else {
    lightColorScheme(
      primary = pick(s.btn1Bg), onPrimary = pick(s.btn1Fg),
      secondary = pick(s.fg2), onSecondary = pick(s.bg0),
      background = pick(s.bg0), onBackground = pick(s.fg1),
      surface = pick(s.bg0), onSurface = pick(s.fg1), onSurfaceVariant = pick(s.fg2),
      surfaceContainerLowest = pick(s.bg1), surfaceContainerLow = pick(s.bg1), surfaceContainer = pick(s.bg1),
      surfaceContainerHigh = pick(s.bg1), surfaceContainerHighest = pick(s.bg2),
      secondaryContainer = pick(s.active), onSecondaryContainer = pick(s.fg1),
      outline = pick(s.lineStrong), outlineVariant = pick(s.conversationLine),
      error = pick(s.danger),
    )
  }
  val base = Typography()
  fun TextStyle.inter() = copy(fontFamily = inter)
  val typography = Typography(
    displayLarge = base.displayLarge.inter(), displayMedium = base.displayMedium.inter(), displaySmall = base.displaySmall.inter(),
    headlineLarge = base.headlineLarge.inter(), headlineMedium = base.headlineMedium.inter(), headlineSmall = base.headlineSmall.inter(),
    titleLarge = base.titleLarge.copy(fontFamily = inter, fontSize = Type.barTitle, fontWeight = FontWeight.SemiBold),
    titleMedium = base.titleMedium.inter(), titleSmall = base.titleSmall.inter(),
    bodyLarge = base.bodyLarge.copy(fontFamily = inter, fontSize = Type.body, lineHeight = Type.bodyLine),
    bodyMedium = base.bodyMedium.copy(fontFamily = inter, fontSize = Type.text2, lineHeight = Type.text2Line),
    bodySmall = base.bodySmall.copy(fontFamily = inter, fontSize = Type.text4, lineHeight = Type.text4Line),
    labelLarge = base.labelLarge.copy(fontFamily = inter, fontSize = Type.text3),
    labelMedium = base.labelMedium.inter(), labelSmall = base.labelSmall.inter(),
  )
  CompositionLocalProvider(LocalDark provides dark) {
    MaterialTheme(colorScheme = scheme, typography = typography, content = content)
  }
}
