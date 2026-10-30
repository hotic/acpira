import AcpiraModels
import Foundation

// chat/turnActionHelpers.ts and TurnActions.tsx StatsCard: what a reply's statistics card lists, only the rows the peer
// reported: the timing tiles, the token lines, the context it left.

/// modelLabel: the model that answered. The peer's own id when it reported one, else the selection captured on the
/// prompt, named through the model control's options as its family (effort, Thinking, Fast and 1M left out; a Fusion
/// pair keeps its whole name); an unknown id shows as is
func modelLabel(_ usage: TurnUsage?, settings: TurnSettings?, controls: SessionControls) -> String? {
  let control = controls.options.first { $0.category == "model" }
  guard let id = usage?.model ?? control.flatMap({ settings?.config[$0.id] }) else { return nil }
  guard let name = control?.options.first(where: { $0.id == id })?.name else { return id }
  let parsed = ModelCatalog.parse(name)
  return parsed.lead == nil ? parsed.family : name
}

/// turnTtft: from the prompt going out to the first streamed block. Imported history has neither stamp, and a turn
/// that failed before streaming has no `respondedAt`
func turnTtft(_ turn: AgentTurn) -> String? {
  guard let started = turn.startedAt, let responded = turn.respondedAt, responded >= started else { return nil }
  return commandDuration(ms: responded - started)
}

/// The turn's generated tokens: `output` already holds `reasoning` on OpenAI-style peers (input + output = total), while
/// peers that count thoughts apart report reasoning above output, or input + output + reasoning = total
private func generatedTokens(_ u: TurnUsage?) -> Int? {
  guard let u, let output = u.output else { return nil }
  let r = u.reasoning ?? 0
  if r <= 0 { return output }
  if r > output { return output + r }
  if let total = u.total, let input = u.input, abs(input + output + r - total) < abs(input + output - total) { return output + r }
  return output
}

/// The foreground tool runs inside `[from, to]`, as spans; a background call runs beside the model, not instead of it
private func foregroundSpans(_ turn: AgentTurn, from: Double, to: Double) -> [(Double, Double)] {
  turn.blocks.compactMap { block -> (Double, Double)? in
    guard case .toolCall(let b) = block, b.background != true, let s = b.startedAt, let e = b.endedAt else { return nil }
    let start = max(from, s), end = min(to, e)
    return end > start ? (start, end) : nil
  }
}

/// Foreground tool time inside the window, overlapping parallel calls merged so they count once
private func foregroundToolMs(_ turn: AgentTurn, from: Double, to: Double) -> Double {
  let spans = foregroundSpans(turn, from: from, to: to).sorted { $0.0 < $1.0 }
  guard var current = spans.first else { return 0 }
  var total = 0.0
  for span in spans.dropFirst() {
    if span.0 <= current.1 { current.1 = max(current.1, span.1) } else { total += current.1 - current.0; current = span }
  }
  return total + current.1 - current.0
}

/// tallyIsLastCall: the tally covers only the turn's last model call (Devin reports the last request's own usage: its
/// input + output matches the end-of-turn context), unless the peer said how many calls it counted
private func tallyIsLastCall(_ u: TurnUsage) -> Bool {
  if let calls = u.modelCalls { return calls == 1 }
  guard let context = u.context, context.used > 0, let input = u.input, let output = u.output else { return false }
  let total = u.total ?? input + output
  return abs(Double(total - context.used)) <= Double(context.used) * 0.02
}

/// turnOutputTps: decode throughput, `(tokens - 1) / seconds` from the first streamed block to the end, foreground tool
/// time taken out; a tally of the last call alone is timed from the last foreground tool's end. Left out below 5
/// tokens or a 200 ms window (a buffered dump)
func turnOutputTps(_ turn: AgentTurn) -> Int? {
  guard let responded = turn.respondedAt, let ended = turn.endedAt, let usage = turn.usage,
        let tokens = generatedTokens(usage), tokens >= 5 else { return nil }
  let lastToolEnd = tallyIsLastCall(usage) ? foregroundSpans(turn, from: responded, to: ended).map(\.1).max() : nil
  let active = lastToolEnd.map { ended - $0 } ?? ended - responded - foregroundToolMs(turn, from: responded, to: ended)
  guard active >= 200 else { return nil }
  let tps = Int((Double(tokens - 1) * 1000 / active).rounded())
  return tps > 0 ? tps : nil
}

/// One timing tile (duration, first token, output speed) or one token line; `chip` is the share riding beside the value
struct StatLine: Equatable {
  var label: String
  var value: String
  var chip: String? = nil
}

/// The context the turn left: its share as the chip, then used over size
struct StatContext: Equatable {
  var share: String
  var used: String
  var size: String
}

/// What StatsCard shows, only what the peer reported: the timing tiles, the token lines with the cache share as a chip on
/// the input line, then the post-turn context
struct TurnStatsContent: Equatable {
  var tiles: [StatLine] = []
  var tokens: [StatLine] = []
  var context: StatContext?
}

func turnStatsContent(_ turn: AgentTurn) -> TurnStatsContent {
  let usage = turn.usage
  let locale = Locale(identifier: L10n.locale)
  func number(_ n: Int) -> String { n.formatted(.number.locale(locale)) }
  // Floored, so a 99.6% hit never reads as a full 100%
  func share(_ part: Int, _ whole: Int) -> String {
    (floor(Double(part) / Double(whole) * 100) / 100).formatted(.percent.precision(.fractionLength(0)).locale(locale))
  }
  var out = TurnStatsContent()
  if turn.startedAt != nil && turn.endedAt != nil { out.tiles.append(StatLine(label: t("stats.duration"), value: elapsedDuration(turn))) }
  if let ttft = turnTtft(turn) { out.tiles.append(StatLine(label: t("stats.firstToken"), value: ttft)) }
  if let tps = turnOutputTps(turn) { out.tiles.append(StatLine(label: t("stats.outputShort"), value: "\(number(tps)) tok/s")) }
  // A stopped turn whose counts are all zero reported nothing (claude-agent-acp answers a cancel with zeros)
  let counts = [usage?.input, usage?.output, usage?.cachedRead, usage?.cachedWrite, usage?.reasoning]
  let unreported = turn.stop == .cancelled && !counts.contains { ($0 ?? 0) > 0 }
  if !unreported {
    // input / output whenever reported, 0 included; cache write / reasoning only above zero (Grok reports a 0 write
    // on every turn); the cache read is the input line's chip, not a line of its own
    let fields: [(Int?, String, Bool)] = [
      (usage?.input, "stats.input", true), (usage?.cachedWrite, "stats.cachedWrite", false),
      (usage?.output, "stats.output", true), (usage?.reasoning, "stats.reasoning", false),
    ]
    out.tokens = fields.compactMap { value, key, always in
      guard let value, always || value > 0 else { return nil }
      var line = StatLine(label: t(key), value: number(value))
      if key == "stats.input", let read = usage?.cachedRead, read > 0, value > 0 {
        line.chip = t("stats.cacheHit", ["pct": share(read, value)])
      }
      return line
    }
  }
  if let c = usage?.context, c.size > 0 { out.context = StatContext(share: share(c.used, c.size), used: number(c.used), size: number(c.size)) }
  return out
}
