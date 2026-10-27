import SwiftUI

// The transcript (and the rows it shares with the dock) is the webview's port, matched against the LAB reference
// (`lab/ios-reference.preview.tsx`) at the reading size the app runs at: ui 17 / code 14 under `Reading.phone`, the
// reference page's 16 / 13 otherwise. Type steps follow tokens.css's formulas at that size; every other measure is
// the airy density's own, which tokens.css does not scale with the font. The chrome around the transcript (list,
// settings, composer, sheets) is phone-only design and keeps its own steps in `TypeScale` / `Metrics`.

/// The webview's type steps (`--text-*`, `--text-mono`) at the app's ui / code size
public enum WebType {
  /// --ui-font-size / --code-font-size
  public static var ui: CGFloat { Reading.pick(Tokens.uiFont, Tokens.uiFont + 1) }
  public static var code: CGFloat { Reading.pick(Tokens.codeFont, Tokens.codeFont + 1) }
  /// --text-1 (airy: ui / ui + 9): prose, the user card
  public static var text1: TextStyle { TextStyle(size: ui, lineHeight: ui + 9) }
  /// --text-2 (ui / ui + 5): rows
  public static var text2: TextStyle { TextStyle(size: ui, lineHeight: ui + 5) }
  /// --text-3 (ui − 1 / ui + 3): captions, trailing meta
  public static var text3: TextStyle { TextStyle(size: ui - 1, lineHeight: ui + 3) }
  /// --text-4 (ui − 2 / ui + 2)
  public static var text4: TextStyle { TextStyle(size: ui - 2, lineHeight: ui + 2) }
  /// --text-mono (code / code + 6)
  public static var mono: TextStyle { TextStyle(size: code, lineHeight: code + 6, mono: true) }
  /// Code output lines: --text-mono at --code-output-line-height
  public static var codeOutput: TextStyle { TextStyle(size: code, lineHeight: WebMetrics.codeOutputLine, mono: true) }
}

/// The webview's measures under density airy (`[data-density="airy"]` over the base tokens), font-independent
public enum WebMetrics {
  public static let row: CGFloat = 32
  public static let rowDense: CGFloat = 28
  public static let chatRow: CGFloat = 30
  public static let processRow: CGFloat = 24
  public static let lead: CGFloat = 20
  public static let icon: CGFloat = 14
  public static let iconCtl: CGFloat = 16
  /// Tailwind size-3: the fold chevron, the run's ✓ / ✗
  public static let chevron: CGFloat = 12
  public static let ctl: CGFloat = 32
  public static let ctlSm: CGFloat = 26
  public static let msgGap: CGFloat = 24
  /// The label column: the lead slot and the gap after it
  public static var indent: CGFloat { lead + Metrics.gap }
  /// --code-output-max / --code-output-line-height
  public static let codeOutputMax: CGFloat = 350
  public static let codeOutputLine: CGFloat = 22
}
