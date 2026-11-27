import AcpiraDesign
import AcpiraModels
import SwiftUI
#if canImport(UIKit)
import UIKit
#endif

// Liquid pickers after libraries.dev/gooey. A chip sits in the composer's
// glass group; opening it pushes a column of beans out of it, each bean squeezing out of the one before (a telescope),
// and closing pulls them back in, the outermost first. A bean may turn into a panel of choices: the tapped bean
// stretches into the panel (its bottom-left corner stays put) and the other beans shrink into it, and a pick shrinks
// the panel back into the bean. The chip is the way back: × while the beans are out, ‹ while a panel is, one level per
// tap; a tap anywhere else closes everything, as outside a system menu. One progress vector drives the chip and
// each bean's panel: a spring while opening or picking, a non-bouncing curve while closing.
//
// Every silhouette is Liquid Glass in the composer's own `GlassGroup` (`gooGlass()` on the group), so a
// bean melts out of the group, into its neighbours and into its panel as the system's glass controls do, and the
// group is never covered by a second copy of itself. The glyphs are the glass's own content. The captions, the
// panels' rows and every hit target are drawn by the screen (`gooLayer()`): drawn by the composer they hang outside
// its frame over the transcript, whose scroll view takes every tap. Both read one geometry (`GooGeometry`) at the
// same spring, so the rows ride on the glass. (A blurred, alpha-thresholded Canvas drew opaque grey goo before
// 2026-10-27, and an overlay copy of the chip's glass doubled the group; both are gone.)

/// One bean a liquid picker pulls out of its chip
struct GooPiece: Identifiable {
  let id: String
  /// What the bean's hit target reads to VoiceOver
  var name: String
  /// The resting centre, from the chip's centre
  var offset: CGSize
  /// The glyph on the bean's glass
  var glyph: AnyView
  /// The value on the bean's right (a setting's current value, a source's name), gone while a panel is open
  var caption: AnyView? = nil
  /// The panel a tap turns the bean into, if any
  var panel: GooPanel? = nil
  /// A tap on the bean or its caption
  var action: () -> Void
}

/// The choices a bean turns into: a list of rows (the chip below is the way back, so it carries no back row)
struct GooPanel {
  var size: CGSize
  var content: AnyView
}

/// The progress of a liquid picker: [the chip, bean 1's panel, bean 2's panel, …], each 0 closed and 1 open. A vector
/// so one spring moves every stage at once (a panel can shrink back while the beans go home)
struct GooProgress: VectorArithmetic {
  var values: [CGFloat]

  subscript(_ i: Int) -> CGFloat { i < values.count ? values[i] : 0 }

  static var zero: GooProgress { GooProgress(values: []) }

  static func + (a: GooProgress, b: GooProgress) -> GooProgress { zip(a, b, +) }
  static func - (a: GooProgress, b: GooProgress) -> GooProgress { zip(a, b, -) }

  mutating func scale(by rhs: Double) { values = values.map { $0 * CGFloat(rhs) } }

  var magnitudeSquared: Double { values.reduce(0) { $0 + Double($1 * $1) } }

  /// Element-wise; the shorter side counts as zeros (SwiftUI's `zero` has no length)
  private static func zip(_ a: GooProgress, _ b: GooProgress, _ op: (CGFloat, CGFloat) -> CGFloat) -> GooProgress {
    GooProgress(values: (0..<max(a.values.count, b.values.count)).map { op(a[$0], b[$0]) })
  }
}

/// One bean at one progress: its silhouette, from the chip's centre, and how much of what rides on it shows
struct GooFrame {
  var centre: CGPoint
  var size: CGSize
  var radius: CGFloat
  /// The glyph's opacity, and its scale while an open panel swallows the bean
  var glyph: Double
  var scale: CGFloat
  var caption: Double
  /// Where the bean's panel rests open, from the chip's centre, and how much of its rows shows
  var panel: CGRect?
  var rows: Double
  /// How much of the page's colour backs the glass: a panel's rows over a clear lens read the content behind it
  var backing: Double = 0
  /// Whether the bean and its caption, or its panel, take taps
  var live: Bool
  var panelLive: Bool
}

