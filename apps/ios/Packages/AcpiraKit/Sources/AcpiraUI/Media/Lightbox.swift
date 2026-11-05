import AcpiraDesign
import AcpiraModels
import SwiftUI
#if canImport(UIKit)
import UIKit
#endif

// chat/Lightbox.tsx on the phone: the picture over black, full screen. Pinch or a double tap zooms it, a zoomed picture
// pans, and a swipe down at rest closes it, as ✕ does; share hands it to the system share sheet (Save Image, Copy,
// AirDrop). A tap hides the bars. A sideways swipe turns to the session's previous or next image, as Photos does (the
// desktop shows one image at a time). The desktop's wheel zoom, backdrop click and right-click menu have no phone form.

/// The Lightbox's arithmetic, apart from UIKit so it is tested on the Mac
nonisolated enum LightboxGeometry {
  /// Lightbox.tsx MAX_SCALE: the least a picture zooms to; a larger one goes on to its own pixels
  static let minMaxZoom: CGFloat = 4
  /// How far a double tap zooms in (a second one zooms out)
  static let doubleTapZoom: CGFloat = 2.5
  /// How much the picture shrinks at a full pull
  static let pullShrink: CGFloat = 0.25
  /// A pull past this share of the screen's height closes on release, as does a downward flick faster than
  /// `closeSpeed` (points per second)
  static let closeShare: CGFloat = 0.15
  static let closeSpeed: CGFloat = 800
  /// How near an edge a zoomed picture counts as at it, so a sideways drag turns the page (points)
  static let edgeSlop: CGFloat = 0.5

  /// The picture fitted into the screen, aspect kept (enlarged when smaller, as Photos shows it). Clamped to the
  /// screen: a rounding error past its edge made the unzoomed picture scroll sideways and take the page swipe
  static func fit(_ size: CGSize, in bounds: CGSize) -> CGSize {
    guard size.width > 0, size.height > 0 else { return .zero }
    let scale = min(bounds.width / size.width, bounds.height / size.height)
    return CGSize(width: min(bounds.width, size.width * scale), height: min(bounds.height, size.height * scale))
  }

  /// The insets that centre content smaller than the screen (none along an axis it overflows)
  static func centering(_ content: CGSize, in bounds: CGSize) -> CGSize {
    CGSize(width: max(0, (bounds.width - content.width) / 2), height: max(0, (bounds.height - content.height) / 2))
  }

  /// The deepest zoom: `minMaxZoom`, or further while a screen pixel still meets a picture pixel
  static func maxZoom(pixels: CGSize, fitted: CGSize, screenScale: CGFloat) -> CGFloat {
    guard fitted.width > 0, screenScale > 0 else { return minMaxZoom }
    return max(minMaxZoom, pixels.width / (fitted.width * screenScale))
  }

  /// The area of the fitted picture a zoom to `scale` around `point` shows (UIScrollView.zoom(to:))
  static func zoomRect(around point: CGPoint, scale: CGFloat, in bounds: CGSize) -> CGRect {
    let size = CGSize(width: bounds.width / scale, height: bounds.height / scale)
    return CGRect(x: point.x - size.width / 2, y: point.y - size.height / 2, width: size.width, height: size.height)
  }

  /// How far a downward pull has gone, 0 at rest to 1 at half the screen's height
  static func pull(_ dy: CGFloat, height: CGFloat) -> CGFloat {
    guard height > 0 else { return 0 }
    return min(1, max(0, dy / (height / 2)))
  }

  /// Whether releasing a pull closes the Lightbox
  static func closes(dy: CGFloat, velocity: CGFloat, height: CGFloat) -> Bool {
    dy > height * closeShare || (dy > 0 && velocity > closeSpeed)
  }

  /// Continue along the release direction rather than steering a diagonal pull straight down
  static func departure(from move: CGPoint, velocity: CGPoint, height: CGFloat) -> CGPoint {
    let direction = velocity.y > 0 ? velocity : move
    guard direction.y > 0 else { return move }
    let remaining = max(0, height - move.y)
    return CGPoint(x: move.x + remaining * direction.x / direction.y, y: move.y + remaining)
  }
}

