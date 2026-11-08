// Sizes, spacing, radii and the type scale (apps/ios AcpiraDesign Metrics / Type, apps/android design/Tokens.kt):
// components take every number from here. Lengths are vp, type sizes fp. Colours are resources
// (resources/base|dark/element/color.json), so they follow the system's dark mode.

export class Metrics {
  static readonly row: number = 40;
  static readonly chatRow: number = 36;
  static readonly hairline: number = 1;
  static readonly ctl: number = 40;
  static readonly barCtl: number = 48;
  static readonly lead: number = 24;
  static readonly icon: number = 18;
  static readonly iconCtl: number = 20;
  static readonly rowMark: number = 18;
  static readonly pad: number = 16;
  static readonly page: number = 24;
  static readonly gap: number = 12;
  static readonly gapHalf: number = 6;
  static readonly msgGap: number = 32;
  static readonly s1: number = 4;
  static readonly s2: number = 8;
  static readonly s3: number = 12;
  static readonly s6: number = 24;
  static readonly rSm: number = 6;
  static readonly rMd: number = 8;
  static readonly rLg: number = 12;
  /** A right-aligned prompt bubble's corners, and the room it always leaves on its left */
  static readonly rBubble: number = 20;
  static readonly bubbleInset: number = 72;
  /** The composer card's corners and its send button */
  static readonly rComposer: number = 26;
  static readonly send: number = 36;
  static readonly composerMaxLines: number = 6;
  /** The reading column on wide windows (LayoutRoom.swift readableWidth) */
  static readonly contentW: number = 720;
  /** The sidebar beside the detail column, and the window that earns the split */
  static readonly sidebarW: number = 320;
  static readonly splitMinW: number = 600;
  /** Lucide's stroke on its 24 unit box */
  static readonly stroke: number = 2;
  static readonly iconBox: number = 24;
}

export class Type {
  static readonly text1: number = 17;
  static readonly text2: number = 16;
  static readonly text3: number = 15;
  static readonly text4: number = 13;
  static readonly body: number = 17;
  static readonly bodyLine: number = 26;
  static readonly barTitle: number = 22;
  static readonly mono: number = 14;
  static readonly monoLine: number = 21;
}

// Session list, settings, prefs (list agent): the list's rows and marks, menus, forms, the agent ring and the first run
// (apps/android design/Tokens.kt, apps/ios AcpiraDesign Metrics)

export class ListMetrics {
  static readonly s0_5: number = 2;
  static readonly s1_5: number = 6;
  /** A session row with its place under the title, and a one-line row (a project's header, a nested row) */
  static readonly sessionRow: number = 68;
  static readonly sheetRow: number = 52;
  static readonly groupCaption: number = 36;
  /** A project's rows sit this far in from its folder */
  static readonly nest: number = 16;
  static readonly chevron: number = 14;
  static readonly moreMark: number = 14;
  /** The state marks' slot, the working ring, the dots */
  static readonly stateSlot: number = 18;
  static readonly stateRing: number = 15;
  static readonly stateDot: number = 9;
  static readonly stateUnread: number = 10;
  static readonly stateOrbit: number = 4.5;
  static readonly stateInner: number = 6;
  static readonly hostAlias: number = 120;
  /** The swiped row's Delete capsule */
  static readonly swipeAction: number = 96;
  /** The bars' floating controls and their shadow */
  static readonly barShadow: number = 6;
  /** The sidebar's foot: the settings circle and the engine capsule */
  static readonly footCtl: number = 44;
  static readonly footDot: number = 8;
  /** The toast capsule's height and shadow */
  static readonly toast: number = 44;
  static readonly toastShadow: number = 12;
  /** How long a toast stays, and one with an undo (Motion.toast / toastUndo) */
  static readonly toastMs: number = 2500;
  static readonly toastUndoMs: number = 6000;
  /** The conversation search waits this long after the last key (SessionList.tsx SEARCH_DEBOUNCE) */
  static readonly searchDebounceMs: number = 180;
  /** Sessions an open project shows before "N more", one tap's page, the marks the row shows */
  static readonly perProject: number = 5;
  static readonly morePage: number = 10;
  static readonly moreMarks: number = 3;
  /** State marks' motion (styles/motion.css): spin, beat, flash, halo, orbit */
  static readonly spinMs: number = 1000;
  static readonly beatMs: number = 1400;
  static readonly flashMs: number = 3000;
  static readonly haloMs: number = 2000;
  static readonly orbitMs: number = 3600;
  static readonly motionMs: number = 250;
}

export class MenuMetrics {
  static readonly row: number = 44;
  static readonly minW: number = 220;
  static readonly radius: number = 26;
  static readonly gap: number = 8;
  static readonly shadow: number = 24;
  /** A tall menu scrolls past this many rows */
  static readonly maxRows: number = 10.5;
}

export class FormMetrics {
  static readonly row: number = 52;
  static readonly radius: number = 20;
  static readonly sectionGap: number = 24;
  static readonly dot: number = 7;
}