/// The one geometry the composer's glass and the screen's rows both draw from
enum GooGeometry {
  /// Under this the picker is closed and nothing is drawn
  static let gone: CGFloat = 0.001
  /// Keep the glyph with the shrinking lens until it reaches the chip, rather than leaving an empty glass blob.
  private static let glyphFrom: CGFloat = 0.15, glyphSpan: CGFloat = 0.65
  /// From a bare chip (no glass for the bean to melt out of) the first bean's glyph, and a panel's closing with the
  /// picker once its rows are gone (`rowsFrom`), ride their drop: shown from this share of a bean's height and sized
  /// with it. Grown out of nothing, the bean rose as an empty grey dot over the chip, and the panel shrank home as an
  /// empty grey drop
  private static let bareFrom: CGFloat = 0.15, bareSpan: CGFloat = 0.25
  /// Captions come last on the way out and go first on the way in (the chip's own value has to be nearly 1)
  private static let captionFrom: CGFloat = 0.7, captionMain: CGFloat = 0.85
  /// A panel's rows show once it is mostly grown; the other beans are gone by `swallow` of the way
  private static let rowsFrom: CGFloat = 0.55, rowsSpan: CGFloat = 0.35, swallow: CGFloat = 0.6
  /// Crossfade a bean's glyph with its panel rows. A gap between their ranges left a bare lens during every pick.
  /// Captions leave as a panel starts and return once the beans have nearly grown back.
  private static let glyphOut: CGFloat = 0.75, glyphSwallowed: CGFloat = 0.35, captionOut: CGFloat = 0.1
  /// Taps land once a bean or panel has arrived, and on the beans only once no panel is left over them
  private static let arrived: CGFloat = 0.9, uncovered: CGFloat = 0.05
  /// Grow with the whole flight, so a closing bean shrinks before its glyph fades rather than leaving an empty hump.
  private static let grown: CGFloat = 1

  static func frames(_ pieces: [GooPiece], _ progress: GooProgress, chip: CGSize, seed: CGFloat) -> [GooFrame] {
    let main = progress[0]
    let stages = pieces.indices.map { max(0, progress[$0 + 1]) }
    let opened = unit(stages.max() ?? 0)
    // The lags spread over at most gooStaggerMax, so a long column still arrives
    let span = min(Motion.gooStagger * CGFloat(max(0, pieces.count - 1)), Motion.gooStaggerMax)
    // A panel's bottom-left corner is the column's: it grows from the lowest bean's corner up and to the right
    let corner = pieces.first.map { CGPoint(x: $0.offset.width - chip.width / 2, y: $0.offset.height + chip.height / 2) } ?? .zero
    func rest(_ panel: GooPanel) -> CGRect {
      CGRect(x: corner.x, y: corner.y - panel.size.height, width: panel.size.width, height: panel.size.height)
    }
    // The panel that is opening (or open), where its glass is now: the other beans slide into it as they shrink
    let host = stages.indices.max { stages[$0] < stages[$1] }.flatMap { i in pieces[i].panel.map { (i, $0) } }
    var out: [GooFrame] = []
    var at = CGPoint.zero
    var last = CGPoint.zero
    for (i, piece) in pieces.enumerated() {
      let lag = pieces.count > 1 ? span * CGFloat(i) / CGFloat(pieces.count - 1) : 0
      let t = share(main, lag: lag, span: span)
      // Each bean rides out inside the one before and leaves it for its own rest
      let goal = CGPoint(x: piece.offset.width, y: piece.offset.height)
      at = CGPoint(x: at.x + (goal.x - last.x) * t, y: at.y + (goal.y - last.y) * t)
      last = goal
      let s = stages[i]
      let others = stages.enumerated().filter { $0.offset != i }.map(\.element).max() ?? 0
      // How far another bean's panel has swallowed this bean: by as far as that panel is further out than this
      // bean's own, so a panel still springing home is swallowed too when another opens before it lands (it used to
      // hang on as a whole, empty bean until its spring settled). Measured against the chip's own progress: stepping
      // back to the beans (the chip stays open) lets it out again, while closing everything (the chip falls with the
      // panel) keeps it inside, so the panel goes home as one drop instead of spilling the beans on the way
      let swallowed = main > GooGeometry.gone ? unit(max(0, others - s) / (swallow * min(1, main))) : 1
      let scale = 1 - swallowed
      // The bean swells out of the chip's own glass circle (`seed`) and shrinks back into it, so near home it lies
      // inside the bar. At full size it stood out of the bar as a dome through the spring's slow tail, then vanished
      // at once
      let side = seed + (chip.height - seed) * unit(t / grown)
      let bean = CGSize(width: side + chip.width - chip.height, height: side)
      // The bean, or its panel part of the way out, then shrunk into the panel that swallows it
      var centre = at
      var size = bean
      var radius = bean.height / 2
      let own = piece.panel.map(rest)
      if let rect = own, s > 0 {
        centre = CGPoint(x: at.x + (rect.midX - at.x) * s, y: at.y + (rect.midY - at.y) * s)
        size = CGSize(width: bean.width + (rect.width - bean.width) * s, height: bean.height + (rect.height - bean.height) * s)
        radius = bean.height / 2 + (Metrics.rGooPanel - bean.height / 2) * unit(s)
      }
      if let (h, panel) = host, h != i {
        let rect = rest(panel)
        centre = CGPoint(x: centre.x + (rect.midX - centre.x) * swallowed, y: centre.y + (rect.midY - centre.y) * swallowed)
      }
      var f = GooFrame(centre: centre, size: CGSize(width: size.width * scale, height: size.height * scale),
                       radius: radius * scale, glyph: 0, scale: scale, caption: 0, panel: own, rows: 0,
                       live: t > arrived && opened < uncovered, panelLive: s > arrived)
      let kept = 1 - unit(swallowed / glyphSwallowed)
      if seed == 0 && (i == 0 || s > 0) {
        let held = unit(size.height / chip.height)
        let leave = main < 1 ? rowsFrom : glyphOut
        f.glyph = Double(unit((held - bareFrom) / bareSpan) * (1 - unit(s / leave)) * kept)
        f.scale = scale * held
      } else {
        f.glyph = Double(unit((t - glyphFrom) / glyphSpan) * (1 - unit(s / glyphOut)) * kept)
      }
      f.caption = Double(min(unit((t - captionFrom) / (1 - captionFrom)), unit((main - captionMain) / (1 - captionMain)))
                         * (1 - unit(opened / captionOut)))
      f.rows = Double(unit((s - rowsFrom) / rowsSpan) * kept)
      f.backing = Double(unit(s))
      out.append(f)
    }
    return out
  }

