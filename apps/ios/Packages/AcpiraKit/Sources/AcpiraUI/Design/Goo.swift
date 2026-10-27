import AcpiraDesign
import AcpiraModels
import SwiftUI
#if canImport(UIKit)
import UIKit
#endif

// Design study `chrome=system` (2026-10-26): liquid pickers after libraries.dev/gooey. A chip sits in the composer's
// glass group; opening it pushes a column of beans out of it, each bean squeezing out of the one before (a telescope),
// and closing pulls them back in, the outermost first. A bean may turn into a panel of choices: the tapped bean
// stretches into the panel (its bottom-left corner stays put) and the other beans shrink into it, and a pick shrinks
// the panel back into the bean. The chip is the way back: × while the beans are out, ‹ while a panel is, one level per
// tap; a tap anywhere else closes everything, as outside a system menu. One spring drives a progress vector: the chip's own value, then one
// per bean that opens a panel.
//
// Every silhouette is Liquid Glass in the composer's own `GlassEffectContainer` (`gooGlass()` on the group), so a
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
  /// Whether the bean and its caption, or its panel, take taps
  var live: Bool
  var panelLive: Bool
}

/// The one geometry the composer's glass and the screen's rows both draw from
enum GooGeometry {
  /// Under this the picker is closed and nothing is drawn
  static let gone: CGFloat = 0.001
  /// A bean's glyph shows over the last part of its way out
  private static let glyphFrom: CGFloat = 0.5, glyphSpan: CGFloat = 0.4
  /// Captions come last on the way out and go first on the way in (the chip's own value has to be nearly 1)
  private static let captionFrom: CGFloat = 0.7, captionMain: CGFloat = 0.85
  /// A panel's rows show once it is mostly grown; the other beans are gone by `swallow` of the way
  private static let rowsFrom: CGFloat = 0.55, rowsSpan: CGFloat = 0.35, swallow: CGFloat = 0.6
  /// A glyph leaves its bean for the panel early, and is gone well before a swallowed bean is; a caption goes as a
  /// panel starts and comes back once the beans have nearly grown back
  private static let glyphOut: CGFloat = 0.3, glyphSwallowed: CGFloat = 0.35, captionOut: CGFloat = 0.1
  /// Taps land once a bean or panel has arrived, and on the beans only once no panel is left over them
  private static let arrived: CGFloat = 0.9, uncovered: CGFloat = 0.05

