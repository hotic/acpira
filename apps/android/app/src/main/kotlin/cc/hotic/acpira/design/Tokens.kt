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
  /** The system red of a swipe's Delete (UIKit's systemRed, as the iOS app's swipe action) */
  val swipeDelete = Swatch(hex(0xff453a), hex(0xff3b30))
  val ok = Swatch(hex(0x439e7e), hex(0x2f7a5b))
  val warn = Swatch(hex(0xd29922), hex(0x9a6700))
  val danger = Swatch(hex(0xf85149), hex(0xcf222e))
  val brand = Swatch(hex(0xe6a63e), hex(0xb7791f))
  val command = Swatch(hex(0x6ea8fe), hex(0x2f6fe0))
  /** A summoned persona's mark in a prompt (--summon) */
  val summon = Swatch(hex(0xd2a8ff), hex(0x8250df))
  val shimmerBase = Swatch(hex(0x9a9a9a), hex(0x707070))
  val diffAdd = ok.mix(bgCode, 0.06f)
  val diffDel = danger.mix(bgCode, 0.08f)
  val railColor = fg3.mix(bg1, 0.65f)
  /** A card (alert, permission, plan): white on the light page, a step up in dark */
  val card = Swatch(hex(0x1c1c1c), hex(0xffffff))
  /** A menu (the iOS app's system menus): a step lighter than any page or card under it, hairline-edged */
  val menuBg = Swatch(hex(0x2a2a2a), hex(0xffffff))
  val menuLine = Swatch(white(0.08f), black(0.06f))
  /** The band between a menu's sections, as UIMenu's thick separator */
  val menuGap = Swatch(black(0.22f), black(0.05f))
  /** The page under an open composer picker, and a panel's backing over it (Goo.swift) */
  val gooDim = Swatch(black(0.62f), black(0.18f))
  /** Fast's bolt while it is on */
  val fast = Swatch(hex(0xf0b440), hex(0xb7791f))
  /** The composer's hint, as the list's search field sets its own */
  val barHint = Swatch(white(0.4f), black(0.42f))
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
  val hover @Composable @ReadOnlyComposable get() = of(Swatches.hover)
  val swipeDelete @Composable @ReadOnlyComposable get() = of(Swatches.swipeDelete)
  val ok @Composable @ReadOnlyComposable get() = of(Swatches.ok)
  val warn @Composable @ReadOnlyComposable get() = of(Swatches.warn)
  val danger @Composable @ReadOnlyComposable get() = of(Swatches.danger)
  val brand @Composable @ReadOnlyComposable get() = of(Swatches.brand)
  val command @Composable @ReadOnlyComposable get() = of(Swatches.command)
  val summon @Composable @ReadOnlyComposable get() = of(Swatches.summon)
  val shimmerBase @Composable @ReadOnlyComposable get() = of(Swatches.shimmerBase)
  val diffAdd @Composable @ReadOnlyComposable get() = of(Swatches.diffAdd)
  val diffDel @Composable @ReadOnlyComposable get() = of(Swatches.diffDel)
  val rail @Composable @ReadOnlyComposable get() = of(Swatches.railColor)
  val card @Composable @ReadOnlyComposable get() = of(Swatches.card)
  val scrim @Composable @ReadOnlyComposable get() = of(Swatches.scrim)
  val menuBg @Composable @ReadOnlyComposable get() = of(Swatches.menuBg)
  val menuLine @Composable @ReadOnlyComposable get() = of(Swatches.menuLine)
  val menuGap @Composable @ReadOnlyComposable get() = of(Swatches.menuGap)
  val gooDim @Composable @ReadOnlyComposable get() = of(Swatches.gooDim)
  val fast @Composable @ReadOnlyComposable get() = of(Swatches.fast)
  val barHint @Composable @ReadOnlyComposable get() = of(Swatches.barHint)
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
  /** How far the page colour fades into a scrolled page under its top bar (apps/ios Metrics.topFadeTail) */
  val topFadeTail = 28.dp
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
  /** The state marks' unread dot, the background ring's orbiting dot and the dot inside it once a reply came in */
  val stateUnread = 10.dp
  val stateOrbit = 4.5.dp
  val stateInner = 6.dp
  // The session list (apps/ios Sessions/SessionListScreen.swift)
  /** A one-line list row (a project's header, a session inside its project, "N more") */
  val sheetRow = 52.dp
  /** A session's row a step in from its project's folder */
  val nest = 16.dp
  /** How far a list row slides to bare Delete, the capsule and its margins included */
  val swipeAction = 96.dp
  /** A release faster than this (per second) decides a swipe by its direction alone (apps/ios Motion.swipeFlick) */
  val swipeFlick = 300.dp
  /** "N more"'s agent marks */
  val moreMark = 14.dp
  /** A dragged row's card under the finger: its widest */
  val dragPreview = 260.dp
  /** A drag near the list's top or bottom edge scrolls it: the band it starts in, and the speed at the very edge per second */
  val dragEdge = 72.dp
  val dragScrollMax = 1200.dp
  /** A host folder's alias at the header's end, and its connection dot */
  val hostAlias = 120.dp
  val hostDot = 7.dp
  /** A host folder's globe badge, the globe in it and how far it hangs past the folder's box */
  val remoteBadge = 13.dp
  val remoteGlobe = 10.dp
  val remoteOverhang = 5.dp
  /** The phone composer's thumb-sized controls */
  val mCtl = 36.dp
  /** The bar's floating controls (Material's minimum target) */
  val barCtl = 48.dp
  /** A code card's height cap in the transcript */
  val codeOutputMax = 176.dp
  /** A wide popover card (a reply's statistics): three timing tiles at the phone's reading size fit in it */
  val popWide = 336.dp
  /** The quotes' and the context ring's cards (popLg) */
  val popLg = 300.dp
  /** A reply block's preview over its long-press menu keeps this many lines */
  const val messagePreviewLines = 6
  /** The composer's share of the height the keyboard leaves: its card grows to its most lines only where that fits */
  const val composerShare = 0.45f
  /** A prompt mark's pill: its reach past the name's glyphs at most, the clearance kept from a neighbour, its pad */
  val markOverhang = 4.dp
  val markClear = 1.5.dp
  /** The floating bar pieces' and the toast's shadows */
  val barShadow = 3.dp
  /** Two floating pieces closer than this melt into one (apps/ios Metrics.gooMerge): the bar hand-over's neck */
  val gooMerge = 10.dp
  val toastShadow = 6.dp
  val heroTile = 64.dp
  val heroBare = 38.dp
  /**
   * The agent ring (new session, apps/ios Metrics.ring*): the other agents' bubbles round the hero's mark, their marks,
   * the ring's least radius, and the room between neighbouring centres along it (the radius grows with the count)
   */
  val ringBubble = 48.dp
  val ringMark = 22.dp
  val ringRadiusMin = 96.dp
  val ringPitch = 104.dp
  /** The reading column: transcripts, composers and forms hold to it, centred in whatever is wider */
  val contentW = 720.dp
  /** The session list's column beside the detail in a wide window (apps/ios Metrics.sidebarW) */
  val sidebarW = 320.dp
  /**
   * A window at least this wide and tall splits into the list and the page beside it (Material's medium width and
   * height): a phone held sideways is wide but short, so it keeps the stack with the reading column centred
   */
  val splitMinW = 600.dp
  val splitMinH = 480.dp
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
  /** In a wide column (tablets, foldables open) the card grows with its width at this height-to-width share */
  const val pairScanAspect = 0.7f
  /** The form's column on a wide page, in place of a sheet (the first-run flow) */
  val pairColumn = 640.dp
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
  /** The scanned code's box, quiet zone included, its track's corner and stroke (pairGeometry.ts QR_BOX, RING_R, RING_STROKE) */
  val pairCode = 228.dp
  val pairCodeRadius = 16.dp
  val pairCodeTrack = 1.5.dp
  /** The paired moment's blurs: the rings dissolving, the mark leaving, the computer arriving (PairingTokens Motion) */
  val pairBandBlur = 1.6.dp
  val pairMarkBlur = 3.dp
  val pairGlyphBlur = 3.dp
  /** The connect frame's glyph centre sits this far down the page: its optical centre, a little above the middle */
  const val heroRise = 0.42f
  /** A menu: its corner, row, narrowest width, shadow and the gap band between sections (AppMenu) */
  val rMenu = 26.dp
  val menuRow = 44.dp
  val menuMinW = 220.dp
  val menuShadow = 12.dp
  /** The composer's glyphs on the bar (`+`, the model's mark, a bean's) and the chip's × / ‹ */
  val barIcon = 22.dp
  val gooBack = 26.dp
  /** A bean's panel: width, row, rows shown before it scrolls, corner, row inset, and the beans' spacing */
  val gooPanelW = 232.dp
  val gooRow = 40.dp
  const val gooPanelRows = 6.5f
  val rGooPanel = 22.dp
  val gooPadX = 14.dp
  val gooGap = 12.dp
  /** The page's blur under an open picker */
  val gooBlur = 16.dp
  /** The room a bean's caption gets beside it */
  val gooAside = 240.dp
  /** A staged attachment in the composer's card: an image tile (Metrics.thumb), a file chip's widest, the remove mark */
  val stagedTile = 56.dp
  val stagedFileW = 200.dp
  val stagedRemove = 20.dp
  // Media (apps/ios Tokens.swift `thumb`, `agentImage`, `widgetPlaceholder`, `widgetMax`)
  /** A prompt's image thumbnail and a video tile */
  val thumb = 56.dp
  /** Every transcript image card's height (the card's width follows the picture) */
  val agentImage = 240.dp
  /** An HTML widget (tokens.css --widget-placeholder / --widget-max): the room held before its page reports a height, and its cap */
  val widgetPlaceholder = 120.dp
  val widgetMax = 900.dp
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
  /** The composer's field (TypeScale.field): ui + 1 on a ui + 8 line */
  val field = 17.sp
  val fieldLine = 24.sp
  /** A page's own large title (the first-run pairing step's form) */
  val largeTitle = 28.sp
  val largeTitleLine = 34.sp
  /** The first-run intro's headline, at the design page's scale */
  val welcomeHeadline = 34.sp
  val welcomeHeadlineLine = 42.sp
}

