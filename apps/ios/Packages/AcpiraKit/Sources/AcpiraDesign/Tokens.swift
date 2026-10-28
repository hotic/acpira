import SwiftUI

// The webview's design tokens (packages/ui/styles/tokens.css) at the baseline appearance the app ships:
// density airy, radius 12, surface tonal, font Inter, accent agent. Components read only these names, never raw numbers,
// the same rule AGENTS.md sets for the webview. The phone type scale is ui 16 / code 13 (the web reference page uses the same).
public enum Tokens {
  /// acpira.uiFontSize / acpira.codeFontSize on the phone
  public static let uiFont: CGFloat = 16
  public static let codeFont: CGFloat = 13
}

// MARK: - Colour

/// A plain sRGB colour with alpha, so mixes (`color-mix(in srgb, …)`) can be computed the way CSS does
public struct RGBA: Hashable, Sendable {
  public var r: Double, g: Double, b: Double, a: Double
  public init(_ r: Double, _ g: Double, _ b: Double, _ a: Double = 1) { self.r = r; self.g = g; self.b = b; self.a = a }
  /// `#rrggbb`
  public init(hex: UInt32, alpha: Double = 1) {
    self.init(Double((hex >> 16) & 0xff) / 255, Double((hex >> 8) & 0xff) / 255, Double(hex & 0xff) / 255, alpha)
  }
  public static func white(_ a: Double) -> RGBA { RGBA(1, 1, 1, a) }
  public static func black(_ a: Double) -> RGBA { RGBA(0, 0, 0, a) }
  /// `color-mix(in srgb, self p, other)`: p is self's share
  public func mix(_ other: RGBA, _ p: Double) -> RGBA {
    RGBA(r * p + other.r * (1 - p), g * p + other.g * (1 - p), b * p + other.b * (1 - p), a * p + other.a * (1 - p))
  }
}

#if canImport(UIKit)
import UIKit
extension RGBA {
  var platform: UIColor { UIColor(red: r, green: g, blue: b, alpha: a) }
}
#elseif canImport(AppKit)
import AppKit
extension RGBA {
  var platform: NSColor { NSColor(srgbRed: r, green: g, blue: b, alpha: a) }
}
#endif

extension Color {
  /// A colour that follows the environment's colour scheme, like a token redefined under [data-theme="light"]
  public init(dark: RGBA, light: RGBA) {
    #if canImport(UIKit)
    self.init(uiColor: UIColor { $0.userInterfaceStyle == .dark ? dark.platform : light.platform })
    #elseif canImport(AppKit)
    self.init(nsColor: NSColor(name: nil) { $0.bestMatch(from: [.darkAqua, .aqua]) == .darkAqua ? dark.platform : light.platform })
    #endif
  }
}

/// One token's dark and light values; `color` is the dynamic SwiftUI colour
public struct Swatch: Sendable {
  public let dark: RGBA
  public let light: RGBA
  public init(_ dark: RGBA, _ light: RGBA) { self.dark = dark; self.light = light }
  public init(_ both: RGBA) { self.init(both, both) }
  public var color: Color { Color(dark: dark, light: light) }
  public func mix(_ other: Swatch, _ p: Double) -> Swatch { Swatch(dark.mix(other.dark, p), light.mix(other.light, p)) }
}