  /// A bean's own share of the chip's progress: each starts a lag after the one before and all arrive together. The
  /// spring's overshoot past 1 is passed on as is, so the column wobbles as one and no bean flies past its rest alone
  private static func share(_ main: CGFloat, lag: CGFloat, span: CGFloat) -> CGFloat {
    let out = main <= lag ? 0 : min(1, (min(main, 1) - lag) / (1 - span))
    return out + max(0, main - 1)
  }

  private static func unit(_ x: CGFloat) -> CGFloat { min(1, max(0, x)) }
}

/// The beans' glass, drawn by the composer inside its own glass container (`gooGlass()`), each glyph on its bean
private struct GooGlass: View, Animatable {
  var progress: GooProgress
  /// The chip's frame in this view's space
  var chip: CGRect
  var pieces: [GooPiece]
  var seed: CGFloat

  var animatableData: GooProgress {
    get { progress }
    set { progress = newValue }
  }

  var body: some View {
    let frames = GooGeometry.frames(pieces, progress, chip: chip.size, seed: seed)
    ZStack(alignment: .topLeading) {
      if progress[0] > GooGeometry.gone {
        // Plain views under the glass: a panel growing out of its bean fills with the page's colour, so the title or
        // the transcript behind it no longer reads through its rows
        ForEach(pieces.indices, id: \.self) { i in
          let f = frames[i]
          if f.backing > 0, f.size.width > Metrics.s0_5 {
            RoundedRectangle(cornerRadius: f.radius, style: .continuous)
              .fill(Palette.gooBacking)
              .opacity(f.backing)
              .frame(width: f.size.width, height: f.size.height)
              .position(x: chip.midX + f.centre.x, y: chip.midY + f.centre.y)
          }
        }
        ForEach(Array(pieces.enumerated()), id: \.element.id) { i, piece in
          let f = frames[i]
          if f.size.width > Metrics.s0_5 {
            piece.glyph
              .scaleEffect(f.scale)
              .opacity(f.glyph)
              .frame(width: f.size.width, height: f.size.height)
              .glassSurface(GlassStyle.bar, in: RoundedRectangle(cornerRadius: f.radius, style: .continuous))
              // A bean comes and goes inside the chip, where nothing shows: the system's own morph on insertion would
              // run its own clock over the spring
              .glassMorph(.identity)
              .position(x: chip.midX + f.centre.x, y: chip.midY + f.centre.y)
          }
        }
      }
    }
    .allowsHitTesting(false)
  }
}

