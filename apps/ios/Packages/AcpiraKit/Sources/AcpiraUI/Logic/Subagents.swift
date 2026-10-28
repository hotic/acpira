import AcpiraModels
import Foundation

// chat/subagents/subagentState.ts, the delegation rules of chat/processGroups.ts / planSections.ts, and the observe
// bookkeeping of App.tsx + Shell.tsx: which turn a child belongs to, where its row sits, what the row says, and which
// child's transcript the phone follows. Names follow the TypeScript one to one.

// MARK: - Placement

/// isDelegation: a root tool call that launched a subagent. It never renders as a tool: where its node is placed, the
/// subagent rows take its place, and everywhere else it is left out
func isDelegation(_ block: AgentBlock) -> Bool { block.toolCall?.subagentId != nil }

/// nodesByTurn: the nodes anchored to each root agent turn (`turnIndex`), in announce order
func nodesByTurn(_ nodes: [SubagentSummary]) -> [Int: [SubagentSummary]] {
  Dictionary(grouping: nodes, by: \.turnIndex)
}

/// delegatedIds: the turn's nodes that can sit at their delegation row, those whose id a root tool call carries
func delegatedIds(_ blocks: [AgentBlock], _ turnNodes: [SubagentSummary]) -> Set<String> {
  let ids = Set(turnNodes.map(\.id))
  return Set(blocks.compactMap { $0.toolCall?.subagentId }.filter(ids.contains))
}

