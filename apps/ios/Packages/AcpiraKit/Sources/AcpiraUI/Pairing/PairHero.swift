import AcpiraDesign
import AcpiraSync
import SwiftUI

// The pairing screens' one centre piece, laid out as the system's empty states are: a computer's glyph, its name, one
// sentence and one action. The empty list, connecting, connected and every failure use it, so moving between them swaps
// contents in a frame that stays: the glyph keeps its place, the sentence and the action keep their slots. Connected,
// the sentence gives way to the path the wire travels, as the computer's card shows the phone's under its title.

struct PairHero: View {
  /// rest: the sentence alone; spin: the system spinner leads it
  enum Ring { case rest, spin }
  var symbol: String
  var ring: Ring = .rest
  /// The code being paired with, drawn where the glyph goes (the sheet, after a scan or a link): the glyph shows in its
  /// centre once paired
  var code: [String]?
  /// Paired: the code dissolves, the glyph takes a check and the name says so, on the computer card's timeline
  var paired = false
  var name: String?
  var line: String
  /// Paired and the wire up: the path it travels, the card's tag in the sentence's place. Nil while it is not known, so
  /// nothing is guessed; a path that changes (a relay giving way to a punched hole) crosses over in place
  var path: PathKind?
  var lineInk: Color = Palette.fg2
  var action: String?
  /// The computer does not answer: its glyph steps back
  var dim = false
  var onAction: () -> Void = {}
  @Environment(\.motionEnabled) private var motion

  var body: some View {
    VStack(spacing: 0) {
      Group {
        if let code {
          PairCodeFrame(rows: code, symbol: symbol, paired: paired, dim: dim)
        } else {
          // A link names its computer but not its kind: the glyph turns into the right one once paired
          PairGlyph(symbol: symbol, ink: paired ? Palette.fg1 : dim ? Palette.fg3 : Palette.fg2, checked: paired)
            .contentTransition(.symbolEffect(.replace))
            .animation(motion ? Motion.open : nil, value: symbol)
            .frame(height: Metrics.pairGlyphBox)
        }
      }
      .padding(.bottom, Metrics.page)
      .accessibilityHidden(true)
      if let name {
        // The name, then "Paired <name>" in its place, as the card's title turns into the device it paired
        ZStack {
          title(name, off: paired)
          title(t("devices.paired", ["name": name]), off: !paired)
        }
        .padding(.bottom, Metrics.s2)
      }
      // The sentence, or the path's tag in its place: every variant in one cell, so the slot keeps the largest and a
      // change (the sentence giving way to the tag, one path to another) crosses over without moving anything
      ZStack(alignment: .top) {
        // Working: the system spinner before the sentence, as the system says "Searching…"
        // Centred on the first line: a spinner has no baseline of its own
        HStack(alignment: .center, spacing: Metrics.s2) {
          if ring == .spin { ProgressView().controlSize(.small) }
          Text(web: line, size: TypeScale.body.size)
            .textStyle(TypeScale.body)
            .foregroundStyle(lineInk)
            .multilineTextAlignment(.center)
        }
        .modifier(PairSwap(off: path != nil))
        ForEach(Self.paths, id: \.self) { kind in
          PairPathTag(path: kind).modifier(PairSwap(off: path != kind))
        }
      }
      .frame(maxWidth: .infinity, minHeight: Metrics.pairLineSlot, alignment: .top)
      // The action's slot stays when there is none, so a phase without one moves nothing
      Button(action: onAction) { Text(web: action ?? " ", size: TypeScale.body.size, paragraph: false) }
        .buttonStyle(PairPrimaryStyle())
        .opacity(action == nil ? 0 : 1)
        .disabled(action == nil)
        .accessibilityHidden(action == nil)
        .padding(.top, Metrics.s2)
    }
    .padding(.horizontal, Metrics.page)
  }

