package cc.hotic.acpira.ui.terminal

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.toArgb
import cc.hotic.acpira.design.Swatch
import cc.hotic.acpira.design.Swatches
import kotlin.math.cbrt
import kotlin.math.pow

// Ported from apps/ios Terminal/TerminalLook.swift: the desktop terminal's look (packages/ui/styles/terminal.css, read
// by panel/xterm.ts) resolved for the phone. The text colours on the page's ground, the sixteen ANSI colours as GitHub's
// hues mixed into the text colour in oklab, a set each for dark and light. xterm.ts reads every ANSI colour through a
// canvas as #rrggbb, which drops the alpha a mix with --fg-2 (translucent in dark) leaves; so do these.

/** The colours one appearance of the terminal draws with */
data class TerminalLook(
  val foreground: Color,
  val background: Color,
  val selection: Color,
  /** black, red, green, yellow, blue, magenta, cyan, white, then their bright forms (xterm's order) */
  val ansi: List<Color>,
) {
  /** The palette as the emulator takes it */
  val ansiArgb: IntArray get() = IntArray(ansi.size) { ansi[it].toArgb() }

  companion object {
    fun of(dark: Boolean): TerminalLook {
      fun pick(s: Swatch) = if (dark) s.dark else s.light
      val fg1 = pick(Swatches.fg1)
      val fg2 = pick(Swatches.fg2)
      val fg3 = pick(Swatches.fg3)
      // --panel-bg falls back to --bg-0: the page the terminal fills
      val bg = pick(Swatches.bg0)
      // The hues: GitHub's dark and light terminal palettes, normal then bright, red to cyan
      val hues = if (dark) {
        listOf(0xff7b72, 0x3fb950, 0xd29922, 0x58a6ff, 0xbc8cff, 0x39c5cf, 0xffa198, 0x56d364, 0xe3b341, 0x79c0ff, 0xd2a8ff, 0x56d4dd)
      } else {
        listOf(0xcf222e, 0x116329, 0x7d4e00, 0x0969da, 0x8250df, 0x1b7c83, 0xa40e26, 0x1a7f37, 0x633c01, 0x218bff, 0xa475f9, 0x3192aa)
      }.map { Color(0xff000000.toInt() or it) }
      // --term-tint / --term-tint-bright: how much hue goes into the text colour
      val tint = if (dark) 0.34 else 0.46
      val tintBright = if (dark) 0.42 else 0.52
      val normal = hues.take(6).map { OKLab.mix(it, tint, fg2).opaque() }
      val bright = hues.takeLast(6).map { OKLab.mix(it, tintBright, fg1).opaque() }
      // The grey ends come from the text levels; on a light ground black is the strongest ink and white the faintest
      val black = if (dark) fg3 else fg1
      val white = if (dark) fg2 else fg3
      val brightBlack = if (dark) OKLab.mix(fg3, 0.7, fg2) else fg2
      val brightWhite = if (dark) fg1 else fg3
      val ansi = listOf(black.opaque()) + normal + listOf(white.opaque(), brightBlack.opaque()) + bright + listOf(brightWhite.opaque())
      // iOS takes --focus; the phone's tokens name the same blue `command`
      val selection = pick(Swatches.command).let { it.copy(alpha = if (dark) 0.4f else 0.26f) }
      return TerminalLook(fg1, bg, selection, ansi)
    }

    /** The colour as xterm.ts's canvas reads it back: the same channels, alpha dropped */
    private fun Color.opaque() = copy(alpha = 1f)
  }
}

/**
 * `color-mix(in oklab, …)` as CSS Color 4 computes it: both colours to OKLab, premultiplied by their alpha, mixed, back
 * to sRGB with the channels clamped
 */
internal object OKLab {
  /** `color-mix(in oklab, a p, b)`: p is a's share */
  fun mix(a: Color, p: Double, b: Color): Color {
    val alpha = a.alpha * p + b.alpha * (1 - p)
    if (alpha <= 0) return Color(0f, 0f, 0f, 0f)
    val x = lab(a)
    val y = lab(b)
    val mixed = DoubleArray(3) { (x[it] * a.alpha * p + y[it] * b.alpha * (1 - p)) / alpha }
    val c = rgb(mixed)
    return Color(c[0].toFloat(), c[1].toFloat(), c[2].toFloat(), alpha.toFloat())
  }

  /** sRGB to OKLab (Björn Ottosson's matrices) */
  fun lab(c: Color): DoubleArray {
    val r = linear(c.red.toDouble())
    val g = linear(c.green.toDouble())
    val b = linear(c.blue.toDouble())
    val l = cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b)
    val m = cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b)
    val s = cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b)
    return doubleArrayOf(
      0.2104542553 * l + 0.7936177850 * m - 0.0040720468 * s,
      1.9779984951 * l - 2.4285922050 * m + 0.4505937099 * s,
      0.0259040371 * l + 0.7827717662 * m - 0.8086757660 * s,
    )
  }

  /** OKLab to sRGB, each channel clamped to 0..1 */
  fun rgb(lab: DoubleArray): DoubleArray {
    val l = (lab[0] + 0.3963377774 * lab[1] + 0.2158037573 * lab[2]).pow(3)
    val m = (lab[0] - 0.1055613458 * lab[1] - 0.0638541728 * lab[2]).pow(3)
    val s = (lab[0] - 0.0894841775 * lab[1] - 1.2914855480 * lab[2]).pow(3)
    return doubleArrayOf(
      4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
      -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
      -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s,
    ).map { gamma(it).coerceIn(0.0, 1.0) }.toDoubleArray()
  }

  private fun linear(c: Double) = if (c <= 0.04045) c / 12.92 else ((c + 0.055) / 1.055).pow(2.4)
  private fun gamma(c: Double) = if (c <= 0.0031308) 12.92 * c else 1.055 * c.pow(1 / 2.4) - 0.055
}