/// One page of the Lightbox: its pixels when already at hand (the tapped picture), else where to load them
struct LightboxPage: Equatable {
  var source: MediaSource?
  var name: String?
  var image: DecodedImage?
}

extension LightboxItem {
  /// The pages the Lightbox swipes through: the session's images in reading order, opened at this one, which keeps the
  /// pixels and name its view gave it (a file row names its path). One not among them (no source or session, a photo
  /// not yet sent) stands alone
  func gallery(in view: SessionView?) -> (pages: [LightboxPage], start: Int) {
    let tapped = LightboxPage(source: source, name: name, image: image)
    guard let source, let view else { return ([tapped], 0) }
    var pages = MediaLoader.entries(in: view).map { LightboxPage(source: $0.source, name: $0.name) }
    guard let start = pages.firstIndex(where: { $0.source == source }) else { return ([tapped], 0) }
    pages[start] = tapped
    return (pages, start)
  }
}

struct Lightbox: View {
  let item: LightboxItem
  let loader: MediaLoader
  /// The chat's session as it is now, read once when the Lightbox appears
  var session: () -> SessionView?
  var onClose: () -> Void
  @Environment(\.motionEnabled) private var motion
  /// Faded in once on screen, out before the cover goes
  @State private var shown = false
  /// System bars must not overlap the chat's bars through the transparent cover during its fade.
  @State private var chrome = false
  @State private var closing = false
  /// The swipe-down's progress, 0 at rest
  @State private var pull: CGFloat = 0
  @State private var bars = true
  /// The tapped picture alone until the session's other images join it on appear
  @State private var pages: [LightboxPage]
  @State private var index = 0
  /// The pixels of the page in view, nil while they load (no share button then)
  @State private var current: DecodedImage?

  init(item: LightboxItem, loader: MediaLoader, session: @escaping () -> SessionView?, onClose: @escaping () -> Void) {
    self.item = item
    self.loader = loader
    self.session = session
    self.onClose = onClose
    _pages = State(initialValue: [LightboxPage(source: item.source, name: item.name, image: item.image)])
    _current = State(initialValue: item.image)
  }

  var body: some View {
    let name = pages[min(index, pages.count - 1)].name
    NavigationStack {
      pager
        .ignoresSafeArea()
        .opacity(shown ? 1 : 0)
        .background { Color.black.opacity(shown ? 1 - pull : 0).ignoresSafeArea() }
        .navigationTitle(Text(web: name ?? ""))
        .navigationBarTitleDisplayModeInline()
        .toolbar {
          ToolbarItem(placement: .cancellationAction) {
            CloseButton(action: close)
          }
          if let current {
            ToolbarItem(placement: .primaryAction) {
              let image = Image(decorative: current.cgImage, scale: 1)
              ShareLink(item: image, preview: SharePreview(Text(web: name ?? t("common.image")), image: image)) {
                Label { Text(web: t("mobile.share")) } icon: { Image(systemName: "square.and.arrow.up") }
              }
            }
          }
        }
        #if os(iOS)
        .toolbarVisibility(chrome && bars && pull == 0 ? .visible : .hidden, for: .navigationBar)
        .containerBackground(.clear, for: .navigation)
        #endif
    }
    .environment(\.colorScheme, .dark)
    .presentationBackground(.clear)
    .allowsHitTesting(!closing)
    .onAppear {
      let gallery = item.gallery(in: session())
      pages = gallery.pages
      index = gallery.start
      // Wait until black is opaque before exposing the preview's glass controls.
      withAnimation(motion ? Motion.open : nil) { shown = true } completion: {
        if !closing { chrome = true }
      }
    }
  }

  @ViewBuilder private var pager: some View {
    #if canImport(UIKit)
    LightboxPager(pages: pages, index: index, loader: loader, onPage: { index = $0; current = $1 },
                  onPull: { pull = $0 }, onClose: close, onTap: toggleBars)
    #else
    ZoomableImage(image: item.image, onPull: { pull = $0 }, onClose: close, onTap: toggleBars)
    #endif
  }

  private func toggleBars() {
    withAnimation(motion ? Motion.quick : nil) { bars.toggle() }
  }