  static func frames(_ pieces: [GooPiece], _ progress: GooProgress, chip: CGSize) -> [GooFrame] {
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
      // The bean, or its panel part of the way out, then shrunk into the panel that swallows it
      var centre = at
      var size = chip
      var radius = chip.height / 2
      let own = piece.panel.map(rest)
      if let rect = own, s > 0 {
        centre = CGPoint(x: at.x + (rect.midX - at.x) * s, y: at.y + (rect.midY - at.y) * s)
        size = CGSize(width: chip.width + (rect.width - chip.width) * s, height: chip.height + (rect.height - chip.height) * s)
        radius = chip.height / 2 + (Metrics.rGooPanel - chip.height / 2) * unit(s)
      }
      if let (h, panel) = host, h != i {
        let rect = rest(panel)
        centre = CGPoint(x: centre.x + (rect.midX - centre.x) * swallowed, y: centre.y + (rect.midY - centre.y) * swallowed)
      }
      var f = GooFrame(centre: centre, size: CGSize(width: size.width * scale, height: size.height * scale),
                       radius: radius * scale, glyph: 0, scale: scale, caption: 0, panel: own, rows: 0,
                       live: t > arrived && opened < uncovered, panelLive: s > arrived)
      let kept = 1 - unit(swallowed / glyphSwallowed)
      f.glyph = Double(unit((t - glyphFrom) / glyphSpan) * (1 - unit(s / glyphOut)) * kept)
      f.caption = Double(min(unit((t - captionFrom) / (1 - captionFrom)), unit((main - captionMain) / (1 - captionMain)))
                         * (1 - unit(opened / captionOut)))
      f.rows = Double(unit((s - rowsFrom) / rowsSpan) * kept)
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

  var animatableData: GooProgress {
    get { progress }
    set { progress = newValue }
  }

  var body: some View {
    let frames = GooGeometry.frames(pieces, progress, chip: chip.size)
    ZStack(alignment: .topLeading) {
      if progress[0] > GooGeometry.gone {
        ForEach(pieces.indices, id: \.self) { i in
          let f = frames[i]
          if f.size.width > Metrics.s0_5 {
            pieces[i].glyph
              .scaleEffect(f.scale)
              .opacity(f.glyph)
              .frame(width: f.size.width, height: f.size.height)
              .glassEffect(GlassStyle.control, in: RoundedRectangle(cornerRadius: f.radius, style: .continuous))
              // A bean comes and goes inside the chip, where nothing shows: the system's own morph on insertion would
              // run its own clock over the spring
              .glassEffectTransition(.identity)
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
  var chip: CGRect
  var pieces: [GooPiece]
  var close: () -> Void
  var back: () -> Void

  var animatableData: GooProgress {
    get { progress }
    set { progress = newValue }
  }

  var body: some View {
    let frames = GooGeometry.frames(pieces, progress, chip: chip.size)
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
        ForEach(pieces.indices, id: \.self) { i in
          let piece = pieces[i]
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
            Button(action: piece.action) {
              caption.frame(height: chip.height).contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .frame(width: Metrics.gooAside, alignment: .leading)
            .opacity(f.caption)
            .position(x: c.x + chip.width / 2 + Metrics.s2 + Metrics.gooAside / 2, y: c.y)
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
  }
}

/// What a chip publishes: where it is, how far open, its beans, and how to close it or step back one level
struct GooItem {
  let id: String
  let anchor: Anchor<CGRect>
  let progress: GooProgress
  let pieces: [GooPiece]
  let close: () -> Void
  let back: () -> Void
}

struct GooKey: PreferenceKey {
  static let defaultValue: [GooItem] = []
  static func reduce(value: inout [GooItem], nextValue: () -> [GooItem]) { value += nextValue() }
}

extension View {
  /// Draws the glass of the liquid pickers published inside this view; applied to the glass the chips sit on, inside
  /// its `GlassEffectContainer`, so the beans melt out of it
  func gooGlass() -> some View {
    overlayPreferenceValue(GooKey.self) { items in
      GeometryReader { proxy in
        ForEach(items, id: \.id) { item in
          GooSpring(progress: item.progress) {
            GooGlass(progress: item.progress, chip: proxy[item.anchor], pieces: item.pieces)
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
            GooControls(progress: item.progress, chip: proxy[item.anchor], pieces: item.pieces, close: item.close,
                        back: item.back)
          }
        }
      }
    }
  }
}

/// Springs its content to a picker's progress: the glass and the rows each run the same spring from the same change
private struct GooSpring<Content: View>: View {
  let progress: GooProgress
  @ViewBuilder var content: () -> Content
  @Environment(\.motionEnabled) private var motion

  var body: some View {
    content().animation(motion ? Motion.gooSpring : nil, value: progress)
  }
}

/// A chip whose beans `gooGlass()` and `gooLayer()` pull out; tapping it opens or closes them. `id` names it among the
/// composer's chips, so opening one closes the others. `stage` is the bean whose panel is open (1…, 0 for none). The
/// chip is a glass circle in the composer's container
struct GooChip<Glyph: View>: View {
  let id: String
  @Binding var open: String?
  @Binding var stage: Int
  var pieces: [GooPiece]
  @ViewBuilder var glyph: () -> Glyph
  @Environment(\.motionEnabled) private var motion

  private var isOpen: Bool { open == id }

  var body: some View {
    let progress = GooProgress(values: [isOpen ? 1 : 0] + pieces.indices.map { isOpen && stage == $0 + 1 ? 1 : 0 })
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
    .glassEffect(GlassStyle.control.interactive(), in: Circle().inset(by: Metrics.gooChipInset))
    .anchorPreference(key: GooKey.self, value: .bounds) { anchor in
      [GooItem(id: id, anchor: anchor, progress: progress, pieces: pieces, close: { set(nil) }, back: back)]
    }
  }

  /// One level back: an open panel into its bean, else the beans into the chip
  private func back() {
    guard stage > 0 else { return set(nil) }
    withAnimation(motion ? Motion.gooSpring : nil) { stage = 0 }
  }

  private func set(_ now: String?) {
    withAnimation(motion ? Motion.gooSpring : nil) {
      open = now
      stage = 0
    }
  }
}

/// A column of beans rising from a chip, each as tall as the chip
enum GooColumn {
  /// The first rests a gap (and `lift`) above the chip's top, the rest above it. Wider pieces line up on the chip's
  /// leading edge, or its trailing edge for a chip near the screen's end, so the column never leaves the screen
  static func offsets(chip: CGSize, widths: [CGFloat], edge: HorizontalEdge = .leading, lift: CGFloat = 0) -> [CGSize] {
    widths.enumerated().map { i, w in
      let dx = edge == .leading ? (w - chip.width) / 2 : (chip.width - w) / 2
      return CGSize(width: dx, height: -(chip.height + Metrics.gooGap + lift + CGFloat(i) * (chip.height + Metrics.gooGap)))
    }
  }
}

// MARK: - The composer's liquid controls

/// Attach as a liquid chip: the photo, camera and file sources squeeze out of `+` in a short column, each named beside
/// it; the `+` turns ×. `lift` raises the column over the card the chip sits in while typing
struct LiquidAttach: View {
  @Binding var open: String?
  var lift: CGFloat = 0
  var onPick: (AttachSource) -> Void
  @Environment(\.motionEnabled) private var motion

  var body: some View {
    let sources: [(AttachSource, LucideIcon, String)] = [
      (.photos, .image, t("mobile.attach.photos")), (.camera, .camera, t("mobile.attach.camera")),
      (.files, .paperclip, t("mobile.attach.files")),
    ]
    let chip = CGSize(width: Metrics.barCtl, height: Metrics.barCtl)
    let offsets = GooColumn.offsets(chip: chip, widths: sources.map { _ in chip.width }, lift: lift)
    GooChip(id: "attach", open: $open, stage: .constant(0), pieces: zip(sources, offsets).map { item, offset in
      let (source, icon, label) = item
      return GooPiece(id: "attach.\(label)", name: label, offset: offset,
                      glyph: AnyView(Icon(icon, size: Metrics.barIcon, strokeWidth: 1.75).foregroundStyle(Palette.fg1)),
                      caption: AnyView(GooCaption(text: label))) {
        withAnimation(motion ? Motion.gooSpring : nil) { open = nil }
        onPick(source)
      }
    }) {
      Icon(.plus, size: Metrics.barIcon, strokeWidth: 1.75)
        .foregroundStyle(Palette.fg1)
        .rotationEffect(.degrees(open == "attach" ? 45 : 0))
        .accessibilityLabel(Text(web: t("mobile.attach")))
    }
  }
}

/// A piece's caption over the dim, on a soft patch of the page's colour so it reads over the transcript in either
/// theme (a map label's halo)
struct GooCaption: View {
  var text: String
  var body: some View {
    Text(web: text, paragraph: false)
      .fixedSize()
      .textStyle(TypeScale.text2, weight: .medium)
      .foregroundStyle(Palette.fgStrong)
      .background {
        Capsule()
          .fill(Palette.bg0)
          .padding(-Metrics.s1)
          .blur(radius: Metrics.s2)
      }
  }
}

/// The model button as one liquid chip (the model's mark, large, beside `+`). Opening it squeezes three beans out of
/// it in a column: the model, the reasoning level and the special modes (the agent's modes, Fast), each with its
/// value on its right, and the mark turns into × (the chip has become the beans, as `+` does). A tapped bean
/// stretches into its panel of choices and swallows the others; a pick shrinks it back into the bean and leaves the
/// beans out, so another setting can change next. Fast flips in place
struct LiquidControls: View {
  let controls: SessionControls
  @Binding var open: String?
  /// The bean whose panel is open (1…3), 0 for none
  @Binding var stage: Int
  var lift: CGFloat = 0
  var onSetMode: (String) -> Void
  var onSetConfig: (String, String) -> Void
  @Environment(\.motionEnabled) private var motion

  /// One bean: its glyph, the value beside it and the panel it turns into
  private struct Bean {
    let key: String
    let title: String
    let glyph: AnyView
    let caption: AnyView
    let rows: Int
    let panel: AnyView
  }

  var body: some View {
    let c = ModelChoices(controls: controls, onSetConfig: onSetConfig)
    let beans = beans(c)
    if !beans.isEmpty {
      GooChip(id: "model", open: $open, stage: $stage, pieces: pieces(beans)) {
        // The model's mark alone, large; Fast shows as a bolt on its shoulder. Open, it is the way back: × over the
        // beans, ‹ over a panel
        ZStack {
          if open == "model" {
            Icon(stage > 0 ? .chevronLeft : .x, size: Metrics.barIcon, strokeWidth: 1.75)
              .id(stage > 0)
              .transition(.scale(scale: Metrics.gooSeed).combined(with: .opacity))
          } else {
            ChipMark(cur: c.cur, fast: c.fast?.on == true)
              .transition(.scale(scale: Metrics.gooSeed).combined(with: .opacity))
          }
        }
        .foregroundStyle(Palette.fg1)
        // The mark leaves at once (the first bean carries it up) and comes back as the last drop lands
        .animation(motion ? open == "model" ? Motion.gooSwap : Motion.gooSwapBack : nil, value: open == "model" ? stage + 1 : 0)
        .accessibilityLabel(Text(web: open == "model" ? t(stage > 0 ? "common.back" : "common.close") : c.cur?.name ?? t("stats.model")))
      }
    }
  }

  /// The beans in a column above the chip, their values on their right; each turns into its panel, which rests on
  /// the column's bottom-left corner so a long list grows up the screen
  private func pieces(_ beans: [Bean]) -> [GooPiece] {
    let chip = CGSize(width: Metrics.barCtl, height: Metrics.barCtl)
    let offsets = GooColumn.offsets(chip: chip, widths: beans.map { _ in chip.width }, lift: lift)
    return zip(beans, offsets).enumerated().map { k, item in
      let (b, at) = item
      let h = min(CGFloat(b.rows), Metrics.gooPanelRows) * Metrics.gooRow + Metrics.s1_5 * 2
      return GooPiece(id: "bean.\(b.key)", name: b.title, offset: at, glyph: b.glyph, caption: b.caption,
                      panel: GooPanel(size: CGSize(width: Metrics.gooPanelW, height: h), content: b.panel)) {
        withAnimation(motion ? Motion.gooSpring : nil) { stage = stage == k + 1 ? 0 : k + 1 }
      }
    }
  }

  private func beans(_ c: ModelChoices) -> [Bean] {
    let families = c.families
    let effort = c.effort
    let mode = c.mode
    var out: [Bean] = []
    if let cur = c.cur, c.modelId != nil {
      out.append(Bean(key: "model", title: t("stats.model"),
                      glyph: AnyView(VendorMark(brand: cur.brand, name: cur.name, size: Metrics.barIcon).foregroundStyle(Palette.fg1)),
                      caption: AnyView(GooCaption(text: cur.name)), rows: families.count,
                      panel: AnyView(GooList(current: cur.key) {
                        ForEach(families, id: \.key) { f in
                          GooRow(name: f.name, checked: f.key == cur.key) {
                            VendorMark(brand: f.brand, name: f.name, size: Metrics.iconCtl)
                          } action: {
                            c.pickFamily(f)
                            fold()
                          }
                          .id(f.key)
                        }
                      })))
    }
    if let effort, effort.levels.count > 1 {
      let name = effort.levels.first { $0.id == effort.value }?.name ?? ""
      out.append(Bean(key: "effort", title: t("composer.effort"),
                      glyph: AnyView(Icon(.brain, size: Metrics.barIcon, strokeWidth: 1.75).foregroundStyle(Palette.fg1)),
                      caption: AnyView(GooCaption(text: name)), rows: effort.levels.count,
                      panel: AnyView(GooList(current: effort.value) {
                        ForEach(effort.levels, id: \.id) { level in
                          GooRow(name: level.name, checked: level.id == effort.value) { EmptyView() } action: {
                            effort.pick(level.id)
                            fold()
                          }
                          .id(level.id)
                        }
                      })))
    }
    let modes = c.modes
    let fast = c.fast
    if !modes.isEmpty || fast != nil {
      out.append(Bean(key: "mode", title: t("composer.mode"),
                      glyph: AnyView(Group {
                        if let mode, !modes.isEmpty {
                          Icon(modeIcon(mode), size: Metrics.barIcon, strokeWidth: 1.75)
                            .foregroundStyle(mode.kind == "full_access" ? Palette.warn : Palette.fg1)
                        } else {
                          FastGlyph(on: fast?.on == true, size: Metrics.barIcon)
                        }
                      }),
                      caption: AnyView(HStack(spacing: Metrics.s1_5) {
                        if fast?.on == true { FastGlyph(on: true, size: Metrics.icon) }
                        if let mode, !modes.isEmpty { GooCaption(text: modeLabel(mode)) }
                        if modes.isEmpty, fast?.on != true { GooCaption(text: "Fast") }
                      }),
                      rows: modes.count + (fast != nil ? 1 : 0),
                      panel: AnyView(GooList(current: mode?.id) {
                        if let fast {
                          // Fast flips in place: the panel stays out
                          GooRow(name: "Fast", checked: fast.on) { FastGlyph(on: fast.on, size: Metrics.icon) } trailing: {
                            Toggle("", isOn: Binding(get: { fast.on }, set: { fast.set($0) }))
                              .labelsHidden()
                              // The system's green, as the settings switch
                              .tint(.green)
                              .disabled(fast.disabled)
                          } action: { if !fast.disabled { fast.set(!fast.on) } }
                        }
                        ForEach(modes) { m in
                          GooRow(name: modeLabel(m), checked: m.id == mode?.id) {
                            Icon(modeIcon(m), size: Metrics.icon, strokeWidth: 1.75)
                              .foregroundStyle(m.kind == "full_access" ? Palette.warn : Palette.fg1)
                          } action: {
                            onSetMode(m.id)
                            fold()
                          }
                          .id(m.id)
                        }
                      })))
    }
    return out
  }

  /// A pick folds the panel back into its bean; the beans stay out for the next change
  private func fold() { withAnimation(motion ? Motion.gooSpring : nil) { stage = 0 } }
}

/// What the composer's model chip offers, read from the session's controls: the model families, the reasoning levels
/// with Fast, and the modes. The liquid chip and the system menu (`chips=menu`) both draw from it
struct ModelChoices {
  /// The reasoning levels on offer (the family's own scale, else the agent's thought_level) and Fast beside them
  typealias Effort = (levels: [(id: String, name: String)], value: String, pick: (String) -> Void, fast: FastState?)

  let families: [ModelFamily]
  let cur: ModelFamily?
  let curVar: ModelVariant?
  /// The config option holding the model, when the agent offers one
  let modelId: String?
  let effort: Effort?
  /// The mode in force (the agent's first until one is set)
  let mode: SessionOption?
  /// The modes to choose between; none when the agent has only one
  let modes: [SessionOption]
  private let onSetConfig: (String, String) -> Void

  init(controls: SessionControls, onSetConfig: @escaping (String, String) -> Void) {
    let split = ComposerControls.split(controls.options)
    let model = split.models.first
    let families = model.map { ModelCatalog.group($0.options) } ?? []
    let cur = families.first { f in f.variants.contains { $0.id == model?.value } }
    let curVar = cur?.variants.first { $0.id == model?.value }
    self.families = families
    self.cur = cur
    self.curVar = curVar
    self.modelId = model?.id
    self.effort = Self.effort(split: split, cur: cur, curVar: curVar, modelId: model?.id, onSetConfig: onSetConfig)
    self.mode = controls.modes.first { $0.id == controls.modeId } ?? controls.modes.first
    self.modes = controls.modes.count > 1 ? controls.modes : []
    self.onSetConfig = onSetConfig
  }

  /// Fast, when the model in use can switch it at the level in force, or while it is on (the bolt on the chip says
  /// so). A family with Fast at other levels only offers none here, rather than a switch that cannot move
  var fast: FastState? { effort?.fast.flatMap { $0.disabled && !$0.on ? nil : $0 } }

  private static func effort(split: ComposerControls.Split, cur: ModelFamily?, curVar: ModelVariant?, modelId: String?,
                             onSetConfig: @escaping (String, String) -> Void) -> Effort? {
    let nativeFast = split.modelConfig.first(where: ComposerControls.isFast).map { c in nativeFastState(c) { onSetConfig(c.id, $0) } }
    if let f = cur, let v = curVar, let modelId, f.fusion == nil, ModelCatalog.variantScale(f) {
      let fast: FastState? = f.hasFast ? {
        func at(_ long: Bool) -> ModelVariant? { f.variants.first { $0.effort == v.effort && $0.fast == !v.fast && $0.long == long } }
        let hit = at(false) ?? at(v.long)
        return FastState(on: v.fast, disabled: hit == nil) { _ in if let hit { onSetConfig(modelId, hit.id) } }
      }() : nativeFast
      return (f.efforts.map { ($0, $0.isEmpty ? t("composer.standard") : $0) }, v.effort, { e in
        if let hit = ModelCatalog.findVariant(f, effort: e, fast: v.fast, long: false) { onSetConfig(modelId, hit.id) }
      }, fast)
    }
    if let c = split.reasoning.first(where: ComposerControls.reasoningVisible) {
      let p = ComposerControls.presentReasoning(c)
      guard p.efforts.count > 1 else { return nil }
      return (p.efforts.map { ($0.id, $0.name) }, p.off ? "" : p.value ?? "", { onSetConfig(c.id, $0) }, nativeFast)
    }
    return nil
  }

  /// ModelPanel.pickFamily: into Fusion the model in use becomes the lead; out of it the lead's effort carries over
  func pickFamily(_ f: ModelFamily) {
    guard let modelId else { return }
    let next = f.fusion != nil
      ? ModelCatalog.findFusionVariant(f, lead: curVar?.lead ?? cur?.name ?? "", effort: curVar?.effort ?? "", sidekick: curVar?.sidekick, fast: curVar?.fast ?? false)
      : curVar.flatMap { ModelCatalog.findVariant(f, effort: $0.effort, fast: $0.fast, long: false) }
    if let id = (next ?? f.variants.first)?.id { onSetConfig(modelId, id) }
  }
}

/// The model chip at rest: the model's mark, large, with Fast as a bolt on its shoulder (a sliders glyph when the
/// agent names no model)
struct ChipMark: View {
  var cur: ModelFamily?
  var fast: Bool

  var body: some View {
    Group {
      if let cur {
        VendorMark(brand: cur.brand, name: cur.name, size: Metrics.barIcon)
      } else {
        Icon(.slidersHorizontal, size: Metrics.barIcon, strokeWidth: 1.75)
      }
    }
    .overlay(alignment: .topTrailing) {
      if fast { FastGlyph(on: true, size: Metrics.chevron).offset(x: Metrics.s1, y: -Metrics.s1) }
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
    Button(action: action) {
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

/// The current choice's check
struct GooCheck: View {
  var on: Bool
  var body: some View {
    Icon(.check, size: Metrics.icon, strokeWidth: 2).foregroundStyle(Palette.fgStrong).opacity(on ? 1 : 0)
  }
}