/// placeNodes: each placed id takes itself and every descendant anchored to the same turn (the nearest placed ancestor
/// wins); nodes with no placed ancestor go to `rest`, which renders after the turn's content. Cycles stop at a visited id
func placeNodes(_ turnNodes: [SubagentSummary], placed: Set<String>) -> (byId: [String: [SubagentSummary]], rest: [SubagentSummary]) {
  let byNodeId = Dictionary(turnNodes.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
  var byId: [String: [SubagentSummary]] = [:]
  var rest: [SubagentSummary] = []
  for node in turnNodes {
    var seen = Set<String>()
    var cur: SubagentSummary? = node
    while let c = cur, !placed.contains(c.id), !seen.contains(c.id) {
      seen.insert(c.id)
      cur = c.parentId.flatMap { byNodeId[$0] }
    }
    if let c = cur, placed.contains(c.id) { byId[c.id, default: []].append(node) } else { rest.append(node) }
  }
  return (byId, rest)
}

/// rootRows: the group's rows, top-level children in announce order. A node whose parent was anchored to another turn
/// still shows here: a child row that never renders anywhere would be worse than a slightly wrong one
func rootRows(_ turnNodes: [SubagentSummary]) -> [SubagentSummary] {
  let ids = Set(turnNodes.map(\.id))
  return turnNodes.filter { $0.parentId == nil || !ids.contains($0.parentId!) }
}

/// descendantCount: every descendant through parentId edges; the visited set keeps a malformed cycle from looping
func descendantCount(_ id: String, _ all: [SubagentSummary]) -> Int {
  var seen: Set<String> = [id]
  var queue = [id]
  var count = 0
  while !queue.isEmpty {
    let cur = queue.removeFirst()
    for node in all where node.parentId == cur && !seen.contains(node.id) {
      seen.insert(node.id)
      queue.append(node.id)
      count += 1
    }
  }
  return count
}

/// breadcrumb: the ancestor chain root → node, the node itself last; stops on a visited id and caps at 16 links
func breadcrumb(_ id: String, _ all: [SubagentSummary]) -> [SubagentSummary] {
  let byId = Dictionary(all.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
  var chain: [SubagentSummary] = []
  var seen = Set<String>()
  var cur = byId[id]
  while let c = cur, !seen.contains(c.id), chain.count < 16 {
    seen.insert(c.id)
    chain.insert(c, at: 0)
    cur = c.parentId.flatMap { byId[$0] }
  }
  return chain
}

/// flattenTree: depth-first in announce order, depth capped at 8; children of a missing or cyclic parent still appear
func flattenTree(_ all: [SubagentSummary]) -> [(node: SubagentSummary, depth: Int)] {
  let ids = Set(all.map(\.id))
  var out: [(node: SubagentSummary, depth: Int)] = []
  var seen = Set<String>()
  func walk(_ node: SubagentSummary, _ depth: Int) {
    guard seen.insert(node.id).inserted else { return }
    out.append((node, min(depth, 8)))
    for child in all where child.parentId == node.id { walk(child, depth + 1) }
  }
  for root in all where root.parentId == nil || !ids.contains(root.parentId!) { walk(root, 0) }
  for node in all { walk(node, 0) }
  return out
}

// MARK: - States and labels

/// receiptOnly: the inspector shows the task and result alone, unless a receipt node gained a transcript of its own (a
/// Claude workflow agent whose sidechain log the engine reads)
func receiptOnly(_ node: SubagentSummary) -> Bool { node.visibility == .receipt && node.hasProcess != true }

/// isWaiting: the child is blocked on a card of its own
func isWaiting(_ node: SubagentSummary) -> Bool { !(node.permissions ?? []).isEmpty || node.question != nil }

/// countsLabel: tallies like "2 running, 1 waiting, 3 done"; a node waiting on a card counts in both buckets. Joined
/// with the locale's list separator rather than the web's " · "
func countsLabel(_ nodes: [SubagentSummary]) -> String {
  let buckets: [(String, Int)] = [
    ("subagents.running", nodes.filter { $0.state == .running }.count),
    ("subagents.waiting", nodes.filter(isWaiting).count),
    ("subagents.completed", nodes.filter { $0.state == .completed }.count),
    ("subagents.failed", nodes.filter { $0.state == .failed }.count),
    ("subagents.cancelled", nodes.filter { $0.state == .cancelled }.count),
    ("subagents.disconnected", nodes.filter { $0.state == .disconnected }.count),
  ]
  return buckets.filter { $0.1 > 0 }.map { t($0.0, ["n": $0.1]) }.joined(separator: t("common.listSep"))
}

/// stateLabel: the node's state in words; a state this build does not know reads as unknown
func stateLabel(_ node: SubagentSummary) -> String {
  t("subagents.state.\(node.state == .unknown ? SubagentState.disconnected.rawValue : node.state.rawValue)")
}

/// subagentTitle: the agent's short label, else its role
func subagentTitle(_ node: SubagentSummary) -> String { node.title ?? node.role ?? t("subagents.untitled") }

/// What the row says the child is doing: a pending card beats the state
func statusLabel(_ node: SubagentSummary) -> String {
  if !(node.permissions ?? []).isEmpty { return t("host.awaitingApproval") }
  if node.question != nil { return t("host.awaitingAnswers") }
  return stateLabel(node)
}

/// secondLine: the row's detail: a pending decision beats the live activity, a finished child quotes its result
func secondLine(_ node: SubagentSummary) -> String {
  if !(node.permissions ?? []).isEmpty { return t("host.awaitingApproval") }
  if node.question != nil { return t("host.awaitingAnswers") }
  if node.state == .running { return node.activity ?? t("host.working") }
  let line = node.result.flatMap { result in
    resultText(result).split(whereSeparator: \.isNewline).lazy
      .map { $0.trimmingCharacters(in: .whitespaces) }.first { !$0.isEmpty }
  }
  if let line { return line.count > 120 ? String(line.prefix(119)) + "…" : line }
  return stateLabel(node)
}

/// resultText: a schema-bound workflow agent returns a JSON object that the CLI's preview cuts off, so it rarely
/// parses. The row quotes its prose: the `summary` field when there is one, else the first string values in order
/// (joined with the locale's list separator, not the web's " · "). A result not shaped like a JSON object passes
/// through unchanged
func resultText(_ result: String) -> String {
  guard result.drop(while: \.isWhitespace).first == "{" else { return result }
  // A string field of a JSON object, up to its closing quote or the end of a cut-off text
  let field = /"([^"\\]+)"\s*:\s*"((?:[^"\\]|\\.)*)"?/
  let fields = result.matches(of: field).map { (String($0.1), unescapeJSON(String($0.2))) }
    .filter { !$0.1.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
  guard !fields.isEmpty else { return result }
  return fields.first { $0.0 == "summary" }?.1 ?? fields.prefix(3).map(\.1).joined(separator: t("common.listSep"))
}

private func unescapeJSON(_ raw: String) -> String {
  // A cut can land inside an escape sequence; drop the partial one
  let whole = raw.replacing(/\\(?:u[0-9a-fA-F]{0,3})?$/, with: "")
  return (try? JSONDecoder().decode(String.self, from: Data("\"\(whole)\"".utf8))) ?? whole
}

/// elapsedMs: from the announcement to the end, or to `now` (ms since the epoch) while it runs
func elapsedMs(_ node: SubagentSummary, now: Double) -> Double { max(0, (node.endedAt ?? now) - node.announcedAt) }

/// elapsedText: the tool rows' whole-second vocabulary
func elapsedText(_ node: SubagentSummary, now: Double) -> String {
  commandDuration(ms: elapsedMs(node, now: now), whole: true)
}

// MARK: - Observing

/// The observed children's transcripts (App.tsx `subagentTranscripts`) and the one child this phone follows (Shell.tsx's
/// observe effect). The engine keeps one observed child per view and ignores an unobserve that names another, so
/// following a new child ends the previous one first, and a page that leaves after the next one came does no harm
struct SubagentFeed {
  struct Key: Hashable {
    var session: String
    var id: String
  }

  private(set) var transcripts: [Key: SubagentTranscript] = [:]
  private(set) var observed: Key?

  func transcript(session: String, id: String) -> SubagentTranscript? { transcripts[Key(session: session, id: id)] }

  /// Follows a child: the messages to post, the previous child's unobserve first. Transcripts of other sessions go, as
  /// the webview drops them when the session changes
  mutating func observe(session: String, id: String) -> [WebviewMsg] {
    let key = Key(session: session, id: id)
    guard observed != key else { return [] }
    var out: [WebviewMsg] = []
    if let prev = observed { out.append(.unobserveSubagent(sessionId: prev.session, subagentId: prev.id)) }
    observed = key
    transcripts = transcripts.filter { $0.key.session == session }
    out.append(.observeSubagent(sessionId: session, subagentId: id))
    return out
  }

  /// Stops following a child; nothing when another one is followed by now
  mutating func unobserve(session: String, id: String) -> [WebviewMsg] {
    guard observed == Key(session: session, id: id) else { return [] }
    observed = nil
    return [.unobserveSubagent(sessionId: session, subagentId: id)]
  }

  /// A `subagent` push: kept unless what is held is as new or newer. Answers whether it changed anything
  @discardableResult
  mutating func receive(session: String, id: String, transcript: SubagentTranscript) -> Bool {
    let key = Key(session: session, id: id)
    if let held = transcripts[key], transcript.rev <= held.rev { return false }
    transcripts[key] = transcript
    return true
  }
}