/// What the screen draws over an open picker, on the positions of the composer's glass: the captions, the open
/// panel's rows and every hit target. The chip steps back one level (a panel into its bean, the beans into the chip);
/// a tap anywhere else closes whatever is open
private struct GooControls: View, Animatable {
  var progress: GooProgress
  /// Whether the picker is open or opening, not animated: a closing picker's catcher took every tap on the screen
  /// (the chip, the field) until its spring had crossed zero
  var open: Bool
  var chip: CGRect
  /// The layer's width: a caption never runs past the screen's edge, so a chip far along the bar wraps its note
  var room: CGFloat
  var pieces: [GooPiece]
  var seed: CGFloat
  var close: () -> Void
  var back: () -> Void

  var animatableData: GooProgress {
    get { progress }
    set { progress = newValue }
  }

  var body: some View {
    let frames = GooGeometry.frames(pieces, progress, chip: chip.size, seed: seed)
    ZStack(alignment: .topLeading) {
      if progress[0] > GooGeometry.gone {
        Color.clear
          .contentShape(Rectangle())
          .ignoresSafeArea()
          .onTapGesture(perform: close)
        Color.clear
          .frame(width: chip.width, height: chip.height)
          .contentShape(Circle())
          .onTapGesture(perform: back)
          .position(x: chip.midX, y: chip.midY)
          .accessibilityElement()
          .accessibilityLabel(Text(web: t(frames.contains { $0.panelLive } ? "common.back" : "common.close")))
          .accessibilityAddTraits(.isButton)
        ForEach(Array(pieces.enumerated()), id: \.element.id) { i, piece in
          let f = frames[i]
          let c = CGPoint(x: chip.midX + f.centre.x, y: chip.midY + f.centre.y)
          Button(action: piece.action) {
            Color.clear.frame(width: chip.width, height: chip.height).contentShape(Circle())
          }
          .buttonStyle(.plain)
          .accessibilityLabel(Text(web: piece.name))
          .position(c)
          .allowsHitTesting(f.live)
          if let caption = piece.caption {
            let start = c.x + chip.width / 2 + Metrics.s2
            let aside = max(0, min(Metrics.gooAside, room - Metrics.pad - start))
            Button(action: piece.action) {
              caption.frame(height: chip.height).contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .frame(width: aside, alignment: .leading)
            .opacity(f.caption)
            .position(x: start + aside / 2, y: c.y)
            .allowsHitTesting(f.live)
            .accessibilityHidden(true)
          }
          if let panel = piece.panel, let rect = f.panel, f.rows > 0 {
            panel.content
              .padding(.vertical, Metrics.s1_5)
              .frame(width: rect.width, height: rect.height)
              // The rows stay where the panel rests and the growing glass uncovers them
              .mask {
                RoundedRectangle(cornerRadius: f.radius, style: .continuous)
                  .frame(width: f.size.width, height: f.size.height)
                  .offset(x: f.centre.x - rect.midX, y: f.centre.y - rect.midY)
              }
              .opacity(f.rows)
              .position(x: chip.midX + rect.midX, y: chip.midY + rect.midY)
              .allowsHitTesting(f.panelLive)
          }
        }
      }
    }
    .allowsHitTesting(open)
  }
}

/// What a chip publishes: where it is, how far open, its beans, and how to close it or step back one level
struct GooItem {
  let id: String
  let anchor: Anchor<CGRect>
  let progress: GooProgress
  let pieces: [GooPiece]
  /// The bean's size as it leaves the chip: the chip's glass circle, or nothing for a bare chip
  let seed: CGFloat
  let close: () -> Void
  let back: () -> Void
}

struct GooKey: PreferenceKey {
  static let defaultValue: [GooItem] = []
  static func reduce(value: inout [GooItem], nextValue: () -> [GooItem]) { value += nextValue() }
}

extension View {
  /// Draws the glass of the liquid pickers published inside this view; applied to the glass the chips sit on, inside
  /// its `GlassGroup`, so the beans melt out of it
  func gooGlass() -> some View {
    overlayPreferenceValue(GooKey.self) { items in
      GeometryReader { proxy in
        ForEach(items, id: \.id) { item in
          GooSpring(progress: item.progress) {
            GooGlass(progress: item.progress, chip: proxy[item.anchor], pieces: item.pieces, seed: item.seed)
          }
        }
      }
    }
  }

