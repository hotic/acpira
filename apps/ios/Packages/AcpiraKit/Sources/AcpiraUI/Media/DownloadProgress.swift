import AcpiraDesign
import SwiftUI

// A download's progress on a video card or the file page: a ring that fills as the bytes arrive (a spinner until the
// computer has said how large the file is), and the "3.2 MB / 15.9 MB" line under it.

/// The ring: an arc on a faint track, drawn the way the context ring is (ContextUsage.swift)
struct DownloadRing: View {
  /// 0...1; nil spins
  let fraction: Double?
  var side = Metrics.iconCtl
  @Environment(\.motionEnabled) private var motion

  var body: some View {
    if let fraction {
      let stroke = side / 8
      ZStack {
        Circle().stroke(lineWidth: stroke).opacity(0.25)
        Circle().trim(from: 0, to: fraction).stroke(style: StrokeStyle(lineWidth: stroke, lineCap: .round))
          .rotationEffect(.degrees(-90))
          .animation(motion ? Motion.ease : nil, value: fraction)
      }
      .padding(stroke / 2)
      .frame(width: side, height: side)
      .accessibilityElement()
      .accessibilityLabel(Text(web: t("mobile.download.progress", ["percent": Int(fraction * 100)])))
    } else {
      ProgressView().frame(width: side, height: side)
    }
  }
}

/// The bytes so far against the file's size, in the system's file units; nil until the size is known
func downloadLine(_ transfer: VideoCache.Transfer) -> String? {
  guard transfer.total > 0 else { return nil }
  let have = transfer.have.formatted(.byteCount(style: .file))
  let total = transfer.total.formatted(.byteCount(style: .file))
  return "\(have) / \(total)"
}
