import AcpiraDesign
import SwiftUI

// The pairing screens' sizes, kept with the screens until Tokens.swift takes them (it has other sessions' edits in
// flight). Views read only these names, never raw numbers.

nonisolated extension Metrics {
  /// The connect frame's glyph (empty list, connecting, connected, a failure) and the slot it keeps
  static let pairGlyph: CGFloat = 52
  static let pairGlyphBox: CGFloat = 64
  /// The row background cut around a settings row's online dot, so the dot reads apart from the glyph under it
  static let pairDotCut: CGFloat = 2
  /// A code box's border
  static let pairCodeLine: CGFloat = 1.5
  /// The frame's sentence keeps two lines, so a phase with a longer one moves nothing under it
  static var pairLineSlot: CGFloat { 2 * TypeScale.body.lineHeight }
  /// The camera card, its viewfinder and the finder's corners
  static let pairScanCard: CGFloat = 340
  static let pairFinder: CGFloat = 176
  static let pairFinderCorner: CGFloat = 30
  static let pairFinderRadius: CGFloat = 22
  static let pairFinderStroke: CGFloat = 4
  /// A digit's box on the code page, the gap between the two groups of three, and the caret
  static let pairCodeBox: CGFloat = 48
  static let pairCodeBoxH: CGFloat = 60
  static let pairCodeSplit: CGFloat = 20
  static let pairCaret: CGFloat = 2
  static let pairCaretH: CGFloat = 30
  /// The code page's status line keeps two lines (a computer's name can make an error wrap)
  static var pairStatusSlot: CGFloat { 2 * TypeScale.text3.lineHeight }
  /// The scanned code's box, quiet zone included, its track's corner and stroke (pairGeometry.ts QR_BOX, RING_R,
  /// RING_STROKE), and the mark's share of the knock-out
  static let pairCode: CGFloat = 228
  static let pairCodeRadius: CGFloat = 16
  static let pairCodeTrack: CGFloat = 1.5
  static let pairCodeMark: CGFloat = 0.7
  /// The paired check on the computer's glyph: its disc, the tick, the ring of page around it, and how far it hangs past
  /// the glyph's corner (devices.css `.dv-qr-check`)
  static let pairCheck: CGFloat = 22
  static let pairCheckIcon: CGFloat = 13
  static let pairCheckStroke: CGFloat = 3
  static let pairCheckRing: CGFloat = 3
  static let pairCheckX: CGFloat = 9
  static let pairCheckY: CGFloat = 5
  /// The path the wire travels, in the sentence's place once it is up: the card's tag under its title (ui/Button
  /// ChipTag: text-4 on the active fill, a hair of padding, the xs corner) at the phone's scale
  static let pairPathX: CGFloat = 6
  static let pairPathH: CGFloat = 22
}

nonisolated extension TypeScale {
  /// The computer's name under the glyph
  static let pairName = TextStyle(size: Tokens.uiFont + 6, lineHeight: Tokens.uiFont + 12)
  /// A digit in a code box
  static let pairDigit = TextStyle(size: Tokens.uiFont + 12, lineHeight: Tokens.uiFont + 18)
}

nonisolated extension Motion {
  /// The paired moment, on the computer card's timeline (devices.css), so a phone held up to the card plays it in step:
  /// the rings dissolve outward one after another, the track goes, the mark gives way to the computer, its check lands
  static let pairBandStagger: Double = 0.042
  static var pairBandFade: Animation { .easeOut(duration: 0.38) }
  static var pairBandGrow: Animation { .timingCurve(0.2, 0.7, 0.2, 1, duration: 0.52) }
  static let pairBandScale: CGFloat = 1.07
  static let pairBandBlur: CGFloat = 1.6
  static var pairTrackOut: Animation { .timingCurve(0.25, 0.1, 0.25, 1, duration: 0.3) }
  static var pairMarkFade: Animation { .timingCurve(0.25, 0.1, 0.25, 1, duration: 0.24).delay(0.16) }
  static var pairMarkGrow: Animation { .timingCurve(0.3, 1.3, 0.5, 1, duration: 0.42).delay(0.16) }
  static let pairMarkScale: CGFloat = 1.4
  static let pairMarkBlur: CGFloat = 3
  static var pairGlyphIn: Animation { .timingCurve(0.2, 0.9, 0.3, 1.15, duration: 0.46).delay(0.38) }
  static let pairGlyphFrom: CGFloat = 0.82
  static let pairGlyphBlur: CGFloat = 3
  static var pairCheckIn: Animation { .timingCurve(0.3, 1.5, 0.5, 1, duration: 0.42).delay(0.72) }
  static let pairCheckFrom: CGFloat = 0.35
  /// A line swapping in place, the title's and the sentence's (devices.css `.dv-swap`)
  static var pairSwapFade: Animation { .timingCurve(0.25, 0.1, 0.25, 1, duration: 0.26) }
  static var pairSwapMove: Animation { .timingCurve(0.2, 0.7, 0.2, 1, duration: 0.32) }
  static let pairSwapY: CGFloat = 3
  static let pairSwapBlur: CGFloat = 2
  /// How long the paired moment holds before the sheet goes, as long as the card holds it before its list
  /// (DevicesCard.tsx HOLD_MS), so both screens move on together
  static let pairHold: Double = 2.4
  /// A wire that comes up after the hold: its path shows this long before the sheet goes
  static let pairDone: Double = 0.6
  /// How long a pasted text that is no pairing link says so in the paste row
  static let pairNotice: Double = 2.4
  /// The code box caret's half period, as a text field's caret blinks
  static let pairBlink: Double = 0.5
}

nonisolated extension Palette {
  /// The camera dimmed outside the viewfinder, and the viewfinder's corners: over a camera picture in either theme
  static var pairFinderDim: Color { .black.opacity(0.22) }
  static var pairFinderInk: Color { .white }
  /// The scanned code's dim track on its box edge (devices.css `--dv-track`)
  static var pairTrack: Color { fg1.opacity(0.15) }
  /// The paired check's tick on its green disc, in either theme
  static var pairCheckInk: Color { .white }
  /// A failed code steps back behind the sentence
  static let pairCodeDim: Double = 0.2
  /// The code page's real field under its boxes: all but invisible, since a hidden field takes no focus
  static let pairHiddenField: Double = 0.02
}
