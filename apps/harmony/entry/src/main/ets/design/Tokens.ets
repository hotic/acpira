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