  /// Draws what rides on the liquid pickers published inside this view over the whole screen, so every caption,
  /// row and bean takes taps (the composer draws the glass and the dim)
  func gooLayer() -> some View {
    overlayPreferenceValue(GooKey.self) { items in
      GeometryReader { proxy in
        ForEach(items, id: \.id) { item in
          GooSpring(progress: item.progress) {
            GooControls(progress: item.progress, open: item.progress[0] > 0, chip: proxy[item.anchor], room: proxy.size.width,
                        pieces: item.pieces, seed: item.seed, close: item.close, back: item.back)
          }
        }
      }
    }
    // The screen under the bar, so a composer can tell how far its beans may rise (`GooColumn.space`)
    .coordinateSpace(.named(GooColumn.space))
  }
}

/// The glass and rows run the same curve from the same change; closing cannot rebound and reinsert a lens.
private struct GooSpring<Content: View>: View {
  let progress: GooProgress
  @ViewBuilder var content: () -> Content
  @Environment(\.motionEnabled) private var motion

  var body: some View {
    content().animation(motion ? (progress[0] == 0 ? Motion.gooClose : Motion.gooSpring) : nil, value: progress)
  }
}

/// A chip whose beans `gooGlass()` and `gooLayer()` pull out; tapping it opens or closes them. `id` names it among the
/// composer's chips, so opening one closes the others. `stage` is the bean whose panel is open (1…, 0 for none). The
/// chip is a glass circle in the composer's container
struct GooChip<Glyph: View>: View {
  let id: String
  @Binding var open: String?
  @Binding var stage: Int
  /// The level of beans out instead of the chip's own (the sidekick's), one more level to step back from
  var deeper: Binding<String?>? = nil
  /// The beans are tucked into the chip while they trade for another level's (`LiquidControls.enter`)
  var held = false
  /// Steps back out of `deeper` (the same trade, the other way)
  var leave: (() -> Void)? = nil
  var pieces: [GooPiece]
  @ViewBuilder var glyph: () -> Glyph
  @Environment(\.motionEnabled) private var motion
  @Environment(\.gooBare) private var bare

  private var isOpen: Bool { open == id }

  var body: some View {
    let progress = GooProgress(values: [isOpen && !held ? 1 : 0] + pieces.indices.map { isOpen && stage == $0 + 1 ? 1 : 0 })
    Button {
      if isOpen { back() } else { set(id) }
    } label: {
      glyph()
        .frame(width: Metrics.barCtl, height: Metrics.barCtl)
        .contentShape(Circle())
    }
    .buttonStyle(.plain)
    // A glass circle of its own, so a press lights this chip alone. Inset so that at rest it lies wholly inside the
    // bar and the container merges it away: at full size its outline ran along the bar's round end and swelled it
    // On the card it is merged away the same way. Over the plate that stands in for the card's glass under the beans
    // the chip is a bare glyph (`gooBare`): there the circles stood out as two small buttons
    .glassSurface(bare ? .identity : GlassStyle.bar.interactive(), in: Circle().inset(by: Metrics.gooChipInset))
    // The glass's shape is also the button's hit shape, so only the inset circle (half the slot) took taps: a touch a
    // little below the glyph fell through to the bar or card under it and only focused the field. The whole slot takes
    // them again
    .contentShape(Circle())
    .anchorPreference(key: GooKey.self, value: .bounds) { anchor in
      [GooItem(id: id, anchor: anchor, progress: progress, pieces: pieces,
               seed: bare ? 0 : Metrics.barCtl - Metrics.gooChipInset * 2, close: { set(nil) }, back: back)]
    }
  }

