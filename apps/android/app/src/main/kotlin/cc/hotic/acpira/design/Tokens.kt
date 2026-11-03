package cc.hotic.acpira.design

import androidx.compose.runtime.Composable
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

// The webview's design tokens (packages/ui/styles/tokens.css) at the baseline the phone ships, the same values as the
// iOS app's AcpiraDesign/Tokens.swift. Components read only these names, never raw numbers (the rule AGENTS.md sets
// for the webview). Material 3 components take their colours from the scheme built out of these in AcpiraTheme.

/** One token's dark and light values */
class Swatch(val dark: Color, val light: Color) {
  constructor(both: Color) : this(both, both)

  fun alpha(a: Float) = Swatch(dark.copy(alpha = dark.alpha * a), light.copy(alpha = light.alpha * a))

  /** `color-mix(in srgb, self p, other)`: p is self's share */
  fun mix(other: Swatch, p: Float) = Swatch(mixOf(dark, other.dark, p), mixOf(light, other.light, p))

  private fun mixOf(a: Color, b: Color, p: Float) =
    Color(a.red * p + b.red * (1 - p), a.green * p + b.green * (1 - p), a.blue * p + b.blue * (1 - p), a.alpha * p + b.alpha * (1 - p))
}

private fun hex(v: Long, a: Float = 1f) = Color(((v shr 16) and 0xff).toInt(), ((v shr 8) and 0xff).toInt(), (v and 0xff).toInt(), (a * 255).toInt())
private fun white(a: Float) = Color(1f, 1f, 1f, a)
private fun black(a: Float) = Color(0f, 0f, 0f, a)

/** Each colour token's dark / light pair, `--name` in tokens.css → `Swatches.name` */
object Swatches {
  val fg1 = Swatch(hex(0xededed), hex(0x0d0d0d))
  val fg2 = Swatch(Color(202 / 255f, 202 / 255f, 202 / 255f, 0.7f), hex(0x6c6c6c))
  val fg3 = Swatch(hex(0x767676))
  val fgStrong = Swatch(hex(0xf5f5f5), hex(0x000000))
  val actionFg = Swatch(hex(0x858585), hex(0x707070))
  /** The phone's one near-black page in dark (list, chat and settings share it) */
  val bg0 = Swatch(hex(0x111111), hex(0xf8f8f8))
  val bg1 = Swatch(hex(0x1f1f1f), hex(0xffffff))
  val bg2 = Swatch(hex(0x262626), hex(0xf1f1f1))
  val bgCode = Swatch(hex(0x1f1f1f), hex(0xefefef))
  val line = Swatch(white(0.06f), black(0.06f))
  val conversationLine = Swatch(white(0.12f), black(0.10f))
  val chip = Swatch(white(0.07f), hex(0xf4f4f4))
  /** A settings form's page and its rows */
  val formPage = Swatch(hex(0x111111), hex(0xf2f2f7))
  val formRow = Swatch(hex(0x242424), hex(0xffffff))
  /** The prompt bubble, a step darker than --chip in light so it reads on the light page */
  val bubbleBg = Swatch(white(0.08f), hex(0xebebeb))
  val btn1Bg = Swatch(white(0.95f), hex(0x151515))
  val btn1Fg = Swatch(hex(0x151515), hex(0xffffff))
  val sendBg = Swatch(white(0.04f), black(0.04f))
  val sendInk = Swatch(white(0.5f), black(0.5f))
  val lineStrong = Swatch(hex(0x3c3c3c), hex(0xcecece))
  val hover = Swatch(white(0.05f), black(0.04f))
  val active = Swatch(white(0.08f), black(0.07f))
  val scrim = Swatch(black(0.72f), black(0.45f))
  val ok = Swatch(hex(0x439e7e), hex(0x2f7a5b))
  val warn = Swatch(hex(0xd29922), hex(0x9a6700))
  val danger = Swatch(hex(0xf85149), hex(0xcf222e))
  val brand = Swatch(hex(0xe6a63e), hex(0xb7791f))
  val command = Swatch(hex(0x6ea8fe), hex(0x2f6fe0))
  val shimmerBase = Swatch(hex(0x9a9a9a), hex(0x707070))
  val diffAdd = ok.mix(bgCode, 0.06f)
  val diffDel = danger.mix(bgCode, 0.08f)
  val railColor = fg3.mix(bg1, 0.65f)
  /** A card (alert, permission, plan): white on the light page, a step up in dark */
  val card = Swatch(hex(0x1c1c1c), hex(0xffffff))
}