  /// One of the title's two lines, swapped in place
  private func title(_ text: String, off: Bool) -> some View {
    Text(web: text, size: TypeScale.pairName.size, paragraph: false)
      .lineLimit(1)
      .textStyle(TypeScale.pairName, weight: .semibold)
      .foregroundStyle(Palette.fgStrong)
      .modifier(PairSwap(off: off))
  }

  /// Every path a wire can take, in the card's order (access.ts PATHS)
  private static let paths: [PathKind] = [.lan, .tailscale, .direct, .relay]
}

/// A line swapped in place: off, it fades and blurs out a few points low, on, it comes back (devices.css `.dv-swap`)
private struct PairSwap: ViewModifier {
  var off: Bool
  @Environment(\.motionEnabled) private var motion

  func body(content: Content) -> some View {
    content
      .animation(motion ? Motion.pairSwapFade : nil) { $0.opacity(off ? 0 : 1).blur(radius: off ? Motion.pairSwapBlur : 0) }
      .animation(motion ? Motion.pairSwapMove : nil) { $0.offset(y: off ? Motion.pairSwapY : 0) }
      .accessibilityHidden(off)
  }
}

/// The path a paired computer's wire travels, as the computer's card shows the phone's under its title (ui/Button
/// ChipTag): its name on the active fill. Read out after "Connected", which the tag stands for
struct PairPathTag: View {
  let path: PathKind

  var body: some View {
    Text(web: pathLabel(path), paragraph: false)
      .textStyle(TypeScale.text4)
      .foregroundStyle(Palette.fg2)
      .lineLimit(1)
      .padding(.horizontal, Metrics.pairPathX)
      .frame(height: Metrics.pairPathH)
      .background(RoundedRectangle(cornerRadius: Metrics.rXs, style: .continuous).fill(Palette.active))
      .accessibilityLabel(Text(verbatim: [t("mobile.pair.connected"), pathLabel(path)].joined(separator: t("common.metaSep"))))
  }
}

/// Puts a hero's glyph (or code) centre at the page's optical centre (`heroRise`) with everything else hanging under it,
/// so a phase that adds or drops a line keeps the glyph where it was
struct PairHeroPlacement<Content: View>: View {
  /// The height of the hero's top piece: the glyph's slot, or the code's box
  var box: CGFloat = Metrics.pairGlyphBox
  @ViewBuilder var content: () -> Content

  var body: some View {
    GeometryReader { page in
      VStack(spacing: 0) {
        Color.clear.frame(height: max(0, page.size.height * Metrics.heroRise - box / 2))
        content()
        Spacer(minLength: 0)
      }
      .frame(width: page.size.width)
    }
  }
}

/// The one primary action: a crisp solid capsule, white in dark and black in light, like the composer's send
struct PairPrimaryStyle: ButtonStyle {
  @Environment(\.motionEnabled) private var motion
  func makeBody(configuration: Configuration) -> some View {
    configuration.label
      .textStyle(TypeScale.body, weight: .semibold)
      .foregroundStyle(Palette.btn1Fg)
      .lineLimit(1)
      .padding(.horizontal, Metrics.page + Metrics.s2)
      .frame(minHeight: Metrics.barCtl)
      .background(Capsule().fill(Palette.btn1Bg))
      .contentShape(Capsule())
      .brightness(configuration.isPressed ? Motion.pressBrightness : 0)
      .scaleEffect(configuration.isPressed && motion ? Motion.pressScale : 1)
      .animation(motion ? Motion.press : nil, value: configuration.isPressed)
  }
}

/// A quiet action beside content: a glass capsule
struct PairGlassStyle: ButtonStyle {
  func makeBody(configuration: Configuration) -> some View {
    configuration.label
      .textStyle(TypeScale.text2, weight: .medium)
      .foregroundStyle(Palette.fg1)
      .lineLimit(1)
      .padding(.horizontal, Metrics.pad)
      .frame(minHeight: Metrics.ctl)
      .contentShape(Capsule())
      .glassEffect(GlassStyle.control.interactive(), in: Capsule())
  }
}