  private func close() {
    guard !closing else { return }
    var quiet = Transaction()
    quiet.disablesAnimations = true
    // Hide the system bar before the background reveals the chat; the image alone fades away.
    withTransaction(quiet) {
      closing = true
      chrome = false
    }
    guard motion else { onClose(); return }
    withAnimation(Motion.close) { shown = false } completion: { onClose() }
  }
}

#if canImport(UIKit)
/// The pages side by side in a paging scroll view, `Metrics.page` apart as Photos keeps them. The page in view and its
/// neighbours are built, each a ZoomView, or a spinner while its pixels load. A sideways drag turns the page (a zoomed
/// picture pans to its edge first, `ZoomScrollView`); an upright drag is the page's own, its pull or a zoomed pan
struct LightboxPager: UIViewRepresentable {
  let pages: [LightboxPage]
  let index: Int
  let loader: MediaLoader
  /// The page now in view and its pixels, nil while they load
  var onPage: (Int, DecodedImage?) -> Void
  var onPull: (CGFloat) -> Void
  var onClose: () -> Void
  var onTap: () -> Void

  func makeUIView(context: Context) -> PagerView {
    let view = PagerView(loader: loader)
    updateUIView(view, context: context)
    return view
  }

  func updateUIView(_ view: PagerView, context: Context) {
    view.onPage = onPage
    view.onPull = onPull
    view.onClose = onClose
    view.onTap = onTap
    view.motion = context.environment.motionEnabled
    view.show(pages, at: index)
  }
}

final class PagerView: UIView, UIScrollViewDelegate {
  var onPage: (Int, DecodedImage?) -> Void = { _, _ in }
  var onPull: (CGFloat) -> Void = { _ in }
  var onClose: () -> Void = {}
  var onTap: () -> Void = {}
  var motion = true {
    didSet { for case let zoom as ZoomView in slots.values { zoom.motion = motion } }
  }
  private let loader: MediaLoader
  private let scroll = PagingScrollView()
  private(set) var pages: [LightboxPage] = []
  private(set) var index = 0
  /// The built pages by index: a ZoomView, or a PagePlaceholder
  private var slots: [Int: UIView] = [:]
  /// Pixels a load brought for a built page, kept in case the cache lets them go
  private var held: [Int: DecodedImage] = [:]
  private var loads: [Int: Task<Void, Never>] = [:]
  private var failed: Set<Int> = []
  /// The size the pages were last arranged for; a new one (rotation) arranges them again
  private var arranged: CGSize = .zero
  /// Set while layout moves the offset itself, which is no page turn
  private var arranging = false
  private static let gap = Metrics.page

  init(loader: MediaLoader) {
    self.loader = loader
    super.init(frame: .zero)
    // The scroll view is a gap wider than the screen, so a page and its gap turn as one
    clipsToBounds = true
    scroll.isPagingEnabled = true
    scroll.showsHorizontalScrollIndicator = false
    scroll.showsVerticalScrollIndicator = false
    scroll.contentInsetAdjustmentBehavior = .never
    // The navigation bar takes this scroll view for its content and laid its edge effect, a grey band, over the black
    if #available(iOS 26, *) { scroll.topEdgeEffect.isHidden = true }
    scroll.delegate = self
    addSubview(scroll)
  }

  required init?(coder: NSCoder) { nil }

  /// Shows these pages from `index`; the same pages again change nothing, so the page the reader turned to stays
  func show(_ pages: [LightboxPage], at index: Int) {
    guard pages != self.pages else { return }
    loads.values.forEach { $0.cancel() }
    slots.values.forEach { $0.removeFromSuperview() }
    (loads, slots, held, failed) = ([:], [:], [:], [])
    self.pages = pages
    self.index = max(0, min(index, pages.count - 1))
    arranged = .zero
    setNeedsLayout()
  }

  override func layoutSubviews() {
    super.layoutSubviews()
    let size = bounds.size
    guard size.width > 0, size != arranged else { return }
    arranged = size
    let stride = size.width + Self.gap
    arranging = true
    scroll.frame = CGRect(x: 0, y: 0, width: stride, height: size.height)
    scroll.contentSize = CGSize(width: stride * CGFloat(pages.count), height: size.height)
    scroll.contentOffset = CGPoint(x: stride * CGFloat(index), y: 0)
    arranging = false
    for (i, slot) in slots { slot.frame = frame(of: i) }
    fill()
  }