export class RingMetrics {
  static readonly heroTile: number = 64;
  static readonly heroBare: number = 38;
  static readonly bubble: number = 48;
  static readonly mark: number = 22;
  static readonly radiusMin: number = 96;
  static readonly pitch: number = 104;
  /** A bubble leaves the drop at this share of its size */
  static readonly seed: number = 0.35;
  static readonly staggerMs: number = 180;
  static readonly staggerMaxMs: number = 450;
  static readonly flightMs: number = 500;
  static readonly landMs: number = 280;
  static readonly heroAwayMs: number = 200;
}

export class WelcomeMetrics {
  static readonly column: number = 480;
  static readonly markMax: number = 220;
  static readonly brandMark: number = 17;
  static readonly headline: number = 34;
  static readonly headlineLine: number = 42;
  static readonly headlineGap: number = 18;
  static readonly agentsGap: number = 30;
  static readonly agentsRow: number = 12;
  static readonly agentMark: number = 22;
  static readonly agentStep: number = 20;
  static readonly actionsGap: number = 40;
  static readonly copyInset: number = 10;
  static readonly rise: number = 6;
  static readonly bellRing: number = 112;
  static readonly bell: number = 34;
  static readonly notifyTitle: number = 28;
  static readonly notifyTitleGap: number = 36;
  static readonly notifyDescGap: number = 10;
  static readonly laterHeight: number = 36;
  static readonly easeMs: number = 600;
  static readonly swapMs: number = 550;
  /** The entrances' delays: brand, the two headline lines, the agents, the button */
  static readonly brandAt: number = 100;
  static readonly hero1At: number = 300;
  static readonly hero2At: number = 540;
  static readonly factAt: number = 950;
  static readonly actionsAt: number = 1350;
}

/**
 * composer: the composer's and the dock's sizes (apps/ios AcpiraDesign Tokens.swift barCtl, goo*, dock*, question*,
 * plan*; apps/android design/Tokens.kt stagedTile, usageCardW)
 */
export class Cmp {
  /** The bar's height, its chips' and send's slot; the glyphs on it */
  static readonly bar: number = 48;
  static readonly barIcon: number = 22;
  /** The model chip's × and ‹ in place of the mark */
  static readonly gooBack: number = 26;
  /** The field: TypeScale.field (17 on 24), its card cap and the lines from which the whole-screen editor is offered */
  static readonly field: number = 17;
  static readonly fieldLine: number = 24;
  static readonly maxLines: number = 8;
  static readonly expandLines: number = 3;
  static readonly ctlSm: number = 36;
  static readonly chevron: number = 14;
  static readonly s0_5: number = 2;
  static readonly s1_5: number = 6;
  static readonly s2_5: number = 10;
  static readonly s3: number = 12;
  /** A bar's shadow under the composer and the dock cards */
  static readonly shadow: number = 6;
  /** The gap between the surfaces under the thread (question, alert, editing bar, composer) and above it (plan card) */
  static readonly dockGap: number = 8;
  static readonly rCard: number = 20;
  /** The beans: their column gap, a panel's row, width, rows shown and corners, a caption's room */
  static readonly gooGap: number = 12;
  static readonly gooRow: number = 40;
  static readonly gooPanelW: number = 232;
  static readonly gooPanelRows: number = 6.5;
  static readonly rGooPanel: number = 22;
  static readonly gooAside: number = 240;
  static readonly gooPadX: number = 14;
  /** The card's text under an open picker, and the page's blur there */
  static readonly gooRecede: number = 0.35;
  static readonly gooBlur: number = 16;
  /** A bean leaves the chip at this share of its size */
  static readonly gooSeed: number = 0.35;
  /** The staged strip: a tile, a file pill's widest, the ✕ on a tile */
  static readonly stagedTile: number = 56;
  static readonly stagedFileW: number = 200;
  static readonly stagedRemove: number = 20;
  /** The sidekick's badge on its mark, and the ring cut round it */
  static readonly sidekickBadge: number = 10;
  static readonly sidekickNotch: number = 2;
  /** The context ring's card and the quotes' card */
  static readonly usageCardW: number = 260;
  static readonly popLg: number = 300;
  /** How faint a queued prompt's bubble stands until it goes, and the lines it shows */
  static readonly queuedBubble: number = 0.55;
  static readonly queuedLines: number = 3;
  /** The question card: section gap, top inset, an option's bleed and a label's description gap, its body cap */
  static readonly questionSectionGap: number = 6;
  static readonly questionTopPad: number = 12;
  static readonly questionOptionPad: number = 4;
  static readonly questionDescriptionGap: number = 2;
  static readonly questionBody: number = 320;
  static readonly optionKey: number = 11;
  /** The docked to-do card: vertical inset, head gap, an entry's padding, a dense row and the height cap (7 rows) */
  static readonly planPadY: number = 6;
  static readonly planSectionGap: number = 4;
  static readonly planEntryPadY: number = 2;
  static readonly rowDense: number = 32;
  static readonly planMax: number = 224;
  static readonly planDot: number = 8;
  /** A disabled row in a panel (a Fusion sidekick the lead does not pair with) */
  static readonly dimmed: number = 0.4;
  /** `+` turns into × by this many degrees */
  static readonly plusTurn: number = 45;
  /** The dock cards' line heights (WebType text2 / text3) */
  static readonly text2Line: number = 22;
  static readonly text3Line: number = 20;
  /** A bean panel grows from this share of its size */
  static readonly panelSeed: number = 0.2;
  /** The ring: its stroke and inset are this part of the glyph box, its track this share of the ink, a stale fill this */
  static readonly ringParts: number = 8;
  static readonly ringTrack: number = 0.25;
  static readonly ringStale: number = 0.5;
  /** The threshold's figure on an unsupported card */
  static readonly ringNoteFaint: number = 0.6;
  /** The quote card shows this many lines of each quote; its comment field grows to this many */
  static readonly quoteLines: number = 5;
  static readonly quoteCommentLines: number = 4;
  /** The thread's own controls: a queued bubble's text inset */
  static readonly queuedPadX: number = 14;
  /** Picked photos: at most this many at once, the longest side cut to this many px, the JPEG quality */
  static readonly maxPhotos: number = 4;
  static readonly imageSide: number = 1024;
  static readonly jpegQuality: number = 80;
  /** Motion: the beans' spring (response s, damping), each bean's lag (ms), a panel's and the card's spring */
  static readonly gooResponse: number = 0.42;
  static readonly gooDamping: number = 0.78;
  static readonly gooLagMs: number = 24;
  static readonly morphResponse: number = 0.36;
  static readonly morphDamping: number = 0.86;
  static readonly quickMs: number = 160;
  /** Mention.tsx's wait before a query goes out (ms) */
  static readonly mentionDebounceMs: number = 60;
  /** How much of the screen a docked question may take */
  static readonly questionShare: number = 0.35;
  /** Half a turn, for a caret that folds */
  static readonly halfTurn: number = 180;
  /** The web scale's pill sizes (WebMetrics icon / ctlSm / chevron), for the attachment pills */
  static readonly webIcon: number = 14;
  static readonly webCtlSm: number = 26;
  static readonly webChevron: number = 12;
  static readonly webLead: number = 20;
  /** A plan dot's ring */
  static readonly planRing: number = 1.5;
}

