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
