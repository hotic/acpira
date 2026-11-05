#if canImport(UIKit)
import AcpiraDesign
import Foundation
import os
import QuartzCore
import UIKit

/// Settings → Developer: a frame-rate and hitch overlay for judging smoothness on a real phone, release builds
/// included. It lives in its own small window above every sheet and the keyboard's host window, so it shows over any
/// page; a drag moves it and a tap clears its counters. Timing comes from one display link asking for the panel's
/// full rate. A frame counts as a hitch when it came more than half a frame late, and its hitch time is how late it
/// came, as Instruments and `FrameProbe` count it. Each hitch is also logged (`cc.hotic.acpira` / `hitch`) and
/// marked as a signpost, so Console and Instruments line it up with the rest of a trace.
/// The overlay's own cost stays off the frames it measures: its text and graph redraw a few times per second
@MainActor
public final class PerfHUD: NSObject {
  public static let shared = PerfHUD()

  private var showFPS = false
  private var showHitches = false
  private var window: HUDWindow?
  private var link: CADisplayLink?
  private var last: CFTimeInterval = 0
  private var lastPaint: CFTimeInterval = 0
  /// Recent frame intervals (seconds) and whether each was a hitch, oldest first, for the graph
  private var recent: [(interval: Double, hitch: Bool)] = []
  /// Frame times within the last second, for the rate
  private var stamps: [CFTimeInterval] = []
  /// Hitches since the last clear, and each one's time and lateness within the ratio's window
  private var hitchCount = 0
  private var hitchWindow: [(at: CFTimeInterval, late: Double)] = []
  private var lastHitch: (at: CFTimeInterval, late: Double)?
  private var expected: Double = 1.0 / 60

  private let log = Logger(subsystem: "cc.hotic.acpira", category: "hitch")
  private let signposter = OSSignposter(subsystem: "cc.hotic.acpira", category: .pointsOfInterest)

  /// Follows the two switches in UserDefaults (also settable as launch arguments, `-acpira.hud.fps YES`) and the
  /// app's activity. Call once at launch
  public func install() {
    let center = NotificationCenter.default
    // Defaults may change on any thread; the first scene may connect after launch, and the overlay needs one
    for name in [UserDefaults.didChangeNotification, UIScene.didActivateNotification] {
      center.addObserver(forName: name, object: nil, queue: .main) { _ in MainActor.assumeIsolated { PerfHUD.shared.sync() } }
    }
    // In the background the display link stops; the gap before the next frame is no hitch
    center.addObserver(forName: UIApplication.didBecomeActiveNotification, object: nil, queue: .main) { _ in
      MainActor.assumeIsolated { PerfHUD.shared.last = 0 }
    }
    sync()
  }

  private func sync() {
    let defaults = UserDefaults.standard
    let fps = defaults.bool(forKey: Prefs.hudFPS)
    let hitches = defaults.bool(forKey: Prefs.hudHitches)
    guard fps != showFPS || hitches != showHitches || ((fps || hitches) && window == nil) else { return }
    showFPS = fps
    showHitches = hitches
    if fps || hitches { start() } else { stop() }
  }

