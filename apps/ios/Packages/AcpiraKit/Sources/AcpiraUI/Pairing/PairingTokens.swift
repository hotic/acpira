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
}

nonisolated extension TypeScale {
  /// The computer's name under the glyph
  static let pairName = TextStyle(size: Tokens.uiFont + 6, lineHeight: Tokens.uiFont + 12)
  /// A digit in a code box
  static let pairDigit = TextStyle(size: Tokens.uiFont + 12, lineHeight: Tokens.uiFont + 18)
}

nonisolated extension Motion {
  /// How long the connected check stays before the sheet goes
  static let pairDone: Double = 1.2
  /// How long a pasted text that is no pairing link says so in the paste row
  static let pairNotice: Double = 2.4
  /// The code box caret's half period, as a text field's caret blinks
  static let pairBlink: Double = 0.5
}

nonisolated extension Palette {
  /// The camera dimmed outside the viewfinder, and the viewfinder's corners: over a camera picture in either theme
  static var pairFinderDim: Color { .black.opacity(0.22) }
  static var pairFinderInk: Color { .white }
  /// The code page's real field under its boxes: all but invisible, since a hidden field takes no focus
  static let pairHiddenField: Double = 0.02
}