  /// One level back: an open panel into its bean, a deeper level's beans for the chip's own, else the beans into the
  /// chip
  private func back() {
    if stage > 0 { return withAnimation(motion ? Motion.gooSpring : nil) { stage = 0 } }
    if deeper?.wrappedValue != nil, let leave { return leave() }
    set(nil)
  }

  private func set(_ now: String?) {
    withAnimation(motion ? Motion.gooSpring : nil) {
      open = now
      stage = 0
      // Opening starts on the chip's own beans; closing leaves a deeper level as it is, so it shrinks home unchanged
      if now != nil { deeper?.wrappedValue = nil }
    }
  }
}

/// A column of beans rising from a chip, each as tall as the chip
enum GooColumn {
  /// A bean's panel: its rows up to `gooPanelRows`, and no taller than `room`, the space above the chip, where it
  /// rests on the column's bottom corner, its list scrolling inside. With the keyboard up that room is short, and a
  /// panel of full height climbed under the navigation bar
  static func panelHeight(rows: Int, labels: Int = 0, room: CGFloat) -> CGFloat {
    let pad = Metrics.s1_5 * 2
    let natural = min(CGFloat(rows) * Metrics.gooRow + CGFloat(labels) * Metrics.gooSection, Metrics.gooPanelRows * Metrics.gooRow) + pad
    return max(Metrics.gooRow + pad, min(natural, room - Metrics.gooGap))
  }

  /// The coordinate space `gooLayer()` names: the screen below its bar
  nonisolated static let space = "goo.screen"

  /// The first rests a gap above the chip's top, the rest above it. Wider pieces line up on the chip's leading edge,
  /// or its trailing edge for a chip near the screen's end, so the column never leaves the screen
  static func offsets(chip: CGSize, widths: [CGFloat], edge: HorizontalEdge = .leading) -> [CGSize] {
    widths.enumerated().map { i, w in
      let dx = edge == .leading ? (w - chip.width) / 2 : (chip.width - w) / 2
      return CGSize(width: dx, height: -(chip.height + Metrics.gooGap + CGFloat(i) * (chip.height + Metrics.gooGap)))
    }
  }
}

/// A piece's caption over the dim, on a soft patch of the page's colour so it reads over the transcript in either
/// theme (a map label's halo). A note under the value says what the setting is, wrapped to the room beside the bean
struct GooCaption: View {
  var text: String
  var note: String? = nil
  var body: some View {
    VStack(alignment: .leading, spacing: Metrics.s0_5) {
      Text(web: text, paragraph: false)
        .fixedSize()
        .textStyle(TypeScale.text2, weight: .medium)
        .foregroundStyle(Palette.fgStrong)
      if let note {
        Text(web: note, paragraph: false)
          .textStyle(TypeScale.text3)
          .foregroundStyle(Palette.fg2)
          .lineLimit(2)
          .fixedSize(horizontal: false, vertical: true)
      }
    }
    .background {
      RoundedRectangle(cornerRadius: note == nil ? Metrics.barCtl : Metrics.rGooPanel, style: .continuous)
        .fill(Palette.bg0)
        .padding(-Metrics.s1)
        .blur(radius: Metrics.s2)
    }
  }
}

/// One bean of a setting chip: its glyph, the value beside it and the panel it turns into
struct GooBean {
  let key: String
  let title: String
  let glyph: AnyView
  let caption: AnyView
  var rows = 0
  /// Group labels among the rows (`GooSection`), shorter than a row
  var labels = 0
  var panel: AnyView? = nil
  /// A bean that acts in place (Fast flips) instead of opening a panel
  var tap: (() -> Void)? = nil
}

extension GooColumn {
  /// A setting chip's beans in a column above it, their values on their right; each turns into its panel, which
  /// rests on the column's bottom-left corner so a long list grows up the screen. A tap on a bean opens its panel, or
  /// folds it back
  static func pieces(_ beans: [GooBean], room: CGFloat = .infinity, motion: Bool, stage: Binding<Int>) -> [GooPiece] {
    let chip = CGSize(width: Metrics.barCtl, height: Metrics.barCtl)
    let offsets = offsets(chip: chip, widths: beans.map { _ in chip.width })
    return zip(beans, offsets).enumerated().map { k, item in
      let (b, at) = item
      if let tap = b.tap {
        return GooPiece(id: "bean.\(b.key)", name: b.title, offset: at, glyph: b.glyph, caption: b.caption, action: tap)
      }
      let h = panelHeight(rows: b.rows, labels: b.labels, room: room)
      return GooPiece(id: "bean.\(b.key)", name: b.title, offset: at, glyph: b.glyph, caption: b.caption,
                      panel: b.panel.map { GooPanel(size: CGSize(width: Metrics.gooPanelW, height: h), content: $0) }) {
        withAnimation(motion ? Motion.gooSpring : nil) {
          stage.wrappedValue = stage.wrappedValue == k + 1 ? 0 : k + 1
        }
      }
    }
  }
}

/// A bean's panel: its rows in a scroll view that opens on the current one
struct GooList<Content: View>: View {
  var current: String?
  @ViewBuilder var content: () -> Content
  @State private var at: String?

