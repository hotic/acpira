import AcpiraDesign
import AcpiraModels
import SwiftUI
#if canImport(UIKit)
import UIKit
#endif

// One floating bar between the session list and a page with a composer (a chat, the new session). The list's search
// field and new session sit on the system bottom bar, the composer is the page's own glass bar, and each went out and
// came in with its page: two pieces sliding past each other. Across the push or pop between the two, a stand-in drawn over the whole stack takes
// both places instead and turns one into the other in step with the pages, the finger's drag included: the composer's
// bar draws back from its trailing end and leaves send behind as the new-session circle, while the composer's face
// gives way to the search field's. Both real bars stay hidden under it until the transition has ended.

/// The hand-over between a page's composer and the list's bottom bar (`BarHandoffLayer` draws it, `BarHandoffProbe`
/// drives it)
@Observable
final class BarHandoff {
  /// How far the bar has turned: 0 the composer, 1 the list's bar; nil while no hand-over runs. Written every frame,
  /// so only the stand-in reads it
  var progress: CGFloat?
  /// The session whose composer the stand-in replaces (a chat, or the new session's draft); its page hides the composer
  /// meanwhile
  var hidden: String?
  /// That session as its composer drew it when the hand-over began, and the composer's hint at rest (the page's own
  /// placeholder, nil for the composer's default)
  var face: SessionView?
  var placeholder: String?
  /// The composer's bar, the list's search capsule and its new-session circle, in window coordinates
  var composer: CGRect = .zero
  var search: CGRect = .zero
  var new: CGRect = .zero
  /// Where the search field's glyph and hint sit in its capsule, as the system bar last laid them out (iOS 27's until
  /// then)
  var searchGlyph = CGRect(x: 12, y: 14, width: 20.67, height: 19.33)
  var searchHint = CGRect(x: 40.67, y: 14, width: 151.67, height: 20.33)
  /// The new-session button's glyph as the system bar last drew it, at its window frame, for the light or dark
  /// appearance it was taken in
  var newFace: BarFace?
  /// The hand-over under way (`HandoffRun`), kept here rather than by the page, which a pop releases before the
  /// stand-in has given way
  @ObservationIgnored var run: AnyObject?
  /// The container the system draws the bars in, as last found
  @ObservationIgnored weak var barHost: AnyObject?

  /// The two glass pieces at a progress. The bar draws back from its trailing end ahead of the rest, so send parts from
  /// it as a drop in the middle of the move rather than at its very end
  func pieces(_ p: CGFloat) -> (bar: CGRect, circle: CGRect) {
    let send = CGRect(x: composer.maxX - composer.height, y: composer.minY, width: composer.height, height: composer.height)
    let trail = BarHandoff.ease(min(1, p / BarHandoff.trailLead))
    let minX = lerp(composer.minX, search.minX, p)
    let maxX = lerp(composer.maxX, search.maxX, trail)
    let minY = lerp(composer.minY, search.minY, p)
    let maxY = lerp(composer.maxY, search.maxY, p)
    let bar = CGRect(x: minX, y: minY, width: max(maxX - minX, maxY - minY), height: maxY - minY)
    return (bar, lerp(send, new, p))
  }

  /// The share of the move in which the bar's trailing end reaches the search field's
  static let trailLead: CGFloat = 0.75

  static func ease(_ t: CGFloat) -> CGFloat { 1 - (1 - t) * (1 - t) }

  /// 0 below `from`, 1 above `to`, smooth between
  static func step(_ from: CGFloat, _ to: CGFloat, _ x: CGFloat) -> CGFloat {
    let t = min(1, max(0, (x - from) / (to - from)))
    return t * t * (3 - 2 * t)
  }
}

/// A picture of one of the system bar's contents and where it stood
struct BarFace {
  let image: Image
  let frame: CGRect
  let dark: Bool
}

private func lerp(_ a: CGFloat, _ b: CGFloat, _ t: CGFloat) -> CGFloat { a + (b - a) * t }

private func lerp(_ a: CGRect, _ b: CGRect, _ t: CGFloat) -> CGRect {
  CGRect(x: lerp(a.minX, b.minX, t), y: lerp(a.minY, b.minY, t), width: lerp(a.width, b.width, t),
         height: lerp(a.height, b.height, t))
}

extension EnvironmentValues {
  /// The composer draws its face alone, without its glass: the hand-over's stand-in lays it over glass of its
  /// own
  @Entry var composerBare = false
}