val LocalDark = staticCompositionLocalOf { false }

/** Colour roles resolved for the current theme, `--name` → `Palette.name` */
object Palette {
  private val dark: Boolean @Composable @ReadOnlyComposable get() = LocalDark.current
  @Composable @ReadOnlyComposable private fun of(s: Swatch) = if (dark) s.dark else s.light

  val fg1 @Composable @ReadOnlyComposable get() = of(Swatches.fg1)
  val fg2 @Composable @ReadOnlyComposable get() = of(Swatches.fg2)
  val fg3 @Composable @ReadOnlyComposable get() = of(Swatches.fg3)
  val fgStrong @Composable @ReadOnlyComposable get() = of(Swatches.fgStrong)
  val actionFg @Composable @ReadOnlyComposable get() = of(Swatches.actionFg)
  val bg0 @Composable @ReadOnlyComposable get() = of(Swatches.bg0)
  val bg1 @Composable @ReadOnlyComposable get() = of(Swatches.bg1)
  val bg2 @Composable @ReadOnlyComposable get() = of(Swatches.bg2)
  val bgCode @Composable @ReadOnlyComposable get() = of(Swatches.bgCode)
  val line @Composable @ReadOnlyComposable get() = of(Swatches.line)
  val conversationLine @Composable @ReadOnlyComposable get() = of(Swatches.conversationLine)
  val chip @Composable @ReadOnlyComposable get() = of(Swatches.chip)
  val formPage @Composable @ReadOnlyComposable get() = of(Swatches.formPage)
  val formRow @Composable @ReadOnlyComposable get() = of(Swatches.formRow)
  val bubbleBg @Composable @ReadOnlyComposable get() = of(Swatches.bubbleBg)
  val btn1Bg @Composable @ReadOnlyComposable get() = of(Swatches.btn1Bg)
  val btn1Fg @Composable @ReadOnlyComposable get() = of(Swatches.btn1Fg)
  val sendBg @Composable @ReadOnlyComposable get() = of(Swatches.sendBg)
  val sendInk @Composable @ReadOnlyComposable get() = of(Swatches.sendInk)
  val lineStrong @Composable @ReadOnlyComposable get() = of(Swatches.lineStrong)
  val active @Composable @ReadOnlyComposable get() = of(Swatches.active)
  val ok @Composable @ReadOnlyComposable get() = of(Swatches.ok)
  val warn @Composable @ReadOnlyComposable get() = of(Swatches.warn)
  val danger @Composable @ReadOnlyComposable get() = of(Swatches.danger)
  val brand @Composable @ReadOnlyComposable get() = of(Swatches.brand)
  val command @Composable @ReadOnlyComposable get() = of(Swatches.command)
  val shimmerBase @Composable @ReadOnlyComposable get() = of(Swatches.shimmerBase)
  val diffAdd @Composable @ReadOnlyComposable get() = of(Swatches.diffAdd)
  val diffDel @Composable @ReadOnlyComposable get() = of(Swatches.diffDel)
  val rail @Composable @ReadOnlyComposable get() = of(Swatches.railColor)
  val card @Composable @ReadOnlyComposable get() = of(Swatches.card)
}

