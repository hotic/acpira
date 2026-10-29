import AcpiraDesign
import SwiftUI

// The webview's ticking times: a running turn's counter on its head (TurnClock.tsx LiveElapsed) and an unfinished
// command's seconds at its row's end (useToolSeconds.ts). Both read the app's clock (`clockOffset`) and are not motion:
// they keep ticking with motion off.

/// Content redrawn on every whole second after `since` (ms since the epoch), handed the ms elapsed on the app's clock
struct LiveTime<Content: View>: View {
  let since: Double
  @ViewBuilder let content: (Double) -> Content
  @Environment(\.clockOffset) private var offset

  var body: some View {
    // The real time at which the app's clock reads `since`
    TimelineView(.periodic(from: Date(timeIntervalSince1970: since / 1000 - offset), by: 1)) { _ in
      // The schedule only lands the redraws on whole seconds; the value is read off the clock itself
      content((Date().timeIntervalSince1970 + offset) * 1000 - since)
    }
  }
}

/// LiveElapsed: the bare duration like the finished head, nothing in the first second. It rides with the head's label
/// as one part, so it brings its own gap-2 only once it shows
struct LiveElapsed: View {
  /// `clockStart`
  let since: Double
  @Environment(\.clockOffset) private var offset
  /// Syncing, the counter stands still where it was: the turn may have ended since
  @Environment(\.syncing) private var syncing

  var body: some View {
    if syncing {
      label((Date().timeIntervalSince1970 + offset) * 1000 - since)
    } else {
      LiveTime(since: since) { label($0) }
    }
  }

  @ViewBuilder
  private func label(_ ms: Double) -> some View {
    if ms >= 1000 {
      // Tabular digits keep the width steady between ticks
      Text(web: durationText(ms: ms))
        .foregroundStyle(Palette.fg3)
        .monospacedDigit()
        .lineLimit(1)
        .fixedSize()
        .padding(.leading, Metrics.s2)
    }
  }
}

/// useToolSeconds: an unfinished command's whole seconds since it started ("0s" at once), in the row's trailing meta
struct CommandSeconds: View {
  /// The call's `startedAt`
  let since: Double

  var body: some View {
    LiveTime(since: since) { ms in Text(web: commandDuration(ms: ms, whole: true)) }
  }
}