/// The stand-in over the whole stack while a hand-over runs: two glass pieces in one container, so the bar and the
/// circle melt apart, the composer's face fading out of them and the list's fading in
struct BarHandoffLayer: View {
  @Environment(BarHandoff.self) private var handoff
  @Environment(\.colorScheme) private var scheme

  var body: some View {
    if let p = handoff.progress, let session = handoff.face {
      let (bar, circle) = handoff.pieces(p)
      // Both ends are the same glass (`GlassStyle.bar`), so only the shape and what lies on it change
      let out = BarHandoff.step(0, 0.45, p)
      let into = BarHandoff.step(0.4, 0.9, p)
      ZStack(alignment: .topLeading) {
        GlassEffectContainer(spacing: Metrics.gooMerge) {
          ZStack(alignment: .topLeading) {
            piece(Capsule(), bar) {
              searchFace.opacity(into).blur(radius: Metrics.s1 * (1 - into))
            }
            piece(Circle(), circle) { EmptyView() }
          }
        }
        ComposerFace(session: session, placeholder: handoff.placeholder)
          .frame(width: handoff.composer.width, height: handoff.composer.height)
          .position(x: handoff.composer.midX, y: handoff.composer.midY)
          .opacity(1 - out)
          .blur(radius: Metrics.s1 * out)
        newFace(circle: circle)
          .opacity(into)
          .blur(radius: Metrics.s1 * (1 - into))
      }
      .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
      .ignoresSafeArea()
      .allowsHitTesting(false)
      .accessibilityHidden(true)
    }
  }

  /// A glass piece with what lies on it, from its leading end
  private func piece<S: Shape, Face: View>(_ shape: S, _ frame: CGRect, @ViewBuilder face: () -> Face) -> some View {
    // A clear base, so the circle with nothing on it still has a body for its glass
    Color.clear
      .overlay(alignment: .topLeading) { face() }
      .frame(width: frame.width, height: frame.height)
      .clipShape(shape)
      .glassEffect(GlassStyle.bar, in: shape)
      .glassEffectTransition(.identity)
      .position(x: frame.midX, y: frame.midY)
  }

  /// The search field's glyph and hint, at their places from the capsule's leading end. Drawn on the glass rather than
  /// pictured: the system field's colours are the glass's own treatment of its content (its hint's layer holds white
  /// text), which the same hierarchical styles get here. The type is the field's, medium body; the hint is plain text,
  /// as the field sets it
  private var searchFace: some View {
    let glyph = handoff.searchGlyph
    let hint = handoff.searchHint
    return ZStack(alignment: .topLeading) {
      Image(systemName: "magnifyingglass")
        .font(.body.weight(.medium))
        .imageScale(.medium)
        .foregroundStyle(.primary)
        .frame(width: glyph.width, height: glyph.height)
        .offset(x: glyph.minX, y: glyph.minY)
      Text(verbatim: t("session.searchContent"))
        .font(.body.weight(.medium))
        .foregroundStyle(.secondary)
        .lineLimit(1)
        .fixedSize()
        .frame(height: hint.height)
        .offset(x: hint.minX, y: hint.minY)
    }
  }

  /// The new-session glyph's picture, centred in the circle
  @ViewBuilder private func newFace(circle: CGRect) -> some View {
    if let face = handoff.newFace, face.dark == (scheme == .dark) {
      face.image.resizable()
        .frame(width: face.frame.width, height: face.frame.height)
        .position(x: face.frame.midX + circle.midX - handoff.new.midX, y: face.frame.midY + circle.midY - handoff.new.midY)
    } else {
      Image(systemName: "square.and.pencil").font(.body)
        .position(x: circle.midX, y: circle.midY)
    }
  }
}

/// The composer as it rests, drawn bare: the same view, so the stand-in starts exactly as the bar it replaces. Its
/// inputs stay the same value through the hand-over, so the stand-in's every-frame redraw leaves it be
private struct ComposerFace: View {
  let session: SessionView
  let placeholder: String?
  @Environment(AppModel.self) private var model

  var body: some View {
    Composer(session: session, text: .constant(""), staged: .constant([]), placeholder: placeholder)
      .environment(\.composerBare, true)
      // A text in place of the field, and no motion of its own
      .environment(\.snapshot, true)
      .environment(\.motionEnabled, false)
      .environment(\.initialPopup, nil)
      // As the chat sets it: a view carried over from the last wire rests until this one shows it
      .environment(\.syncing, !model.current || !model.shown(session.id))
      .environment(\.hiddenModels, model.settings?.hiddenOptions[session.agent] ?? [:])
  }
}