  private func start() {
    if window == nil {
      guard let scene = UIApplication.shared.connectedScenes.lazy.compactMap({ $0 as? UIWindowScene })
        .first(where: { $0.activationState == .foregroundActive }) ?? UIApplication.shared.connectedScenes.lazy
        .compactMap({ $0 as? UIWindowScene }).first else { return }
      let window = HUDWindow(windowScene: scene)
      window.onTap = { [weak self] in self?.clear() }
      self.window = window
    }
    window?.view.mode(fps: showFPS, hitches: showHitches)
    window?.place()
    window?.isHidden = false
    if link == nil {
      let link = CADisplayLink(target: self, selector: #selector(tick(_:)))
      // The panel's top rate: a 120 Hz screen is timed at 120 Hz (Info.plist's CADisableMinimumFrameDurationOnPhone
      // lifts the iPhone's 60 Hz cap on the app's own frames)
      link.preferredFrameRateRange = CAFrameRateRange(minimum: 60, maximum: 120, preferred: 120)
      link.add(to: .main, forMode: .common)
      self.link = link
    }
    clear()
  }

  private func stop() {
    link?.invalidate()
    link = nil
    window?.isHidden = true
    window = nil
  }

  private func clear() {
    recent.removeAll(keepingCapacity: true)
    stamps.removeAll(keepingCapacity: true)
    hitchWindow.removeAll(keepingCapacity: true)
    hitchCount = 0
    lastHitch = nil
    last = 0
    paint(now: CACurrentMediaTime())
  }

  @objc private func tick(_ link: CADisplayLink) {
    let now = link.timestamp
    expected = max(link.targetTimestamp - link.timestamp, 1.0 / 240)
    defer { last = now }
    stamps.append(now)
    while let first = stamps.first, now - first > 1 { stamps.removeFirst() }
    guard last > 0 else { return }
    let interval = now - last
    let hitch = interval > expected * 1.5
    recent.append((interval, hitch))
    if recent.count > HUDLayout.bars { recent.removeFirst(recent.count - HUDLayout.bars) }
    if hitch {
      let late = interval - expected
      hitchCount += 1
      hitchWindow.append((now, late))
      lastHitch = (now, late)
      let ms = Int((late * 1000).rounded())
      log.notice("hitch \(ms, privacy: .public) ms late")
      signposter.emitEvent("Hitch", "\(ms) ms late")
      window?.view.flash()
    }
    while let first = hitchWindow.first, now - first.at > HUDLayout.ratioWindow { hitchWindow.removeFirst() }
    if now - lastPaint >= HUDLayout.paintEvery || hitch { paint(now: now) }
  }

  private func paint(now: CFTimeInterval) {
    lastPaint = now
    guard let window else { return }
    let view = window.view
    window.placeIfResized()
    let span = stamps.count > 1 ? (stamps.last! - stamps.first!) : 0
    let fps = span > 0 ? Int((Double(stamps.count - 1) / span).rounded()) : 0
    let late = hitchWindow.reduce(0) { $0 + $1.late }
    // Hitch time ratio over the window, Apple's measure: under 5 ms/s reads smooth, over 10 ms/s plainly janky
    let ratio = late * 1000 / HUDLayout.ratioWindow
    let recentHitch = lastHitch.flatMap { now - $0.at < HUDLayout.lastHitchShown ? Int(($0.late * 1000).rounded()) : nil }
    view.update(fps: fps, target: Int((1 / expected).rounded()), frames: recent, expected: expected,
                hitches: hitchCount, ratio: ratio, lastHitchMs: recentHitch)
  }
}

/// The overlay's geometry and pacing: a debug instrument outside the app's design system, so it keeps its own values
private enum HUDLayout {
  static let bars = 90
  static let barWidth: CGFloat = 1.5
  static let graphHeight: CGFloat = 28
  /// Frame time at the graph's top; longer frames are clipped there
  static let graphCeiling = 0.05
  static let pad: CGFloat = 6
  static let gap: CGFloat = 3
  static let radius: CGFloat = 8
  static let font: CGFloat = 11
  static let margin: CGFloat = 8
  /// Room left for the navigation bar under the top safe area, so the panel never covers back or the bar's buttons
  static let barClearance: CGFloat = 52
  static let paintEvery = 0.2
  static let ratioWindow = 5.0
  static let lastHitchShown = 3.0
  static let flash = 0.6
}

/// The overlay's own window, sized to the panel: it takes touches only on itself and never becomes key
private final class HUDWindow: UIWindow {
  let view = HUDView()
  var onTap: (() -> Void)?
  /// Where the panel sits in the free room of the scene's safe area (0…1 on each axis), kept across rotation and
  /// resizing; it starts at the top end
  private var anchor = CGPoint(x: 1, y: 0)
  private var placedIn: CGRect = .zero
  private var dragFrom: (finger: CGPoint, origin: CGPoint)?