  func scrollViewDidScroll(_ scrollView: UIScrollView) {
    guard !arranging, bounds.width > 0, !pages.isEmpty else { return }
    let turned = Int((scroll.contentOffset.x / (bounds.width + Self.gap)).rounded())
    let i = max(0, min(pages.count - 1, turned))
    guard i != index else { return }
    index = i
    fill()
    report()
  }

  private func report() {
    onPage(index, (slots[index] as? ZoomView)?.decoded)
  }

  private func frame(of i: Int) -> CGRect {
    CGRect(x: (bounds.width + Self.gap) * CGFloat(i), y: 0, width: bounds.width, height: bounds.height)
  }

  /// Builds the page in view and its neighbours and drops the rest, so a page turned back to starts unzoomed, as in
  /// Photos
  private func fill() {
    guard !pages.isEmpty, bounds.width > 0 else { return }
    let keep = max(0, index - 1)...min(pages.count - 1, index + 1)
    for (i, slot) in slots where !keep.contains(i) {
      slot.removeFromSuperview()
      slots[i] = nil
      held[i] = nil
    }
    for i in keep where slots[i] == nil {
      let slot = makeSlot(i)
      slot.frame = frame(of: i)
      scroll.addSubview(slot)
      slots[i] = slot
    }
  }

  private func makeSlot(_ i: Int) -> UIView {
    let page = pages[i]
    if let image = page.image ?? held[i] ?? page.source.flatMap({ loader.peek($0) }) {
      let zoom = ZoomView(decoded: image)
      zoom.motion = motion
      zoom.onPull = { [weak self] in self?.onPull($0) }
      zoom.onClose = { [weak self] in self?.onClose() }
      zoom.onTap = { [weak self] in self?.onTap() }
      return zoom
    }
    let lost = page.source == nil || failed.contains(i)
    if !lost { load(i) }
    let placeholder = PagePlaceholder(failed: lost, name: page.name)
    placeholder.onTap = { [weak self] in self?.onTap() }
    return placeholder
  }

  private func load(_ i: Int) {
    guard loads[i] == nil, let source = pages[i].source else { return }
    loads[i] = Task { [weak self, loader] in
      let image = try? await loader.image(source)
      guard let self, !Task.isCancelled else { return }
      loads[i] = nil
      if let image { held[i] = image } else { failed.insert(i) }
      // The spinner gives way to the picture, or to the failure, if the page is still built
      guard let slot = slots[i] else { return }
      slot.removeFromSuperview()
      slots[i] = nil
      fill()
      if i == index { report() }
    }
  }
}

/// Turns pages on a sideways drag only, leaving an upright one to the page: its pull, or panning a zoomed picture
private final class PagingScrollView: UIScrollView {
  override func gestureRecognizerShouldBegin(_ recognizer: UIGestureRecognizer) -> Bool {
    if recognizer === panGestureRecognizer {
      let velocity = panGestureRecognizer.velocity(in: self)
      if abs(velocity.y) > abs(velocity.x) { return false }
    }
    return super.gestureRecognizerShouldBegin(recognizer)
  }
}

/// A page's zooming scroll view. Its pan begins even when the picture cannot move, and then holds the drag from the
/// pager: a sideways drag toward an edge the picture already shows (always, unzoomed) is left to the pager instead
private final class ZoomScrollView: UIScrollView {
  override func gestureRecognizerShouldBegin(_ recognizer: UIGestureRecognizer) -> Bool {
    guard recognizer === panGestureRecognizer else { return super.gestureRecognizerShouldBegin(recognizer) }
    let velocity = panGestureRecognizer.velocity(in: self)
    if abs(velocity.x) > abs(velocity.y) {
      let least = -adjustedContentInset.left
      let most = max(least, contentSize.width + adjustedContentInset.right - bounds.width)
      // A finger moving right shows more of the left side
      if velocity.x > 0 ? contentOffset.x <= least + LightboxGeometry.edgeSlop : contentOffset.x >= most - LightboxGeometry.edgeSlop {
        return false
      }
    }
    return super.gestureRecognizerShouldBegin(recognizer)
  }
}