#if os(iOS)
/// Sits behind a page's composer (a chat's, the new session's) and hands it over when the page is pushed from the list
/// or popped back to it. A child of the page, it hears the page's appearance calls, and through the transition's
/// coordinator it moves a probe view alongside the pages, whose presentation layer then reads how far they have gone:
/// a drag's progress and a cancelled drag's way back included
struct BarHandoffProbe: UIViewControllerRepresentable {
  let session: SessionView
  let placeholder: String?
  @Environment(BarHandoff.self) private var handoff: BarHandoff?

  func makeUIViewController(context: Context) -> BarHandoffController { BarHandoffController() }

  func updateUIViewController(_ controller: BarHandoffController, context: Context) {
    controller.session = session
    controller.placeholder = placeholder
    controller.handoff = handoff
  }
}

final class BarHandoffController: UIViewController {
  var session: SessionView?
  var placeholder: String?
  weak var handoff: BarHandoff?
  /// Animated from 0 to 1 alongside the transition
  let driver = UIView()

  override func loadView() {
    view = UIView()
    view.isUserInteractionEnabled = false
    driver.isUserInteractionEnabled = false
    view.addSubview(driver)
  }

  override func viewWillAppear(_ animated: Bool) {
    super.viewWillAppear(animated)
    begin(toList: false)
  }

  override func viewWillDisappear(_ animated: Bool) {
    super.viewWillDisappear(animated)
    begin(toList: true)
  }

  /// The composer's bar where it rests, or nil while it is the card (a draft, staged files, an edit) or not laid out
  /// yet: only the bar is handed over
  var bar: CGRect? {
    guard view.window != nil, view.bounds.height > 0, view.bounds.height <= Metrics.barCtl + Metrics.hairline else { return nil }
    return view.convert(view.bounds, to: nil)
  }

  private func begin(toList: Bool) {
    guard let handoff, let session, let coordinator = transitionCoordinator, coordinator.isAnimated,
          let nav = navigationController, let root = nav.viewControllers.first,
          coordinator.viewController(forKey: toList ? .to : .from) === root,
          // A search under way: the list's bar is then the search field's own, over the keyboard
          root.navigationItem.searchController?.isActive != true,
          root.navigationItem.searchController?.searchBar.text?.isEmpty != false else { return }
    // A pop starts with the chat in place; a pushed chat stands where it lands only once the transition is set up
    var composer: CGRect?
    if toList {
      guard let at = bar else { return }
      composer = at
    }
    (handoff.run as? HandoffRun)?.cancel()
    let run = HandoffRun(handoff: handoff, probe: self, nav: nav, toList: toList)
    handoff.run = run
    run.start(coordinator: coordinator, composer: composer, placeholder: placeholder, session: session,
              dark: traitCollection.userInterfaceStyle == .dark)
  }
}

/// One hand-over, from the transition's start until the stand-in gives way
final class HandoffRun: NSObject {
  private weak var handoff: BarHandoff?
  private weak var probe: BarHandoffController?
  private weak var nav: UINavigationController?
  private let toList: Bool
  private var link: CADisplayLink?
  /// The system bars' container, its bottom band masked away under the stand-in, and the mask it had before
  private weak var masked: UIView?
  private var maskedBefore: UIView?
  /// The composer's place still to be read (a pushed chat, once it stands where it lands)
  private var placing = false
  /// The transition is over; the stand-in waits on the list until this time for the system bar's entrance
  private var holdUntil: CFTimeInterval?
  /// The transition ended on the chat: the stand-in ends once it has landed on the composer
  private var landing = false
  private var done = false
  /// The progress shown, eased after the pages' (`target`) rather than jumping with it. A push's first frames find the
  /// main thread still building the chat while the render server already moves the pages, which are well on their way
  /// by the next frame; followed exactly, the bar leapt most of the way in one frame
  private var shown: CGFloat = 0
  private var target: CGFloat = 0
  private var last: CFTimeInterval = 0
  /// The easing's time constant: a drag's progress is followed within a frame or two
  private static let follow: CFTimeInterval = 0.06

  init(handoff: BarHandoff, probe: BarHandoffController, nav: UINavigationController, toList: Bool) {
    self.handoff = handoff
    self.probe = probe
    self.nav = nav
    self.toList = toList
  }