/** Spacing, control and slot sizes (airy density, the phone's reading sizes), `--name` → `Metrics.name` */
object Metrics {
  val row = 40.dp
  val rowDense = 32.dp
  val chatRow = 36.dp
  val hairline = 1.dp
  val processRow = 32.dp
  val processProseGap = 12.dp
  val ctl = 40.dp
  val ctlSm = 36.dp
  val lead = 24.dp
  val icon = 18.dp
  val iconCtl = 20.dp
  val chevron = 14.dp
  val pad = 16.dp
  val page = 24.dp
  val gap = 12.dp
  val gapHalf = 6.dp
  val msgGap = 32.dp
  val s0 = 0.dp
  val s0_5 = 2.dp
  val s1 = 4.dp
  val s1_5 = 6.dp
  val s2 = 8.dp
  val s3 = 12.dp
  val s6 = 24.dp
  val rXs = 3.dp
  val rSm = 6.dp
  val rMd = 8.dp
  val rLg = 12.dp
  /** A right-aligned prompt bubble's corners, and the room it always leaves on its left */
  val rBubble = 20.dp
  val bubbleInset = 72.dp
  /** The thread's inset on top of the messages' own `pad` */
  val threadInset = 4.dp
  /** The composer card's corners */
  val rComposer = 26.dp
  val rCardRound = 20.dp
  /** A bare agent mark leading a session row */
  val rowMark = 18.dp
  // A settings row (the system Form row on iOS)
  val formRowHeight = 52.dp
  /** A session in the phone's list: two lines beside the agent's mark */
  val sessionRow = 68.dp
  val groupCaption = 36.dp
  val stateSlot = 18.dp
  val stateRing = 15.dp
  val stateDot = 9.dp
  val unreadDot = 7.dp
  /** The phone composer's thumb-sized controls */
  val mCtl = 36.dp
  /** The bar's floating controls (Material's minimum target) */
  val barCtl = 48.dp
  /** A code card's height cap in the transcript */
  val codeOutputMax = 176.dp
  /** The floating bar pieces' and the toast's shadows */
  val barShadow = 3.dp
  val toastShadow = 6.dp
  val heroTile = 64.dp
  val heroBare = 38.dp
  val contentW = 720.dp
  /** A settings row's online dot */
  val rowDot = 8.dp
  // Pairing (apps/ios Pairing/PairingTokens.swift)
  /** The connect frame's glyph and the slot it keeps */
  val pairGlyph = 52.dp
  val pairGlyphBox = 64.dp
  /** The row background cut around a settings row's online dot */
  val pairDotCut = 2.dp
  /** The frame's sentence keeps two lines, so a phase with a longer one moves nothing under it */
  val pairLineSlot = 52.dp
  /** The camera card, its viewfinder and the finder's corners */
  val pairScanCard = 340.dp
  val pairFinder = 176.dp
  val pairFinderCorner = 30.dp
  val pairFinderRadius = 22.dp
  val pairFinderStroke = 4.dp
  /** A digit's box on the code page, its border, the gap between the two groups of three, and the caret */
  val pairCodeBox = 48.dp
  val pairCodeBoxH = 60.dp
  val pairCodeLine = 1.5.dp
  val pairCodeSplit = 20.dp
  val pairCaret = 2.dp
  val pairCaretH = 30.dp
  /** The code page's status line keeps two lines */
  val pairStatusSlot = 40.dp
  /** The path the wire travels, in the sentence's place once it is up (ui/Button ChipTag) */
  val pairPathX = 6.dp
  val pairPathH = 22.dp
  /** The connect frame's glyph centre sits this far down the page: its optical centre, a little above the middle */
  const val heroRise = 0.42f
}

/** The phone type scale: ui 16 / code 13, as the iOS app and the web reference page */
object Type {
  val ui = 16.sp
  val code = 13.sp
  val text1 = 17.sp
  val text1Line = 27.sp
  val text2 = 16.sp
  val text2Line = 22.sp
  val text3 = 15.sp
  val text3Line = 20.sp
  val text4 = 13.sp
  val text4Line = 18.sp
  val body = 17.sp
  val bodyLine = 26.sp
  val barTitle = 22.sp
  val mono = 14.sp
  val monoLine = 21.sp
  val codeOutput = 13.sp
  val codeOutputLine = 20.sp
  /** The computer's name under the connect frame's glyph */
  val pairName = 22.sp
  val pairNameLine = 28.sp
  /** A digit in a code box */
  val pairDigit = 28.sp
}