/// Each colour token's dark / light pair, `--name` in tokens.css → `Swatches.name`
public enum Swatches {
  // Text levels
  public static let fg1 = Swatch(RGBA(hex: 0xededed), RGBA(hex: 0x0d0d0d))
  public static let fg2 = Swatch(RGBA(202 / 255, 202 / 255, 202 / 255, 0.7), RGBA(hex: 0x6c6c6c))
  public static let fg3 = Swatch(RGBA(hex: 0x767676))
  /// `composer-field` redefines --fg-2 as color-mix(--input-fg 78%, --input-bg): fg-1 over the translucent field
  /// (Chrome's computed values, LAB)
  public static let cmpFg2 = Swatch(RGBA(0.930184, 0.930184, 0.930184, 0.788627), RGBA(0.0613625, 0.0613625, 0.0613625, 0.788627))
  public static let fgStrong = Swatch(RGBA(hex: 0xf5f5f5), RGBA(hex: 0x000000))
  public static let actionFg = Swatch(RGBA(hex: 0x858585), RGBA(hex: 0x707070))
  // Surfaces
  /// The phone departs from --bg-0 in dark: list, chat and settings share one near-black page (Grok's settings grey),
  /// picked 2026-10-27; the web's #181818 read as a grey wash and true black as too dark
  public static let bg0 = Swatch(RGBA(hex: 0x111111), RGBA(hex: 0xf8f8f8))
  public static let bg1 = Swatch(RGBA(hex: 0x1f1f1f), RGBA(hex: 0xffffff))
  public static let bg2 = Swatch(RGBA(hex: 0x262626), RGBA(hex: 0xf1f1f1))
  public static let bgCode = Swatch(RGBA(hex: 0x1f1f1f), RGBA(hex: 0xefefef))
  // Lines and fills
  public static let line = Swatch(.white(0.06), .black(0.06))
  public static let conversationLine = Swatch(.white(0.12), .black(0.10))
  public static let conversationLineFocus = Swatch(.white(0.20), .black(0.18))
  public static let chip = Swatch(.white(0.07), RGBA(hex: 0xf4f4f4))
  public static let chipHover = Swatch(.white(0.1), RGBA(hex: 0xf1f1f1))
  public static let cmpBg = Swatch(.white(0.04), .white(0.04))
  /// A settings form's page: the same page as list and chat in dark, the system's grouped grey in light so the white
  /// rows read
  public static let formPage = Swatch(RGBA(hex: 0x111111), RGBA(hex: 0xf2f2f7))
  /// A system Form's row on the form page (a step above it in dark, white in light)
  public static let formRow = Swatch(RGBA(hex: 0x242424), RGBA(hex: 0xffffff))
  /// The dim behind an open liquid picker, as Messages dims its page under the `+` menu; dark enough that the
  /// captions beside the pieces read over the transcript
  public static let gooDim = Swatch(.black(0.62), .black(0.18))
  /// Under a liquid picker's panel: the page's colour, so its clear glass keeps the rim but the content behind it no
  /// longer reads through the rows
  public static let gooBacking = bg0.alpha(0.86)
  /// The prompt bubble, a step darker than --chip in light so it reads on the light page
  public static let bubbleBg = Swatch(.white(0.08), RGBA(hex: 0xebebeb))
  public static let btn1Bg = Swatch(.white(0.95), RGBA(hex: 0x151515))
  public static let btn1Fg = Swatch(RGBA(hex: 0x151515), RGBA(hex: 0xffffff))
  public static let sendBg = Swatch(.white(0.04), .black(0.04))
  public static let sendInk = Swatch(.white(0.5), .black(0.5))
  public static let sendRing = Swatch(.white(0.02), .black(0.04))
  /// --send-ring's second shadow: a 1 px inset highlight along the top edge (dark only)
  public static let sendHighlight = Swatch(.white(0.04), .black(0))
  public static let lineStrong = Swatch(RGBA(hex: 0x3c3c3c), RGBA(hex: 0xcecece))
  public static let hover = Swatch(.white(0.05), .black(0.04))
  public static let active = Swatch(.white(0.08), .black(0.07))
  public static let focus = Swatch(RGBA(hex: 0x0078d4), RGBA(hex: 0x005fb8))
  public static let scrim = Swatch(.black(0.72), .black(0.45))
  // Status
  public static let ok = Swatch(RGBA(hex: 0x439e7e), RGBA(hex: 0x2f7a5b))
  public static let warn = Swatch(RGBA(hex: 0xd29922), RGBA(hex: 0x9a6700))
  public static let danger = Swatch(RGBA(hex: 0xf85149), RGBA(hex: 0xcf222e))
  public static let fast = Swatch(RGBA(hex: 0xf0b440), RGBA(hex: 0xb7791f))
  public static let ultra = Swatch(RGBA(hex: 0xb3a2f5), RGBA(hex: 0x6b52c9))
  // Accents
  public static let brand = Swatch(RGBA(hex: 0xe6a63e), RGBA(hex: 0xb7791f))
  public static let brandFg = Swatch(RGBA(hex: 0x1b1300), RGBA(hex: 0xffffff))
  public static let command = Swatch(RGBA(hex: 0x6ea8fe), RGBA(hex: 0x2f6fe0))
  public static let summon = Swatch(RGBA(hex: 0xd2a8ff), RGBA(hex: 0x8250df))
  public static let agentDevin = Swatch(RGBA(hex: 0x6ea8fe), RGBA(hex: 0x2f6fe0))
  public static let agentGrok = Swatch(RGBA(hex: 0xe8e8e8), RGBA(hex: 0x1a1a1a))
  public static let shimmerBase = Swatch(RGBA(hex: 0x9a9a9a), RGBA(hex: 0x707070))
  // Derived mixes
  public static let diffAdd = ok.mix(bgCode, 0.06)
  public static let diffDel = danger.mix(bgCode, 0.08)
  public static let railColor = fg3.mix(bg1, 0.65)
  /// `action-details` redefines --rail-color as the action ink half into the page
  public static let railAction = actionFg.mix(bg0, 0.5)
  public static let quotaOk = ok.mix(bg2, 0.65)
  public static let quotaWarn = warn.mix(bg2, 0.75)
  public static let quotaDanger = danger.mix(bg2, 0.7)
  // Popup surface: `.popup-surface[data-palette="menu"]` over --menu-bg = --bg-1. The surface redefines the text levels
  // as mixes of its own foreground into its background, and the tonal states as that foreground at low alpha
  public static let menuBg = bg1
  /// --menu-border and --menu-separator: the page's --line, resolved on the shell
  public static let menuLine = line
  public static let popFg2 = fg1.mix(bg1, 0.78)
  public static let popFg3 = fg1.mix(bg1, 0.64)
  public static let popHover = fg1.alpha(0.06)
  public static let popActive = fg1.alpha(0.11)
  /// --shadow: the two layers of the pop shadow (a wide drop and a contact line)
  public static let shadowWide = Swatch(.black(0.45), .black(0.12))
  public static let shadowContact = Swatch(.black(0.4), .black(0.08))
}