  override init(windowScene: UIWindowScene) {
    super.init(windowScene: windowScene)
    windowLevel = .alert + 1
    backgroundColor = .clear
    let host = UIViewController()
    host.view = view
    rootViewController = host
    view.addGestureRecognizer(UIPanGestureRecognizer(target: self, action: #selector(drag(_:))))
    view.addGestureRecognizer(UITapGestureRecognizer(target: self, action: #selector(tap)))
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) { fatalError() }

  override var canBecomeKey: Bool { false }

  /// Sizes the window to the panel and puts it at its anchor inside the scene's safe area
  func place() {
    guard let scene = windowScene else { return }
    let size = view.fittingSize()
    let bounds = scene.coordinateSpace.bounds
    placedIn = bounds
    let area = room(in: bounds)
    let x = area.minX + (area.width - size.width) * anchor.x
    let y = area.minY + (area.height - size.height) * anchor.y
    frame = CGRect(origin: CGPoint(x: x.rounded(), y: y.rounded()), size: size)
  }

  /// Where the panel may sit: the app window's safe area below the navigation bar, inset by a margin
  private func room(in bounds: CGRect) -> CGRect {
    var safe = windowScene?.windows.first(where: { $0 !== self })?.safeAreaInsets ?? .zero
    safe.top += HUDLayout.barClearance
    return bounds.inset(by: safe).insetBy(dx: HUDLayout.margin, dy: HUDLayout.margin)
  }

  /// After a rotation or a window resize, checked as the panel repaints
  func placeIfResized() {
    if let scene = windowScene, scene.coordinateSpace.bounds != placedIn { place() }
  }

  @objc private func drag(_ pan: UIPanGestureRecognizer) {
    guard let scene = windowScene else { return }
    // The finger in the scene's space: the window itself moves under it
    let finger = convert(pan.location(in: self), to: scene.coordinateSpace)
    switch pan.state {
    case .began: dragFrom = (finger, frame.origin)
    case .changed:
      if let from = dragFrom {
        frame.origin = CGPoint(x: from.origin.x + finger.x - from.finger.x, y: from.origin.y + finger.y - from.finger.y)
      }
    default: break
    }
    guard pan.state == .ended || pan.state == .cancelled else { return }
    dragFrom = nil
    let area = room(in: scene.coordinateSpace.bounds)
    let free = CGSize(width: max(area.width - frame.width, 1), height: max(area.height - frame.height, 1))
    anchor = CGPoint(x: min(max((frame.minX - area.minX) / free.width, 0), 1),
                     y: min(max((frame.minY - area.minY) / free.height, 0), 1))
    place()
  }

  @objc private func tap() { onTap?() }
}

/// The panel: the rate over the last second with a graph of recent frame times, and the hitch count with the hitch
/// time ratio; a hitch flashes its edge and shows how late it came for a few seconds
private final class HUDView: UIView {
  private let rate = UILabel()
  private let hitchLine = UILabel()
  private let graph = CAShapeLayer()
  private let late = CAShapeLayer()
  private let budget = CAShapeLayer()
  private var fps = true
  private var hitches = true
  private let mono = UIFont.monospacedDigitSystemFont(ofSize: HUDLayout.font, weight: .semibold)

  init() {
    super.init(frame: .zero)
    backgroundColor = UIColor.black.withAlphaComponent(0.72)
    layer.cornerRadius = HUDLayout.radius
    layer.cornerCurve = .continuous
    layer.borderWidth = 1
    layer.borderColor = UIColor.white.withAlphaComponent(0.12).cgColor
    for label in [rate, hitchLine] {
      label.font = mono
      label.textColor = .white
      addSubview(label)
    }
    graph.fillColor = UIColor.white.withAlphaComponent(0.55).cgColor
    late.fillColor = UIColor.systemRed.cgColor
    budget.strokeColor = UIColor.systemGreen.withAlphaComponent(0.7).cgColor
    budget.lineWidth = 1
    budget.fillColor = nil
    for shape in [graph, late, budget] {
      // Paths change a few times a second; no implicit animation between them
      shape.actions = ["path": NSNull()]
      layer.addSublayer(shape)
    }
    isAccessibilityElement = false
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) { fatalError() }

  func mode(fps: Bool, hitches: Bool) {
    self.fps = fps
    self.hitches = hitches
    rate.isHidden = !fps
    graph.isHidden = !fps
    late.isHidden = !fps
    budget.isHidden = !fps
    hitchLine.isHidden = !hitches
  }

  private var graphWidth: CGFloat { CGFloat(HUDLayout.bars) * HUDLayout.barWidth }
  /// Wide enough for the longest hitch line, so its numbers are never cut
  private lazy var contentWidth: CGFloat = {
    let sample = "\(t("mobile.dev.hud.hitches")) 999  999.9 ms/s  +999ms" as NSString
    return max(graphWidth, ceil(sample.size(withAttributes: [.font: mono]).width))
  }()
  private var lineHeight: CGFloat { ceil(mono.lineHeight) }

  func fittingSize() -> CGSize {
    var height = HUDLayout.pad * 2
    var parts: [CGFloat] = []
    if fps { parts += [lineHeight, HUDLayout.graphHeight] }
    if hitches { parts.append(lineHeight) }
    height += parts.reduce(0, +) + HUDLayout.gap * CGFloat(max(parts.count - 1, 0))
    return CGSize(width: contentWidth + HUDLayout.pad * 2, height: height)
  }

  override func layoutSubviews() {
    super.layoutSubviews()
    var y = HUDLayout.pad
    let width = bounds.width - HUDLayout.pad * 2
    if fps {
      rate.frame = CGRect(x: HUDLayout.pad, y: y, width: width, height: lineHeight)
      y += lineHeight + HUDLayout.gap
      let box = CGRect(x: HUDLayout.pad, y: y, width: contentWidth, height: HUDLayout.graphHeight)
      for shape in [graph, late, budget] { shape.frame = box }
      y += HUDLayout.graphHeight + HUDLayout.gap
    }
    if hitches { hitchLine.frame = CGRect(x: HUDLayout.pad, y: y, width: width, height: lineHeight) }
  }

  func update(fps: Int, target: Int, frames: [(interval: Double, hitch: Bool)], expected: Double,
              hitches: Int, ratio: Double, lastHitchMs: Int?) {
    if self.fps {
      rate.text = "\(fps) / \(target) FPS"
      rate.textColor = fps >= target * 9 / 10 ? .white : (fps >= target * 2 / 3 ? .systemYellow : .systemRed)
      let normal = UIBezierPath()
      let red = UIBezierPath()
      let h = HUDLayout.graphHeight
      // Right-aligned, newest at the right edge
      let barWidth = contentWidth / CGFloat(HUDLayout.bars)
      let start = CGFloat(HUDLayout.bars - frames.count) * barWidth
      for (i, frame) in frames.enumerated() {
        let bar = min(CGFloat(frame.interval / HUDLayout.graphCeiling), 1) * h
        let rect = CGRect(x: start + CGFloat(i) * barWidth, y: h - bar, width: barWidth, height: bar)
        (frame.hitch ? red : normal).append(UIBezierPath(rect: rect))
      }
      graph.path = normal.cgPath
      late.path = red.cgPath
      // The frame budget: bars above this line came late
      let line = h - min(CGFloat(expected / HUDLayout.graphCeiling), 1) * h
      let path = UIBezierPath()
      path.move(to: CGPoint(x: 0, y: line))
      path.addLine(to: CGPoint(x: contentWidth, y: line))
      budget.path = path.cgPath
    }
    if self.hitches {
      let ratioText = String(format: "%.1f ms/s", ratio)
      var text = "\(t("mobile.dev.hud.hitches")) \(hitches)  \(ratioText)"
      if let lastHitchMs { text += "  +\(lastHitchMs)ms" }
      hitchLine.text = text
      hitchLine.textColor = ratio < 5 ? .white : (ratio < 10 ? .systemYellow : .systemRed)
    }
  }

  /// A hitch's mark on the edge, fading back
  func flash() {
    layer.removeAnimation(forKey: "flash")
    let anim = CABasicAnimation(keyPath: "borderColor")
    anim.fromValue = UIColor.systemRed.cgColor
    anim.toValue = UIColor.white.withAlphaComponent(0.12).cgColor
    anim.duration = HUDLayout.flash
    layer.add(anim, forKey: "flash")
  }
}
#endif
