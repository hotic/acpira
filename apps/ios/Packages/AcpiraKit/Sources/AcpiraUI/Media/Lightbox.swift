import AcpiraDesign
import SwiftUI
#if canImport(UIKit)
import UIKit
#endif

// chat/Lightbox.tsx on the phone: the picture over black, full screen. Pinch or a double tap zooms it, a zoomed picture
// pans, and a swipe down at rest closes it, as ✕ does; share hands it to the system share sheet (Save Image, Copy,
// AirDrop). A tap hides the bars. The desktop's wheel zoom, backdrop click and right-click menu have no phone form.

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

  /// The picture fitted into the screen, aspect kept (enlarged when smaller, as Photos shows it)
  static func fit(_ size: CGSize, in bounds: CGSize) -> CGSize {
    guard size.width > 0, size.height > 0 else { return .zero }
    let scale = min(bounds.width / size.width, bounds.height / size.height)
    return CGSize(width: size.width * scale, height: size.height * scale)
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
}

struct Lightbox: View {
  let item: LightboxItem
  var onClose: () -> Void
  @Environment(\.motionEnabled) private var motion
  /// Faded in once on screen, out before the cover goes
  @State private var shown = false
  /// The swipe-down's progress, 0 at rest
  @State private var pull: CGFloat = 0
  @State private var bars = true

  var body: some View {
    let image = Image(decorative: item.image.cgImage, scale: 1)
    NavigationStack {
      ZoomableImage(image: item.image, onPull: { pull = $0 }, onClose: close, onTap: toggleBars)
        .ignoresSafeArea()
        .opacity(shown ? 1 : 0)
        .background { Color.black.opacity(shown ? 1 - pull : 0).ignoresSafeArea() }
        .navigationTitle(Text(web: item.name ?? ""))
        .navigationBarTitleDisplayModeInline()
        .toolbar {
          ToolbarItem(placement: .cancellationAction) {
            Button(role: .close, action: close)
          }
          ToolbarItem(placement: .primaryAction) {
            ShareLink(item: image, preview: SharePreview(Text(web: item.name ?? t("common.image")), image: image)) {
              Label { Text(web: t("mobile.share")) } icon: { Image(systemName: "square.and.arrow.up") }
            }
          }
        }
        #if os(iOS)
        .toolbarVisibility(bars && pull == 0 ? .visible : .hidden, for: .navigationBar)
        .containerBackground(.clear, for: .navigation)
        #endif
    }
    .environment(\.colorScheme, .dark)
    .presentationBackground(.clear)
    .onAppear { withAnimation(motion ? Motion.open : nil) { shown = true } }
  }

  private func toggleBars() {
    withAnimation(motion ? Motion.quick : nil) { bars.toggle() }
  }

  private func close() {
    guard motion else { onClose(); return }
    withAnimation(Motion.close) { shown = false } completion: { onClose() }
  }
}

#if canImport(UIKit)
/// The picture in a zooming scroll view: pinch, double tap and panning are UIScrollView's own, and a one-finger
/// downward pan at rest pulls the picture away (`onPull` 0…1), closing past the threshold
struct ZoomableImage: UIViewRepresentable {
  let image: DecodedImage
  var onPull: (CGFloat) -> Void
  var onClose: () -> Void
  var onTap: () -> Void

  func makeUIView(context: Context) -> ZoomView {
    // An animation plays in UIImageView itself; its frames are already on an even beat
    let picture = image.animated
      ? UIImage.animatedImage(with: image.frames.map { UIImage(cgImage: $0) }, duration: image.loopDuration)
      : nil
    let view = ZoomView(image: picture ?? UIImage(cgImage: image.cgImage))
    updateUIView(view, context: context)
    return view
  }

  func updateUIView(_ view: ZoomView, context: Context) {
    view.onPull = onPull
    view.onClose = onClose
    view.onTap = onTap
    view.motion = context.environment.motionEnabled
  }
}

final class ZoomView: UIView, UIScrollViewDelegate {
  var onPull: (CGFloat) -> Void = { _ in }
  var onClose: () -> Void = {}
  var onTap: () -> Void = {}
  var motion = true
  private let scroll = UIScrollView()
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
    [double, single, pan].forEach(addGestureRecognizer)
  }

  required init?(coder: NSCoder) { nil }

  override func layoutSubviews() {
    super.layoutSubviews()
    scroll.frame = bounds
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

  /// The pull only starts at rest and downward, so a zoomed picture pans and a sideways drag does nothing
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
    case .changed:
      let shrink = 1 - progress * LightboxGeometry.pullShrink
      scroll.transform = CGAffineTransform(translationX: move.x, y: move.y).scaledBy(x: shrink, y: shrink)
      onPull(progress)
    case .ended where LightboxGeometry.closes(dy: move.y, velocity: recognizer.velocity(in: self).y, height: bounds.height):
      // The picture carries on down while the Lightbox fades out
      let away = CGAffineTransform(translationX: move.x, y: bounds.height).scaledBy(x: 1 - LightboxGeometry.pullShrink,
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