  func start(coordinator: UIViewControllerTransitionCoordinator, composer: CGRect?, placeholder: String?,
             session: SessionView, dark: Bool) {
    guard let handoff, let probe, let nav else { return }
    if let composer { handoff.composer = composer } else { placing = true }
    if let bar = Self.listBar(nav) {
      handoff.search = bar.search.frame
      handoff.new = bar.new.frame
      // A push starts on the list: its contents as the system draws them now
      if !toList { Self.takeFaces(bar, into: handoff, dark: dark) }
      mask(Self.host(of: bar.search.view, in: nav), top: bar.search.frame.minY)
    } else if let host = handoff.barHost as? UIView, host.superview === nav.view, handoff.search != .zero {
      // A pop starts before the list's bar is laid out again: its container is the one seen before
      mask(host, top: handoff.search.minY)
    }
    handoff.face = session
    handoff.placeholder = placeholder
    shown = toList ? 0 : 1
    target = shown
    handoff.progress = shown
    handoff.hidden = session.id
    let driver = probe.driver
    driver.layer.removeAllAnimations()
    driver.alpha = 0
    coordinator.animate(alongsideTransition: { _ in driver.alpha = 1 }, completion: { [self] context in
      finish(cancelled: context.isCancelled)
    })
    let link = CADisplayLink(target: self, selector: #selector(tick(_:)))
    link.add(to: .main, forMode: .common)
    self.link = link
  }

  @objc private func tick(_ link: CADisplayLink) {
    guard let handoff, !done else { return cancel() }
    if handoff.search == .zero { fallbackListBar(handoff) }
    // The system bar may only now be laid out (the first pop after opening on a chat): hidden as soon as it is there
    if masked == nil, let nav, let bar = Self.listBar(nav) {
      mask(Self.host(of: bar.search.view, in: nav), top: bar.search.frame.minY)
    }
    if placing {
      // The pushed chat now stands where it lands; a composer that came in as the card is not handed over
      guard let at = probe?.bar else { return cancel() }
      handoff.composer = at
      placing = false
      if handoff.search == .zero { fallbackListBar(handoff) }
    }
    if holdUntil == nil, !landing, let driver = probe?.driver {
      let moved = CGFloat(driver.layer.presentation()?.opacity ?? Float(driver.alpha))
      target = toList ? moved : 1 - moved
    }
    let now = link.targetTimestamp
    // A frame's step is capped at a 60 Hz frame's, so the bar eases on after a stall instead of leaping its length
    let k = last == 0 ? 0 : 1 - exp(-min(now - last, 1.0 / 60) / Self.follow)
    last = now
    shown += (target - shown) * k
    if abs(target - shown) < Self.settled { shown = target }
    handoff.progress = shown
    guard shown == target else { return }
    if landing { return cancel() }
    if let holdUntil, CACurrentMediaTime() >= holdUntil { endOnList() }
  }

  /// Close enough to the target to stand on it
  private static let settled: CGFloat = 0.002

  /// The transition ended: on the chat the composer comes back once the stand-in has landed on it; on the list the
  /// stand-in holds until the system bar has come in under it, then gives way to it
  private func finish(cancelled: Bool) {
    guard !done else { return }
    if toList != cancelled {
      target = 1
      holdUntil = CACurrentMediaTime() + Self.hold
    } else {
      target = 0
      landing = true
    }
  }

  /// How long the stand-in waits on the list for the system bar's own entrance to finish under it
  private static let hold: CFTimeInterval = 0.35

  private func endOnList() {
    // The list's bar is in: its place and contents for the next pop
    if let handoff, let nav, let bar = Self.listBar(nav) {
      handoff.search = bar.search.frame
      handoff.new = bar.new.frame
      Self.takeFaces(bar, into: handoff, dark: nav.traitCollection.userInterfaceStyle == .dark)
    }
    cancel()
  }

  /// Ends the hand-over now: the real bars come back and the stand-in goes, in the same frame
  func cancel() {
    guard !done else { return }
    done = true
    link?.invalidate()
    link = nil
    masked?.mask = maskedBefore
    masked = nil
    maskedBefore = nil
    guard let handoff, handoff.run === self else { return }
    handoff.progress = nil
    handoff.hidden = nil
    handoff.face = nil
    handoff.run = nil
  }

  /// Hides the bottom bar under the stand-in with a mask over the bars' container (`top`: the bar's top edge on the
  /// screen); the bar's own appearance and entrance stay the system's, under it
  private func mask(_ host: UIView?, top: CGFloat) {
    guard let host, masked == nil else { return }
    handoff?.barHost = host
    let edge = host.convert(CGPoint(x: 0, y: top - Metrics.s3), from: nil).y
    let keep = UIView(frame: CGRect(x: -host.bounds.width, y: -host.bounds.height, width: host.bounds.width * 3,
                                    height: host.bounds.height + edge))
    keep.backgroundColor = .black
    maskedBefore = host.mask
    host.mask = keep
    masked = host
  }

  /// The bars' container under the navigation controller's view: the bottom bar's glass is drawn there, not in the
  /// views around its items, and the navigation bar shares it, so only the band from the bottom bar's top down is
  /// hidden
  private static func host(of view: UIView, in nav: UINavigationController) -> UIView? {
    ancestors(view).first { $0.superview === nav.view }
  }

  private func fallbackListBar(_ handoff: BarHandoff) {
    // Never measured (the app opened on a chat): the system bar's layout on iOS 27, its items a step inside the
    // composer's edges with the bar's spacing between them
    let c = handoff.composer
    guard c != .zero else { return }
    let inset = Metrics.s3
    let side = c.height
    handoff.new = CGRect(x: c.maxX - inset - side, y: c.minY, width: side, height: side)
    handoff.search = CGRect(x: c.minX + inset, y: c.minY, width: handoff.new.minX - inset - c.minX - inset, height: side)
  }

  // MARK: The system bar

  typealias ListBar = (search: (view: UIView, frame: CGRect, field: UIView), new: (view: UIView, frame: CGRect))

  /// The list's search capsule and new-session circle: the field's first ancestor as tall as a bar control, and the
  /// leftmost square item of that height on its row past its end
  static func listBar(_ nav: UINavigationController) -> ListBar? {
    guard let window = nav.view.window,
          let field = descendants(of: nav.view).first(where: {
            $0 is UISearchTextField && $0.window != nil && frame($0).minY > window.bounds.midY
          })
    else { return nil }
    var capsule: UIView? = field
    while let v = capsule, v.bounds.height < Metrics.barCtl - Metrics.s2 { capsule = v.superview }
    guard let capsule, capsule !== nav.view, let row = capsule.superview?.superview else { return nil }
    let box = frame(capsule)
    let new = descendants(of: row)
      .filter {
        let f = frame($0)
        return abs(f.height - box.height) < 1 && abs(f.width - f.height) < 1 && abs(f.minY - box.minY) < 1 && f.minX >= box.maxX
      }
      .min { frame($0).minX < frame($1).minX }
    guard let new else { return nil }
    return ((capsule, box, field), (new, frame(new)))
  }

  /// What the stand-in fades in, as the system bar lays it out now: where the search field's glyph and hint sit in
  /// their capsule, and a picture of the new-session item's glyph, without the glass behind it
  static func takeFaces(_ bar: ListBar, into handoff: BarHandoff, dark: Bool) {
    let capsule = bar.search.frame
    let pieces = breadthFirst(bar.search.field)
    if let glyph = pieces.first(where: { $0 is UIImageView }) {
      handoff.searchGlyph = frame(glyph).offsetBy(dx: -capsule.minX, dy: -capsule.minY)
    }
    if let hint = pieces.first(where: { $0 is UILabel }) {
      handoff.searchHint = frame(hint).offsetBy(dx: -capsule.minX, dy: -capsule.minY)
    }
    // The glyph's own view: the first view inside the item narrower than the item
    let item = bar.new.view
    if let glyph = breadthFirst(item).first(where: { $0 !== item && $0.bounds.width > 0 && $0.bounds.width < item.bounds.width - Metrics.s1 }),
       let image = picture(glyph) {
      handoff.newFace = BarFace(image: image, frame: frame(glyph), dark: dark)
    }
  }

  /// The view's layers drawn into a picture, in its traits, so dynamic colours resolve as on the screen
  private static func picture(_ view: UIView) -> Image? {
    guard view.bounds.width > 0, view.bounds.height > 0 else { return nil }
    let format = UIGraphicsImageRendererFormat.preferred()
    format.opaque = false
    var image: UIImage?
    view.traitCollection.performAsCurrent {
      image = UIGraphicsImageRenderer(bounds: view.bounds, format: format).image { view.layer.render(in: $0.cgContext) }
    }
    return image.map { Image(uiImage: $0) }
  }

  private static func ancestors(_ view: UIView) -> [UIView] {
    var out: [UIView] = []
    var v = view.superview
    while let x = v {
      out.append(x)
      v = x.superview
    }
    return out
  }

  private static func frame(_ view: UIView) -> CGRect { view.convert(view.bounds, to: nil) }

  private static func descendants(of view: UIView) -> [UIView] {
    var out: [UIView] = []
    var stack = [view]
    while let next = stack.popLast() {
      out.append(next)
      stack.append(contentsOf: next.subviews)
    }
    return out
  }

  private static func breadthFirst(_ view: UIView) -> [UIView] {
    var out = [view]
    var i = 0
    while i < out.count {
      out.append(contentsOf: out[i].subviews)
      i += 1
    }
    return out
  }
}
#endif