/**
 * The first-run flow (apps/ios Welcome/WelcomeFlow.swift `WL`). Every screen shows one composition laid out on a phone's
 * page (upright or on its side) and scaled with every size in it to fit, so a tablet shows the phone's page larger
 */
object WelcomeMetrics {
  /** The notification step: the bell's ring and glyph, its title, the gaps under them, the later button's height */
  val bellRing = 112.dp
  val bell = 34.dp
  val notifyTitle = 28.sp
  val notifyTitleLine = 34.sp
  val notifyTitleGap = 36.dp
  val notifyDescGap = 10.dp
  val laterHeight = 36.dp
  /** The edge light's resting outline, and the light's stroke unit against a dp */
  const val edgeOutlineAlpha = 0.14f
  const val edgeUnit = 1.2f
  val portraitW = 402.dp
  val portraitH = 778.dp
  val landscapeW = 750.dp
  val landscapeH = 381.dp
  /** A screen smaller than the design page shrinks it no further than this */
  const val minScale = 0.8f
  /** Wider than tall by this much: the mark and the copy stand side by side */
  const val sideBySide = 1.15f
  /** On its side with height to spare, the pair sits this share of the spare below the middle */
  const val wideDrop = 0.2f
  /** The copy column beside the mark on a page on its side */
  val wideColumn = 330.dp
  /** Upright, the mark's room above the copy, and how far the copy rises into it */
  val markRoom = 478.dp
  val copyLift = 48.dp
  /** The mark's widest; its width against the room, and its height against a short room */
  val markMax = 300.dp
  const val posterScale = 0.7f
  const val markFill = 0.62f
  /** How far the copy sits in from the button's edge, so it lines up with the capsule's round end by eye */
  val copyInset = 10.dp
  val brandMarkW = 17.dp
  val brandMarkH = 16.dp
  val headlineGap = 18.dp
  val agentsGap = 30.dp
  val agentsRow = 12.dp
  val agentMark = 22.dp
  val agentStep = 20.dp
  val actionsGap = 40.dp
  /** The notification step keeps to a phone's column on a wide screen (the pairing form: Metrics.pairColumn) */
  val column = 480.dp
  /** The outline's thinnest stroke, the finger's spotlight and its outline */
  val outlineMin = 0.75.dp
  val spot = 150.dp
  val spotLine = 1.2.dp
  /** Entrances: a block rises this far, a headline glyph this far, an agent mark slides in this far */
  val rise = 6.dp
  val glyphRise = 7.5.dp
  val slide = 4.dp
  /** The intro stepping back under the pairing step: down this far, blurred this much */
  val swapDrop = 14.dp
  val swapBlur = 10.dp
  /** The solid mark's machined edge: a fine light rim along its top edges, a faint shade along its feet */
  val rimDrop = 1.dp
  val rimBlur = 0.6.dp
  val shadeRise = 2.dp
  val shadeBlur = 3.dp
}