extension Swatch {
  /// The same colour at another alpha (`color-mix(in srgb, c p%, transparent)`)
  public func alpha(_ a: Double) -> Swatch {
    Swatch(RGBA(dark.r, dark.g, dark.b, dark.a * a), RGBA(light.r, light.g, light.b, light.a * a))
  }
}

/// Colour roles as dynamic colours, `--name` in tokens.css → `Palette.name`
public enum Palette {
  public static var fg1: Color { Swatches.fg1.color }
  public static var fg2: Color { Swatches.fg2.color }
  public static var fg3: Color { Swatches.fg3.color }
  public static var fgStrong: Color { Swatches.fgStrong.color }
  public static var actionFg: Color { Swatches.actionFg.color }
  public static var bg0: Color { Swatches.bg0.color }
  public static var bg1: Color { Swatches.bg1.color }
  public static var bg2: Color { Swatches.bg2.color }
  public static var bgCode: Color { Swatches.bgCode.color }
  public static var line: Color { Swatches.line.color }
  public static var conversationLine: Color { Swatches.conversationLine.color }
  public static var conversationLineFocus: Color { Swatches.conversationLineFocus.color }
  public static var chip: Color { Swatches.chip.color }
  public static var chipHover: Color { Swatches.chipHover.color }
  public static var cmpBg: Color { Swatches.cmpBg.color }
  public static var cmpFg2: Color { Swatches.cmpFg2.color }
  public static var bubbleBg: Color { Swatches.bubbleBg.color }
  public static var formPage: Color { Swatches.formPage.color }
  public static var formRow: Color { Swatches.formRow.color }
  public static var gooDim: Color { Swatches.gooDim.color }
  public static var gooBacking: Color { Swatches.gooBacking.color }
  public static var btn1Bg: Color { Swatches.btn1Bg.color }
  public static var btn1Fg: Color { Swatches.btn1Fg.color }
  public static var sendBg: Color { Swatches.sendBg.color }
  public static var sendInk: Color { Swatches.sendInk.color }
  public static var sendRing: Color { Swatches.sendRing.color }
  public static var sendHighlight: Color { Swatches.sendHighlight.color }
  public static var lineStrong: Color { Swatches.lineStrong.color }
  public static var hover: Color { Swatches.hover.color }
  public static var active: Color { Swatches.active.color }
  public static var focus: Color { Swatches.focus.color }
  public static var scrim: Color { Swatches.scrim.color }
  public static var ok: Color { Swatches.ok.color }
  public static var warn: Color { Swatches.warn.color }
  public static var danger: Color { Swatches.danger.color }
  public static var fast: Color { Swatches.fast.color }
  public static var ultra: Color { Swatches.ultra.color }
  public static var brand: Color { Swatches.brand.color }
  public static var command: Color { Swatches.command.color }
  public static var summon: Color { Swatches.summon.color }
  public static var shimmerBase: Color { Swatches.shimmerBase.color }
  public static var diffAdd: Color { Swatches.diffAdd.color }
  public static var diffDel: Color { Swatches.diffDel.color }
  public static var rail: Color { Swatches.railColor.color }
  public static var railAction: Color { Swatches.railAction.color }
  public static var quotaOk: Color { Swatches.quotaOk.color }
  public static var quotaWarn: Color { Swatches.quotaWarn.color }
  public static var quotaDanger: Color { Swatches.quotaDanger.color }
  public static var menuBg: Color { Swatches.menuBg.color }
  public static var menuLine: Color { Swatches.menuLine.color }
  public static var popFg2: Color { Swatches.popFg2.color }
  public static var popFg3: Color { Swatches.popFg3.color }
  public static var popHover: Color { Swatches.popHover.color }
  public static var popActive: Color { Swatches.popActive.color }
  public static var shadowWide: Color { Swatches.shadowWide.color }
  public static var shadowContact: Color { Swatches.shadowContact.color }
  /// Card surface (surface = tonal): fill and outline
  public static var cardBg: Color { bg2 }
  public static var cardLine: Color { conversationLine }

