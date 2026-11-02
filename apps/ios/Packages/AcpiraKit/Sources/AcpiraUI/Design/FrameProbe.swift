#if DEBUG && canImport(UIKit)
import Foundation
import QuartzCore
import UIKit

/// `-frameprobe`: frame timing for scripted performance runs on the simulator, where Instruments' hitch tracing is not
/// available. A display link times every main-thread frame between two Darwin notifications,
/// `cc.hotic.acpira.frames.start` and `cc.hotic.acpira.frames.stop` (`simctl spawn … notifyutil -p NAME`); the stop
/// writes the run's summary to `tmp/frames.json` in the app's data container and logs it on one line. A frame counts
/// as a hitch when it came more than half a frame late; its hitch time is how late it came, as Instruments counts it.
/// Debug builds only
@MainActor
public final class FrameProbe: NSObject {
  private static var shared: FrameProbe?
  private static let start = "cc.hotic.acpira.frames.start"
  private static let stop = "cc.hotic.acpira.frames.stop"

  private var link: CADisplayLink?
  private var began: CFTimeInterval = 0
  private var last: CFTimeInterval = 0
  /// Each frame's interval and the interval the display asked for, in seconds, with its time since the start
  private var frames: [(at: Double, interval: Double, expected: Double)] = []

  public static func installIfAsked(_ arguments: [String] = ProcessInfo.processInfo.arguments) {
    guard arguments.contains("-frameprobe"), shared == nil else { return }
    let probe = FrameProbe()
    shared = probe
    let center = CFNotificationCenterGetDarwinNotifyCenter()
    for name in [start, stop] {
      CFNotificationCenterAddObserver(center, Unmanaged.passUnretained(probe).toOpaque(), { _, _, name, _, _ in
        guard let name = name?.rawValue as String? else { return }
        DispatchQueue.main.async { MainActor.assumeIsolated { FrameProbe.shared?.handle(name) } }
      }, name as CFString, nil, .deliverImmediately)
    }
  }

  private func handle(_ name: String) {
    if name == Self.start { begin() } else if name == Self.stop { end() }
  }

  private func begin() {
    link?.invalidate()
    frames = []
    began = CACurrentMediaTime()
    last = 0
    let link = CADisplayLink(target: self, selector: #selector(tick(_:)))
    // As fast as the display goes, so a 120 Hz screen is timed at 120 Hz
    link.preferredFrameRateRange = CAFrameRateRange(minimum: 60, maximum: 120, preferred: 120)
    link.add(to: .main, forMode: .common)
    self.link = link
  }

  @objc private func tick(_ link: CADisplayLink) {
    let expected = link.targetTimestamp - link.timestamp
    if last > 0 { frames.append((link.timestamp - began, link.timestamp - last, expected)) }
    last = link.timestamp
  }

  private func end() {
    link?.invalidate()
    link = nil
    let duration = max(CACurrentMediaTime() - began, 0.001)
    let late = frames.filter { $0.interval > $0.expected * 1.5 }
    let hitchMs = late.reduce(0) { $0 + ($1.interval - $1.expected) } * 1000
    let sorted = frames.map(\.interval).sorted()
    func pct(_ p: Double) -> Double { sorted.isEmpty ? 0 : sorted[min(sorted.count - 1, Int(Double(sorted.count) * p))] * 1000 }
    let worst = frames.sorted { $0.interval > $1.interval }.prefix(8).map {
      ["atS": round3($0.at), "ms": round3($0.interval * 1000)]
    }
    let summary: [String: Any] = [
      "seconds": round3(duration),
      "frames": frames.count,
      "frameMs": round3((frames.first?.expected ?? 0) * 1000),
      "hitches": late.count,
      "hitchMs": round3(hitchMs),
      // Instruments' hitch time ratio: late time per second of the run
      "hitchMsPerS": round3(hitchMs / duration),
      "p50Ms": round3(pct(0.5)),
      "p95Ms": round3(pct(0.95)),
      "p99Ms": round3(pct(0.99)),
      "maxMs": round3((sorted.last ?? 0) * 1000),
      "worst": worst,
    ]
    guard let data = try? JSONSerialization.data(withJSONObject: summary, options: [.sortedKeys]) else { return }
    try? data.write(to: FileManager.default.temporaryDirectory.appendingPathComponent("frames.json"), options: .atomic)
    print("FRAMEPROBE \(String(decoding: data, as: UTF8.self))")
  }

  private func round3(_ x: Double) -> Double { (x * 1000).rounded() / 1000 }
}
#endif