  var body: some View {
    ScrollView {
      VStack(spacing: 0) { content() }
        .scrollTargetLayout()
    }
    .scrollIndicators(.hidden)
    .scrollPosition(id: $at, anchor: .center)
    .onAppear { at = current }
  }
}

/// One choice in a bean's panel: glyph, name, and the check (or a trailing control such as Fast's switch)
struct GooRow<Glyph: View, Trailing: View>: View {
  var name: String
  var checked: Bool
  @ViewBuilder var glyph: () -> Glyph
  @ViewBuilder var trailing: () -> Trailing
  var action: () -> Void

  var body: some View {
    // Every row is a choice (a Thinking row's own switch plays the system's feedback instead)
    Button {
      Haptics.play(.pick)
      action()
    } label: {
      HStack(spacing: Metrics.s2) {
        glyph()
        Text(web: name, paragraph: false)
          .lineLimit(1)
          .frame(maxWidth: .infinity, alignment: .leading)
        trailing()
      }
      .textStyle(TypeScale.text2, weight: checked ? .semibold : .regular)
      .foregroundStyle(checked ? Palette.fgStrong : Palette.fg2)
      .padding(.horizontal, Metrics.gooPadX)
      .frame(height: Metrics.gooRow)
      .contentShape(Rectangle())
    }
    .buttonStyle(.plain)
  }
}

extension GooRow where Trailing == GooCheck {
  init(name: String, checked: Bool, @ViewBuilder glyph: @escaping () -> Glyph, action: @escaping () -> Void) {
    self.init(name: name, checked: checked, glyph: glyph, trailing: { GooCheck(on: checked) }, action: action)
  }
}

/// A label over a run of rows in a bean's panel, the channel they come through (the desktop's group label): fainter
/// and smaller than a row, on the rows' inset, sitting on the first row it names
struct GooSection: View {
  var title: String

  var body: some View {
    Text(web: title, paragraph: false)
      .lineLimit(1)
      .textStyle(TypeScale.text3)
      .foregroundStyle(Palette.fg3)
      .frame(maxWidth: .infinity, alignment: .leading)
      .padding(.horizontal, Metrics.gooPadX)
      .frame(height: Metrics.gooSection, alignment: .bottom)
      .accessibilityAddTraits(.isHeader)
  }
}

/// The current choice's check
struct GooCheck: View {
  var on: Bool
  var body: some View {
    Icon(.check, size: Metrics.icon, strokeWidth: 2).foregroundStyle(Palette.fgStrong).opacity(on ? 1 : 0)
  }
}

/// The Fast bolt: outlined, filled in --fast while Fast is on
struct FastGlyph: View {
  var on: Bool
  var size: CGFloat = Metrics.chevron
  var body: some View {
    ZStack {
      if on { IconShape(icon: .zap).fill(Palette.fast) }
      Icon(.zap, size: size, strokeWidth: 2).foregroundStyle(on ? Palette.fast : .primary)
    }
    .frame(width: size, height: size)
  }
}

extension EnvironmentValues {
  /// The chat agent's hidden model families by option id (settings → Agents → Models), for the model chip's lists
  @Entry var hiddenModels: [String: [String]] = [:]
  /// The chips sit bare on the composer's plate: no glass circle of their own, and their beans grow out of nothing
  @Entry var gooBare = false
}
