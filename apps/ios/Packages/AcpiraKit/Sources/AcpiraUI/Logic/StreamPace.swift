import Foundation

// chat/streamMotion.ts useSmoothText: the visible prefix of a streamed text. The visible text advances at the
// agent's measured throughput (characters per second, a time-weighted EMA that counts stalls too), so a burst after
// a pause plays at the speed the stream has been running at instead of all at once. Lag beyond `lag` raises the
// target speed proportionally (never above `maxLag` of backlog), the actual speed eases toward the target over
// `ease`, and `finish` flushes the rest once the stream ends. Counts are graphemes (Swift Characters), so a cut never
// splits a surrogate pair, a ZWJ emoji or a combining mark. Times are seconds.

/// STREAM_PACE (chosen in lab/stream-pace.preview.html)
struct StreamPace: Equatable, Sendable {
  var tick: Double = 0.032
  var lag: Double = 0.5
  var maxLag: Double = 1.5
  var ease: Double = 0.18
  var window: Double = 0.8
  var finish: Double = 0.22

  static let standard = StreamPace()
}

/// One step of the reveal: at `time` the visible prefix grew from `from` to `to` characters
struct StreamReveal: Equatable, Sendable {
  var time: Double
  var from: Int
  var to: Int
}

struct StreamPacer: Equatable {
  var pace = StreamPace.standard
  /// The received text, as graphemes
  private(set) var target: [Character] = []
  /// Characters on screen
  private(set) var count = 0
  private(set) var streaming = false
  /// Recent reveal steps, newest last, kept for one glyph fade (STREAM_DURATION_MS)
  private(set) var reveals: [StreamReveal] = []
  private var pos: Double = 0
  private var at: Double = 0
  /// Characters per second of the incoming stream, and actually revealed
  private var throughput: Double = 0
  private var speed: Double = 0

  /// Starts empty for a text that mounts while streaming, complete otherwise
  init(text: String = "", streaming: Bool = false) {
    target = Array(text)
    self.streaming = streaming
    count = streaming ? 0 : target.count
    pos = Double(count)
  }

  /// The visible text still trails the received one
  var pending: Bool { count < target.count }
  var visible: String { String(target[0..<min(count, target.count)]) }

  /// A new value of the streamed text. Text that does not extend the previous value (an edit, a replay) shows at once
  mutating func receive(_ text: String, streaming: Bool, now: Double) {
    let next = Array(text)
    let appended = next.starts(with: target)
    let added = next.count - target.count
    target = next
    self.streaming = streaming
    guard appended else {
      count = next.count; pos = Double(count); speed = 0; reveals = []
      return
    }
    guard added > 0 else { return }
    if at == 0 {
      // First chunk: no rate yet, assume it covers one lag budget
      throughput = Double(added) / pace.lag
      speed = throughput
    } else {
      // Time-weighted EMA: a stall lowers the measured rate instead of making the next burst look fast
      let gap = min(max(now - at, 0.016), pace.maxLag)
      throughput += (Double(added) / gap - throughput) * (1 - exp(-gap / pace.window))
    }
    at = now
  }

  /// One pacing tick, `dt` seconds after the last
  mutating func tick(now: Double, dt: Double, fade: Double = 0.18) {
    reveals.removeAll { now - $0.time > fade }
    guard pending else { return }
    let backlog = Double(target.count) - pos
    // Proportional catch-up past the lag budget, a hard floor at maxLag, and a quick flush once the stream ends
    let lag = throughput > 0 ? backlog / throughput : pace.lag
    var want = max(throughput * (1 + max(0, lag - pace.lag) / pace.lag), backlog / pace.maxLag)
    if !streaming { want = max(want, backlog / pace.finish) }
    speed += (want - speed) * (1 - exp(-dt / pace.ease))
    pos = min(Double(target.count), pos + speed * dt)
    let next = StreamCut.cut(target, at: Int(pos), streaming: streaming)
    if next > count {
      reveals.append(StreamReveal(time: now, from: count, to: next))
      count = next
    }
  }

  /// Drops reveal steps older than one fade without moving the text
  mutating func expire(now: Double, fade: Double = 0.18) {
    reveals.removeAll { now - $0.time > fade }
  }
}

/// Cut points: markdown punctuation and whitespace a visible prefix must not end on. A transient literal ("- *",
/// "**", "1") would otherwise render as text for a tick and then turn into formatting
enum StreamCut {
  private static let syntax: Set<Character> = ["*", "_", "`", "~", "#", ">", "|", "+", "=", "-", "[", "]", "(", ")", "!", "\\"]
  private static let reach = 64

  static func endsOnSyntax(_ text: [Character], _ cut: Int) -> Bool {
    guard cut > 0 else { return false }
    let last = text[cut - 1]
    if last.isWhitespace || syntax.contains(last) { return true }
    // An ordered-list marker: digits (and a `.` / `)`) after only spaces since a line start. A window that does not
    // reach a line start cannot prove one
    let lo = max(0, cut - 24)
    var i = cut - 1
    if last == "." || last == ")" { i -= 1 }
    let digitsEnd = i
    while i >= lo, text[i].isASCII, text[i].isNumber { i -= 1 }
    guard i < digitsEnd else { return false }
    while i >= lo, text[i] == " " || text[i] == "\t" { i -= 1 }
    if i < lo { return lo == 0 }
    return text[i] == "\n"
  }

  /// Cut point at or after `n` that lands after a content character. While streaming, received text that itself
  /// ends on syntax is held back to its last content character until more arrives
  static func cut(_ text: [Character], at n: Int, streaming: Bool) -> Int {
    var cut = min(max(n, 0), text.count)
    let limit = min(text.count, cut + reach)
    while cut > 0 && cut < limit && endsOnSyntax(text, cut) { cut += 1 }
    if streaming && cut == text.count { while cut > 0 && endsOnSyntax(text, cut) { cut -= 1 } }
    return cut
  }
}