/// A page whose pixels are on their way (a spinner) or could not come (the image glyph); a tap hides the bars, as on a
/// picture
private final class PagePlaceholder: UIView {
  var onTap: () -> Void = {}
  private let mark: UIView

  init(failed: Bool, name: String?) {
    if failed {
      let glyph = UIImageView(image: UIImage(systemName: "photo",
                                             withConfiguration: UIImage.SymbolConfiguration(pointSize: Metrics.iconCtl)))
      glyph.tintColor = UIColor(Palette.fg3)
      mark = glyph
    } else {
      let spinner = UIActivityIndicatorView(style: .medium)
      spinner.color = UIColor(Palette.fg3)
      spinner.startAnimating()
      mark = spinner
    }
    super.init(frame: .zero)
    addSubview(mark)
    addGestureRecognizer(UITapGestureRecognizer(target: self, action: #selector(tapped)))
    if failed {
      isAccessibilityElement = true
      accessibilityLabel = t("attach.loadFailed", ["name": name ?? t("common.image")])
    }
  }

  required init?(coder: NSCoder) { nil }

  override func layoutSubviews() {
    super.layoutSubviews()
    mark.sizeToFit()
    mark.center = CGPoint(x: bounds.midX, y: bounds.midY)
  }

  @objc private func tapped() { onTap() }
}

/// The picture in a zooming scroll view: pinch, double tap and panning are UIScrollView's own, and a one-finger
/// downward pan at rest pulls the picture away (`onPull` 0…1), closing past the threshold
final class ZoomView: UIView, UIScrollViewDelegate, UIGestureRecognizerDelegate {
  var onPull: (CGFloat) -> Void = { _ in }
  var onClose: () -> Void = {}
  var onTap: () -> Void = {}
  var motion = true
  /// A streamed animation's bytes, played while the view is in a window
  var stream: Data?
  /// The pixels shown, for the share button
  private(set) var decoded: DecodedImage?
  private let player = StreamPlayer()
  private let scroll = ZoomScrollView()
  private let picture: UIImageView
  private let pan = UIPanGestureRecognizer()
  /// The size the picture was last fitted to; a new one (rotation) fits it again
  private var fitted: CGSize = .zero

  init(image: UIImage) {
    picture = UIImageView(image: image)
    super.init(frame: .zero)
    scroll.delegate = self
    scroll.showsVerticalScrollIndicator = false
    scroll.showsHorizontalScrollIndicator = false
    scroll.contentInsetAdjustmentBehavior = .never
    scroll.decelerationRate = .fast
    scroll.addSubview(picture)
    addSubview(scroll)
    let double = UITapGestureRecognizer(target: self, action: #selector(doubleTap(_:)))
    double.numberOfTapsRequired = 2
    let single = UITapGestureRecognizer(target: self, action: #selector(singleTap))
    single.require(toFail: double)
    pan.addTarget(self, action: #selector(pulled(_:)))
    pan.maximumNumberOfTouches = 1
    // The scroll view pans a zoomed picture; at rest the pull takes a downward drag first
    scroll.panGestureRecognizer.require(toFail: pan)
    // UIKit asks the touched view (the scroll view), not this one, whether a recognizer attached here may begin: the
    // pull's direction check runs only as its delegate. Without it a sideways drag pulled the picture aside
    pan.delegate = self
    [double, single, pan].forEach(addGestureRecognizer)
  }

  convenience init(decoded image: DecodedImage) {
    // An animation plays in UIImageView itself; its frames are already on an even beat
    let picture = image.animated
      ? UIImage.animatedImage(with: image.frames.map { UIImage(cgImage: $0) }, duration: image.loopDuration)
      : nil
    self.init(image: picture ?? UIImage(cgImage: image.cgImage))
    decoded = image
    stream = image.stream
  }

  required init?(coder: NSCoder) { nil }

  override func didMoveToWindow() {
    super.didMoveToWindow()
    guard let stream, window != nil else { player.stop(); return }
    player.play(stream) { [weak self] in self?.picture.image = UIImage(cgImage: $0) }
  }

  override func layoutSubviews() {
    super.layoutSubviews()
    // A transformed view's frame is undefined; changing it during a pull shifts the picture toward a corner.
    scroll.bounds.size = bounds.size
    scroll.center = CGPoint(x: bounds.midX, y: bounds.midY)
    guard bounds.size != fitted, bounds.width > 0, let size = picture.image?.size else { return }
    fitted = bounds.size
    scroll.zoomScale = 1
    let fit = LightboxGeometry.fit(size, in: bounds.size)
    picture.frame = CGRect(origin: .zero, size: fit)
    scroll.contentSize = fit
    scroll.maximumZoomScale = LightboxGeometry.maxZoom(
      pixels: CGSize(width: size.width * (picture.image?.scale ?? 1), height: size.height * (picture.image?.scale ?? 1)),
      fitted: fit, screenScale: traitCollection.displayScale)
    center()
  }

  func viewForZooming(in scrollView: UIScrollView) -> UIView? { picture }
  func scrollViewDidZoom(_ scrollView: UIScrollView) { center() }

  private func center() {
    let inset = LightboxGeometry.centering(scroll.contentSize, in: bounds.size)
    scroll.contentInset = UIEdgeInsets(top: inset.height, left: inset.width, bottom: inset.height, right: inset.width)
  }

  /// The pull only starts at rest and downward, so a zoomed picture pans and a sideways drag turns the page
  override func gestureRecognizerShouldBegin(_ recognizer: UIGestureRecognizer) -> Bool {
    guard recognizer === pan else { return super.gestureRecognizerShouldBegin(recognizer) }
    let velocity = pan.velocity(in: self)
    return scroll.zoomScale <= scroll.minimumZoomScale && velocity.y > abs(velocity.x)
  }

  @objc private func singleTap() { onTap() }

  @objc private func doubleTap(_ recognizer: UITapGestureRecognizer) {
    if scroll.zoomScale > scroll.minimumZoomScale {
      scroll.setZoomScale(scroll.minimumZoomScale, animated: motion)
    } else {
      let scale = min(scroll.maximumZoomScale, LightboxGeometry.doubleTapZoom)
      scroll.zoom(to: LightboxGeometry.zoomRect(around: recognizer.location(in: picture), scale: scale, in: bounds.size),
                  animated: motion)
    }
  }

  @objc private func pulled(_ recognizer: UIPanGestureRecognizer) {
    let move = recognizer.translation(in: self)
    let progress = LightboxGeometry.pull(move.y, height: bounds.height)
    switch recognizer.state {
    case .began, .changed:
      let shrink = 1 - progress * LightboxGeometry.pullShrink
      scroll.transform = CGAffineTransform(translationX: move.x, y: move.y).scaledBy(x: shrink, y: shrink)
      onPull(progress)
    case .ended where LightboxGeometry.closes(dy: move.y, velocity: recognizer.velocity(in: self).y, height: bounds.height):
      // Keep the release direction while the Lightbox fades out.
      let target = LightboxGeometry.departure(from: move, velocity: recognizer.velocity(in: self), height: bounds.height)
      let away = CGAffineTransform(translationX: target.x, y: target.y).scaledBy(x: 1 - LightboxGeometry.pullShrink,
                                                                                y: 1 - LightboxGeometry.pullShrink)
      if motion { UIView.animate(withDuration: Motion.durClose) { self.scroll.transform = away } }
      onClose()
    case .ended, .cancelled, .failed:
      if motion {
        UIView.animate(springDuration: Motion.foldPhone, bounce: 0) { self.scroll.transform = .identity }
      } else {
        scroll.transform = .identity
      }
      onPull(0)
    default:
      break
    }
  }
}
#else
/// The Mac build (swift test) has no zooming scroll view: the picture fitted
struct ZoomableImage: View {
  let image: DecodedImage
  var onPull: (CGFloat) -> Void
  var onClose: () -> Void
  var onTap: () -> Void

  var body: some View {
    Picture(image: image).aspectRatio(contentMode: .fit).onTapGesture(perform: onTap)
  }
}
#endif