  /// `--accent` under accent = agent: Devin and Grok have their own colour, every other agent takes the brand
  public static func accent(for agent: String) -> Color {
    switch agent {
    case "devin": Swatches.agentDevin.color
    case "grok": Swatches.agentGrok.color
    default: brand
    }
  }
}

// MARK: - Sizes

/// Spacing, control and slot sizes (airy density), `--name` → `Metrics.name`. Prose, rows, glyphs and controls take
/// the phone's reading sizes (17 pt prose, 14 pt code, 18 pt glyphs, rows a thumb apart), not the desktop's 16 / 13 / 14
public enum Metrics {
  public static let row: CGFloat = 40
  public static let rowDense: CGFloat = 32
  public static let chatRow: CGFloat = 36
  /// A 1 px CSS border: hairlines that take layout space (table rules, framed boxes)
  public static let hairline: CGFloat = 1
  public static let processRow: CGFloat = 32
  public static let processRowGap: CGFloat = 2
  public static let processProseGap: CGFloat = 12
  public static let ctl: CGFloat = 40
  public static let ctlSm: CGFloat = 36
  public static let lead: CGFloat = 24
  public static let icon: CGFloat = 18
  public static let iconCtl: CGFloat = 20
  /// size-2.5: the bolt inside a Fast badge
  public static let fastBadge: CGFloat = 10
  /// Tailwind size-3: the fold chevron
  public static let chevron: CGFloat = 14
  public static let pad: CGFloat = 16
  public static let padY: CGFloat = 22
  public static let page: CGFloat = 24
  public static let gap: CGFloat = 12
  public static let gapHalf: CGFloat = 6
  public static let msgGap: CGFloat = 32
  public static let dockGap: CGFloat = 8
  public static let thumb: CGFloat = 56
  public static let agentImage: CGFloat = 240
  /// An HTML widget (tokens.css --widget-placeholder / --widget-max): the room held before its page reports a height,
  /// and the height past which the page scrolls inside itself
  public static let widgetPlaceholder: CGFloat = 120
  public static let widgetMax: CGFloat = 900
  public static let hit: CGFloat = 6
  public static var indent: CGFloat { lead + gap }
  public static let railWidth: CGFloat = 1
  public static let railDot: CGFloat = 4
  /// --rail-fade: the line's top end fades in over this length below its icon
  public static let railFade: CGFloat = 14
  public static let scrollFade: CGFloat = 28
  public static let unreadDot: CGFloat = 7
  /// The session list's state marks (SessionList.tsx StateMark): one slot for every mark so they share a centre line,
  /// the ring drawn smaller than the slot. About half again the desktop's sizes, since the marks stand there without a
  /// label
  public static let stateSlot: CGFloat = 18
  public static let stateRing: CGFloat = 15
  public static let stateDot: CGFloat = 9
  public static let stateUnread: CGFloat = 10
  /// Background: the dot orbiting the still ring, and the dot inside it once a reply came in meanwhile
  public static let stateOrbit: CGFloat = 4.5
  public static let stateInner: CGFloat = 6
  // Tailwind's 4 px steps the components use (`gap-0.5`, `pt-1`, `pb-1.5`, `gap-2` …)
  public static let s0_5: CGFloat = 2
  public static let s1: CGFloat = 4
  public static let s1_5: CGFloat = 6
  public static let s2: CGFloat = 8
  public static let s2_5: CGFloat = 10
  public static let s3: CGFloat = 12
  public static let s6: CGFloat = 24
  /// --quota-track: the quota tube's height
  public static let quotaTrack: CGFloat = 7
  // Command card
  public static let commandX: CGFloat = pad - gap / 2
  public static let commandY: CGFloat = gap * 0.75
  /// An open code card (command output, diff) sits this far below its row and above the next one, the process rows'
  /// own gap included, so it reads as centred between them; the web's --command-after left 11 pt under a card and 6
  /// over it
  public static let codeCardGap: CGFloat = gapHalf
  public static var codeCardAfter: CGFloat { codeCardGap - processRowGap }
  public static let commandPromptLines = 3
  /// A code card's height cap in the transcript; on the phone a short preview of about eight lines, the whole card
  /// one tap away in a sheet
  public static let codeOutputMax: CGFloat = 176
  public static let codeOutputLine: CGFloat = 20
  public static let termLines = 12
  // Diff
  public static let diffHunk: CGFloat = gap * 1.25
  // Radii (radius axis 12)
  public static let rXs: CGFloat = 3
  public static let rSm: CGFloat = 6
  public static let rMd: CGFloat = 8
  public static let rLg: CGFloat = 12
  // Questions / plans
  public static let questionSectionGap: CGFloat = 6
  public static let questionTopPad: CGFloat = 12
  public static let questionOptionPad: CGFloat = 4
  public static let questionDescriptionGap: CGFloat = 2
  public static let planPadY: CGFloat = 6
  public static let planSectionGap: CGFloat = 4
  public static let planEntryPadY: CGFloat = 2
  /// max-h-plan: the docked list scrolls past seven dense rows
  public static var planMax: CGFloat { 7 * rowDense }
  /// A popover's width (the stats card)
  public static let popLg: CGFloat = 300
  // The phone composer (iOS only: the webview has no such layout). The desktop card's radius, controls sized for a
  // thumb: 36 pt (Apple's minimum comfortable target less the card's own inset)
  public static let mCtl: CGFloat = 36
  /// How far the system's bottom bar sinks into the home indicator's safe area (the list's search bar: 28 pt off the
  /// screen's edge over a 34 pt inset, iOS 27.1 simulator). Every screen's bottom bar (the chat's composer, the new
  /// session's composer) rests on the same floor, so moving between screens never shifts it
  public static let barSink: CGFloat = 6
  /// A list row a thumb finds easily (empty list, project header, "N more")
  public static let sheetRow: CGFloat = 52
  /// A session in the phone's list: two lines beside the agent's mark with room above and below
  public static let sessionRow: CGFloat = 68
  /// The progressive blur over a scrolled page's top (Design/TopFade.swift): how far it reaches past the bar's
  /// buttons before it is gone
  public static let topFadeTail: CGFloat = 28
  /// The new-session screen's centre piece: the agent's bare mark (no tile) at `heroBare` in a `heroTile` slot; the
  /// hero's centre sits `heroRise` down the page, its optical centre a little above the middle
  public static let heroTile: CGFloat = 64
  public static let heroBare: CGFloat = 38
  public static let heroRise: CGFloat = 0.42
  /// The agent ring (new session): the other agents as glass bubbles on a circle around the bare mark. The radius
  /// grows with the count so neighbouring centres stay `ringPitch` apart along the ring (captions clear each other),
  /// but never under `ringRadiusMin`, which keeps a bubble just outside the merge distance of the centre's drop
  public static let ringBubble: CGFloat = 48
  public static let ringMark: CGFloat = 22
  public static let ringRadiusMin: CGFloat = 96
  public static let ringPitch: CGFloat = 104
  // iOS 26 glass controls and the phone-native shapes
  /// A floating glass button: Apple's 44 pt target
  public static let glassCtl: CGFloat = 44
  public static let glassIcon: CGFloat = 18
  /// A right-aligned prompt bubble's corners, and the room it always leaves on its left
  public static let rBubble: CGFloat = 20
  public static let bubbleInset: CGFloat = 48
  /// The composer card's corners while typing
  public static let rComposer: CGFloat = 26
  /// A card (alert, question): rounder than the desktop's, inside the composer's
  public static let rCardRound: CGFloat = 20
  /// The session list's large title row and the gap under it
  public static let largeTitle: CGFloat = 52
  /// A bare agent mark leading a session row, and the state dot of the engine's status row
  public static let rowMark: CGFloat = 18
  public static let rowDot: CGFloat = 8
  /// A caption over a group of sessions (pinned, today, earlier)
  public static let groupCaption: CGFloat = 36
  // Liquid pickers (AcpiraUI/Design/Goo.swift): Liquid Glass pieces in one container
  /// The room between two resting pieces: wider than `gooMerge`, so they part once settled
  public static let gooGap: CGFloat = 12
  /// How close two glass pieces come before they melt together (the composer's glass container): under the resting
  /// gap, so a bean draws a neck as it leaves the one before and stands apart once settled
  public static let gooMerge: CGFloat = 10
  /// How far a chip's glass circle sits inside its slot. Past `gooMerge`, so the bar's container never sees the
  /// circle's outline next to the bar's own: a full-size circle on the bar's round end (or on the card's corner, or
  /// touching the next chip) melted into it and swelled the end into a head with a neck behind it
  public static let gooChipInset: CGFloat = 12
  /// The dim behind an open picker: a square wider and taller than any screen, centred on the composer
  public static let gooScrim: CGFloat = 3000
  /// A panel row's inset
  public static let gooPadX: CGFloat = 14
  /// The scale a chip's glyph swaps from (the model's mark and ×)
  public static let gooSeed: CGFloat = 0.35
  /// An account's avatar in the system account sheet (a list row's leading image)
  public static let avatar: CGFloat = 44
  /// The system navigation bar's row under the status bar, with the gap a toast keeps below it
  public static let navBar: CGFloat = 52
  /// The one-line composer: `+`, the field's capsule and the model button, all this tall, with
  /// glyphs this large (Messages and ChatGPT's bar)
  public static let barCtl: CGFloat = 48
  /// How far the composer's field sits below its line box's centre: the CSS line box keeps room under the baseline
  /// that most glyphs leave empty, so a centred line read about this much high in the bar, in Chinese and English alike
  public static let fieldInkDrop: CGFloat = 2
  public static let barIcon: CGFloat = 22
  /// The list a bean opens: a rounded panel beside the bean column, rows of the composer's control height
  public static let gooPanelW: CGFloat = 232
  public static let gooRow: CGFloat = 40
  /// Rows shown before the panel scrolls; the half row shows there is more
  public static let gooPanelRows: CGFloat = 6.5
  public static let rGooPanel: CGFloat = 22
  /// The room a bean's caption may take beside it
  public static let gooAside: CGFloat = 240
  /// The user card's text cap: five body lines
  public static var userMessageMax: CGFloat { 5 * TypeScale.text1.lineHeight }
}