/** Paired remote access's screens (apps/ios Pairing/PairingTokens.swift, apps/android design/Tokens.kt pair*) */
export class PairMetrics {
  /** The connect frame's glyph and the slot it keeps */
  static readonly glyph: number = 52;
  static readonly glyphBox: number = 64;
  /** The glyph centre's place down the page (a share of its height): a phase that adds a line keeps it there */
  static readonly heroRise: number = 0.42;
  /** The frame's sentence keeps two lines, so a phase with a longer one moves nothing under it */
  static readonly lineSlot: number = 52;
  /** The path's tag in the sentence's place (ui/Button ChipTag) */
  static readonly pathX: number = 6;
  static readonly pathH: number = 22;
  static readonly rXs: number = 3;
  /** The paired check on the glyph: its disc, the tick, the page ring around it, and how far it hangs past the corner */
  static readonly check: number = 22;
  static readonly checkIcon: number = 13;
  static readonly checkRing: number = 3;
  static readonly checkX: number = 9;
  static readonly checkY: number = 5;
  /** A digit's box on the code page, the gap between the groups of three, its border, the caret */
  static readonly codeBox: number = 48;
  static readonly codeBoxH: number = 60;
  static readonly codeSplit: number = 20;
  static readonly codeLine: number = 1.5;
  static readonly caret: number = 2;
  static readonly caretH: number = 30;
  /** The code page's status line keeps two lines */
  static readonly statusSlot: number = 40;
  /** The camera card, its viewfinder and the finder's corners */
  static readonly scanCard: number = 340;
  static readonly finder: number = 176;
  static readonly finderCorner: number = 30;
  static readonly finderStroke: number = 4;
  /** A form row's height, a section's header room and the inset grouped rows keep from the page's edges */
  static readonly formRow: number = 52;
  static readonly sectionGap: number = 28;
  /** The online dot on a computer's glyph, and the row fill cut around it */
  static readonly dot: number = 8;
  static readonly dotCut: number = 2;
  /** The paired moment holds this long before the sheet goes (DevicesCard.tsx HOLD_MS); a later wire's path this long */
  static readonly holdMs: number = 2400;
  static readonly doneMs: number = 600;
  /** How long a pasted text that is no pairing link says so */
  static readonly noticeMs: number = 2400;
  /** A line swapping in place (devices.css `.dv-swap`), the check landing from this scale, a disabled action's ink */
  static readonly swapMs: number = 260;
  static readonly checkFrom: number = 0.35;
  static readonly disabled: number = 0.4;
  /** The code box caret's half period */
  static readonly blinkMs: number = 500;
}

export class PairType {
  /** The computer's name under the glyph */
  static readonly nameSize: number = 22;
  static readonly nameLine: number = 28;
  /** The frame's sentence, on a looser line than body */
  static readonly line: number = 17;
  static readonly lineLine: number = 26;
  /** A digit in a code box */
  static readonly digit: number = 28;
}
