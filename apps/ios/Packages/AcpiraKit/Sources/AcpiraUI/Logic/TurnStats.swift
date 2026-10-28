import AcpiraModels
import Foundation

// chat/turnActionHelpers.ts and TurnActions.tsx StatsCard: what a reply's statistics card lists, only the rows the peer
// reported, in the card's three groups (the response, its token accounting, the context it left).

/// modelLabel: the model that answered. The peer's own id when it reported one, else the selection captured on the
/// prompt, named through the model control's options; an unknown id shows as is
func modelLabel(_ usage: TurnUsage?, settings: TurnSettings?, controls: SessionControls) -> String? {
  let control = controls.options.first { $0.category == "model" }
  guard let id = usage?.model ?? control.flatMap({ settings?.config[$0.id] }) else { return nil }
  return control?.options.first { $0.id == id }?.name ?? id
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

/// One line of the statistics card
struct StatLine: Equatable {
  var label: String
  var value: String
}

/// The card's groups, empty ones left out: the response (model, duration, time to first token, speed, tool and model
/// calls), the tokens (input and output whenever reported, the cache and reasoning counts only when above zero, since
/// Grok reports a cache write of 0 on every turn), then the context the turn left
func turnStatGroups(_ turn: AgentTurn, model: String?) -> [[StatLine]] {
  let usage = turn.usage
  let locale = Locale(identifier: L10n.locale)
  func number(_ n: Int) -> String { n.formatted(.number.locale(locale)) }
  var response: [StatLine] = []
  if let model { response.append(StatLine(label: t("stats.model"), value: model)) }
  if turn.startedAt != nil && turn.endedAt != nil { response.append(StatLine(label: t("stats.duration"), value: elapsedDuration(turn))) }
  if let ttft = turnTtft(turn) { response.append(StatLine(label: t("stats.ttft"), value: ttft)) }
  if let tps = turnOutputTps(turn) { response.append(StatLine(label: t("stats.outputSpeed"), value: "\(number(tps)) tok/s")) }
  let calls = turn.blocks.filter { $0.toolCall != nil }.count
  if calls > 0 { response.append(StatLine(label: t("stats.toolCalls"), value: number(calls))) }
  if let rounds = usage?.modelCalls, rounds > 0 { response.append(StatLine(label: t("stats.modelCalls"), value: number(rounds))) }
  let fields: [(Int?, String, Bool)] = [
    (usage?.input, "stats.input", true), (usage?.output, "stats.output", true), (usage?.cachedRead, "stats.cachedRead", false),
    (usage?.cachedWrite, "stats.cachedWrite", false), (usage?.reasoning, "stats.reasoning", false),
  ]
  let tokens = fields.compactMap { value, key, always -> StatLine? in
    guard let value, always || value > 0 else { return nil }
    return StatLine(label: t(key), value: number(value))
  }
  let context = usage?.context.map { [StatLine(label: t("stats.context"), value: "\(number($0.used)) / \(number($0.size))")] } ?? []
  return [response, tokens, context].filter { !$0.isEmpty }
}