// MARK: - Glass

/// The Liquid Glass variant of every pressable control (composer bar, its chips and beans, the new-session ring, the
/// glass header and list buttons): `.clear`, the see-through one, picked 2026-10-27 over `.regular`, whose press read
/// as a flat grey disc. Glass pieces that melt into each other must share one variant: a `.clear` chip on a `.regular`
/// bar no longer merged and drew pinched necks between them. The toast carries text over content and keeps `.regular`
public enum GlassStyle {
  public static let control: Glass = .clear
}

// MARK: - Motion

/// Durations and curves (motion.css / tokens.css)
public enum Motion {
  public static let dur: Double = 0.150
  /// Liquid pickers: the spring that pulls the pieces out and back, and each piece's lag behind the one before
  public static let gooSpring = Animation.spring(response: 0.7, dampingFraction: 0.76)
  public static let gooStagger: CGFloat = 0.18
  /// The longest lag the last piece may have, however many pieces there are
  public static let gooStaggerMax: CGFloat = 0.45
  /// A chip's glyph swapping for × as its pieces leave, and back as they land
  public static let gooSwap = Animation.easeOut(duration: 0.18)
  public static let gooSwapBack = Animation.easeOut(duration: 0.22).delay(0.22)
  /// The dim lifting behind a closing picker: held while the pieces go home
  public static let gooDimOut = Animation.easeIn(duration: 0.42)
  /// The agent ring moves at the liquid pickers' pace: their spring, and each bubble's lag behind the one before
  /// (clockwise out, the last one first on the way home, at half the lag). A 0.46 s spring with a 0.03 s lag put the
  /// whole ring out in a quarter of a second and read as a pop with no transition, as the beans once did
  public static let ringSpring = gooSpring
  public static func ringStagger(_ count: Int) -> Double {
    Double(min(gooStagger, gooStaggerMax / CGFloat(max(count - 1, 1))))
  }
  /// A bubble shows once it clears the mark's drop and is gone before it slides back in
  public static let ringShow = Animation.easeOut(duration: 0.22)
  public static let ringShowDelay: Double = 0.12
  public static let ringHide = Animation.easeIn(duration: 0.12)
  public static let ringHideDelay: Double = 0.22
  /// The bubbles' captions: in after the bubble, out first
  public static let ringCaption = Animation.easeOut(duration: 0.25)
  public static let ringCaptionDelay: Double = 0.3
  public static let ringCaptionOut = Animation.easeIn(duration: 0.1)
  /// The name and folder under the mark: away as the ring opens, back as a picked mark lands or, after a plain close,
  /// once the bubbles are home (`ringHeroHold`)
  public static let ringHeroAway = Animation.easeOut(duration: 0.2)
  public static let ringHeroBack = Animation.spring(response: 0.45, dampingFraction: 0.85)
  public static let ringHeroHold: Double = 0.45
  /// A pick: the bubble's flight into the drop (critically damped, so it lands without overshoot and the swap to the
  /// centre's own mark is invisible), how long until the flown mark gives way to the centre's, and how far into the
  /// flight the agent changes and the name comes back
  public static let ringFlight = Animation.spring(response: 0.42, dampingFraction: 1)
  public static let ringFlightTime: Double = 0.5
  public static let ringLand: Double = 0.28
  /// How long a toast stays up (the phone's confirmation of copy, pin, export …)
  public static let toast: Double = 1.8
  /// A toast offering undo stays long enough to reach for it
  public static let toastUndo: Double = 6
  public static let durOpen: Double = 0.220
  public static let durClose: Double = 0.320
  public static let enter: Double = 0.300
  public static let enterStagger: Double = 0.040
  public static let enterY: CGFloat = 4
  public static let rowEnter: Double = 0.460
  public static let rowEnterPart: Double = 0.065
  public static let rowEnterFile: Double = 0.180
  public static let streamChar: Double = 0.180
  public static let shimmerCycle: Double = 2
  public static let railGrow: Double = 0.220
  /// The fold dwell before a finished item closes on its own (foldStates DWELL_MS)
  public static let foldDwell: Double = 0.900
  /// The outer fold's close and a frame or two (Turns.tsx SETTLE_MS)
  public static let foldSettle: Double = 0.400
  /// The subagent mark's bead loop (subagents.css `subagent-mark-*`, 1 s) and each bar's lead on the next one
  public static let subagentLoop: Double = 1
  public static let subagentBarStagger: Double = 0.33
  /// A child finishing in front of the reader: the bars gather onto the ✓ and it draws (icons.tsx playFinish)
  public static let subagentFinish: Double = 1

  /// A pressed send button: the webview's hover `brightness(1.08)`, and a slight sink
  public static let pressBrightness: Double = 0.08
  public static let pressScale: CGFloat = 0.94

  /// A fold: a spring without bounce, as the system's own disclosures move; the web's 0.22 s ease-out read as a snap
  /// at the phone's sizes
  public static let foldPhone: Double = 0.380
  /// Opening: the fold spring
  public static var open: Animation { .smooth(duration: foldPhone) }
  /// Closing: the fold spring
  public static var close: Animation { .smooth(duration: foldPhone) }
  public static var quick: Animation { .easeOut(duration: dur) }
  /// Press feedback: a quick spring
  public static var press: Animation { .spring(response: 0.22, dampingFraction: 0.7) }
  /// `transition: … var(--dur) ease`: CSS `ease` is cubic-bezier(.25, .1, .25, 1)
  public static var ease: Animation { .timingCurve(0.25, 0.1, 0.25, 1, duration: dur) }
}
