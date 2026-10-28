import AcpiraDesign
import SwiftUI

// The pairing screens' one centre piece, laid out as the system's empty states are: a computer's glyph, its name, one
// sentence and one action. The empty list, connecting, connected and every failure use it, so moving between them swaps
// contents in a frame that stays: the glyph keeps its place, the sentence and the action keep their slots.

struct PairHero: View {
  /// rest: the glyph alone; spin: the system spinner leads the sentence; done: the glyph turns into a check
  enum Ring { case rest, spin, done }
  var symbol: String
  var ring: Ring = .rest
  var name: String?
  var line: String
  var lineInk: Color = Palette.fg2
  var action: String?
  /// The computer does not answer: its glyph steps back
  var dim = false
  var onAction: () -> Void = {}
  @Environment(\.motionEnabled) private var motion

  var body: some View {
    VStack(spacing: 0) {
      Group {
        if ring == .done {
          Image(systemName: "checkmark.circle.fill").foregroundStyle(Palette.ok)
        } else {
          // A link names its computer but not its kind: the glyph turns into the right one once paired
          Image(systemName: symbol).foregroundStyle(dim ? Palette.fg3 : Palette.fg2)
        }
      }
      .font(.system(size: Metrics.pairGlyph))
      .contentTransition(.symbolEffect(.replace))
      .frame(height: Metrics.pairGlyphBox)
      .animation(motion ? Motion.open : nil, value: ring)
      .padding(.bottom, Metrics.page)
      .accessibilityHidden(true)
      if let name {
        Text(web: name, size: TypeScale.pairName.size, paragraph: false)
          .lineLimit(1)
          .textStyle(TypeScale.pairName, weight: .semibold)
          .foregroundStyle(Palette.fgStrong)
          .padding(.bottom, Metrics.s2)
      }
      // Working: the system spinner before the sentence, as the system says "Searching…"
      // Centred on the first line: a spinner has no baseline of its own
      HStack(alignment: .center, spacing: Metrics.s2) {
        if ring == .spin { ProgressView().controlSize(.small) }
        Text(web: line, size: TypeScale.body.size)
          .textStyle(TypeScale.body)
          .foregroundStyle(lineInk)
          .multilineTextAlignment(.center)
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
}

/// Puts a hero's glyph centre at the page's optical centre (`heroRise`) with everything else hanging under it, so a phase
/// that adds or drops a line keeps the glyph where it was
struct PairHeroPlacement<Content: View>: View {
  @ViewBuilder var content: () -> Content

  var body: some View {
    GeometryReader { page in
      VStack(spacing: 0) {
        Color.clear.frame(height: max(0, page.size.height * Metrics.heroRise - Metrics.pairGlyphBox / 2))
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
