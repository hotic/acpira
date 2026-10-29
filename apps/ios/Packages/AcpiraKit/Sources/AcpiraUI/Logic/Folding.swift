import AcpiraModels
import Foundation

// Ports of the webview's pure transcript logic: chat/processGroups.ts, chat/folding.ts, chat/toolDetails.ts,
// chat/commandDuration.ts, chat/turnOutcome.ts and the label helpers in ToolCall.tsx / ToolGroup.tsx.
// Function names and rules follow the TypeScript one to one, so a behaviour change there maps onto one place here.

// MARK: - Codex split

/// A turn's blocks split the way CodexMessage renders them
public struct CodexSplit {
  public var process: [AgentBlock]
  public var reply: [TextBlock]
  public var permissions: [PermissionBlock]
  public var notices: [NoticeBlock]
  /// Goal milestones before any other block, which open the turn above the fold
  public var goalsLead: [GoalBlock] = []
  /// Goal milestones after the turn's last action, which follow the reply
  public var goals: [GoalBlock] = []
}

/// splitCodexBlocks: the text after the last action stays outside the process fold; explicit phases win. Goal
/// milestones are status rows: the leading ones open the turn, the ones after the last action follow the reply, and
/// one that more actions follow (a `/goal` steered into a running turn) keeps its place in the process
public func splitCodexBlocks(_ blocks: [AgentBlock]) -> CodexSplit {
  var permissions: [PermissionBlock] = []
  var notices: [NoticeBlock] = []
  var content: [AgentBlock] = []
  var goalsLead: [GoalBlock] = []
  var goals: [GoalBlock] = []
  func isAction(_ b: AgentBlock) -> Bool {
    switch b {
    case .goal, .text, .permission, .notice: false
    case .question(let q): q.outcome != nil
    default: true
    }
  }
  let firstOther = blocks.firstIndex { if case .goal = $0 { false } else { true } }
  let lastAction = blocks.lastIndex(where: isAction)
  for (i, block) in blocks.enumerated() {
    switch block {
    case .permission(let p): permissions.append(p)
    case .notice(let n): notices.append(n)
    case .question(let q) where q.outcome == nil: break
    case .goal(let g) where firstOther.map({ i < $0 }) ?? true: goalsLead.append(g)
    case .goal(let g) where lastAction.map({ i > $0 }) ?? true: goals.append(g)
    default: content.append(block)
    }
  }
  let foldable = content.contains { if case .toolCall = $0 { true } else { false } }
  var end = content.count
  // Thinking and to-do bookkeeping between closing paragraphs join the fold in a foldable turn
  func tailProcess(_ b: AgentBlock) -> Bool { switch b { case .thought, .plan: true; default: false } }
  while end > 0 {
    let last = content[end - 1]
    if case .text = last { end -= 1; continue }
    if foldable && tailProcess(last) { end -= 1; continue }
    break
  }
  var process: [AgentBlock] = []
  var reply: [TextBlock] = []
  for (i, block) in content.enumerated() {
    if case .text(let text) = block {
      let isReply = text.phase == .final || (i >= end && text.phase != .commentary)
      if isReply { reply.append(text) } else { process.append(block) }
    } else {
      process.append(block)
    }
  }
  return CodexSplit(process: process, reply: reply, permissions: permissions, notices: notices, goalsLead: goalsLead,
                    goals: goals)
}

// MARK: - Process items

public enum GroupKind: String, Sendable { case read, edit }

/// One item of a turn's process list: a block on its own, or a run of consecutive reads / edits under one head
public enum ProcessItem: Identifiable, Hashable {
  case block(id: String, block: AgentBlock)
  case group(id: String, kind: GroupKind, blocks: [ToolCallBlock])

  public var id: String {
    switch self { case .block(let id, _), .group(let id, _, _): id }
  }
}

func groupKind(_ block: AgentBlock) -> GroupKind? {
  guard case .toolCall(let b) = block, b.status != .failed, b.status != .cancelled, b.asyncTask == nil,
        b.subagentId == nil else { return nil }
  // A read with locations always names files; only one without them needs `toolFiles`, which runs on every patch
  if b.kind == .read { return b.status != .completed || !(b.locations ?? []).isEmpty || !toolFiles(b).isEmpty ? .read : nil }
  if b.kind == .edit { return .edit }
  return nil
}

public func blockItemId(_ block: AgentBlock, at: Int) -> String {
  if case .toolCall(let b) = block { return b.id }
  if case .thought(let t) = block, let started = t.startedAt { return "thought:\(Int(started))" }
  return block.blockId ?? "at:\(at)"
}

/// groupProcess: consecutive reads (with files) or edits share one head
public func groupProcess(_ blocks: [AgentBlock]) -> [ProcessItem] {
  var out: [ProcessItem] = []
  var run: [ToolCallBlock] = []
  var runKind: GroupKind?
  var runAt = 0
  func flush() {
    if run.count == 1 { out.append(.block(id: blockItemId(.toolCall(run[0]), at: runAt), block: .toolCall(run[0]))) }
    else if run.count > 1 { out.append(.group(id: run[0].id, kind: runKind!, blocks: run)) }
    run = []
  }
  for (at, block) in blocks.enumerated() {
    let kind = groupKind(block)
    if let kind, kind == runKind, let call = block.toolCall { run.append(call); continue }
    flush()
    runKind = kind
    if kind != nil, let call = block.toolCall { run.append(call); runAt = at }
    else { out.append(.block(id: blockItemId(block, at: at), block: block)) }
  }
  flush()
  return out
}

/// itemSettled: none of its calls pending or running, its thought no longer streaming
public func itemSettled(_ item: ProcessItem) -> Bool {
  let blocks: [AgentBlock] = switch item {
  case .group(_, _, let calls): calls.map { .toolCall($0) }
  case .block(_, let block): [block]
  }
  return blocks.allSatisfy { block in
    switch block {
    case .thought(let t): t.streaming != true
    case .toolCall(let b): b.background == true || (b.status != .pending && b.status != .in_progress)
    default: true
    }
  }
}

/// foldStates: the latest item is open while the turn works; a finished one closes once the next has started and it
/// has been open `Motion.foldDwell`; a lone edit never opens on its own; a manual choice wins
public struct FoldInput {
  public var items: [ProcessItem]
  public var now: Double
  public var live: Bool
  public var autoExpand: Bool
  public var following: Bool
  public var first: Bool
  public var held: Set<String>
  public var manual: (String) -> Bool?
}

public func foldStates(_ input: FoldInput, doneAt: inout [String: Double]) -> (folds: [String: Bool], open: Set<String>, wake: Double) {
  let dwell = 900.0
  let tail = input.items.last?.id
  var folds: [String: Bool] = [:]
  var open = Set<String>()
  var wake = Double.infinity
  for item in input.items {
    if itemSettled(item) { if doneAt[item.id] == nil { doneAt[item.id] = input.first ? -.infinity : input.now } }
    else { doneAt[item.id] = nil }
    let until = (doneAt[item.id] ?? .infinity) + dwell
    var diff = false
    if case .block(_, .toolCall(let b)) = item, b.kind == .edit { diff = true }
    var auto = input.live && input.autoExpand && !diff && (item.id == tail || input.now < until)
    if auto && item.id != tail && until != .infinity { wake = min(wake, until) }
    if !auto && !input.following && input.held.contains(item.id) { auto = true }
    if auto { open.insert(item.id) }
    folds[item.id] = input.manual(item.id) ?? auto
  }
  return (folds, open, wake)
}

// MARK: - Tool details

private let editReceipts: Set<String> = ["Edit applied successfully.", "Wrote file successfully."]

/// visibleToolContents: a completed edit's success receipt is dropped, everything else stays
public func visibleToolContents(_ block: ToolCallBlock) -> [ToolContent] {
  let items = block.contents ?? (block.content.map { [$0] } ?? [])
  guard block.kind == .edit, block.status == .completed else { return items }
  return items.filter { if case .text(let t) = $0 { !editReceipts.contains(t.trimmingCharacters(in: .whitespacesAndNewlines)) } else { true } }
}

/// fileReference: strip a trailing `:line[-end][:col]`
public func fileReference(_ hit: String) -> (path: String, line: Int?) {
  if let m = hit.firstMatch(of: FilePatterns.lineSuffix) { return (String(m.1), Int(m.2)) }
  return (hit, nil)
}

private func fileHit(_ raw: String) -> String? {
  var text = raw
  if text.hasPrefix("file://") {
    guard let url = URL(string: text) else { return nil }
    text = (url.host().map { "//\($0)" } ?? "") + url.path(percentEncoded: false)
  }
  let hit = text.firstMatch(of: FilePatterns.hitLine)
  let path = hit.map { String($0.1) } ?? text
  let explicit = path.firstMatch(of: FilePatterns.explicitPath) != nil
  let bare = path.wholeMatch(of: FilePatterns.barePath) != nil
  guard explicit || bare else { return nil }
  return hit.map { "\(path):\($0.2)" } ?? path
}

private let lineless = /(?i)\.(?:png|jpe?g|gif|webp|bmp|ico|avif|heic|tiff?|pdf|zip|gz|tgz|wasm|mp[34]|mov|wav|woff2?|ttf|otf)$/

/// The file-reference patterns, compiled once: a regex literal is compiled again each time it is evaluated, and these
/// run for the process rows on every streamed patch
private enum FilePatterns {
  nonisolated(unsafe) static let lineSuffix = /^(.+?):(\d+)(?:[–-]\d+)?(?::\d+)?$/
  nonisolated(unsafe) static let hitLine = /^(.+?):(\d+)(?::\d+)?(?::.*)?$/
  nonisolated(unsafe) static let explicitPath = /^(?:\.{0,2}\/|[A-Za-z]:[\\\/])/
  nonisolated(unsafe) static let barePath = /[^\s:]+[.][\w-]+/
  nonisolated(unsafe) static let rangeSuffix = /^(.+?):(\d+(?:[–-]\d+)?)(?::\d+)?$/
}

/// toolFiles: the file references a read or search names
public func toolFiles(_ block: ToolCallBlock) -> [String] {
  guard block.kind == .read || block.kind == .search else { return [] }
  var files: [String] = (block.locations ?? []).map { l in
    if block.kind == .read && l.path.firstMatch(of: lineless) != nil { return l.path }
    if block.kind == .read, let r = block.readRange, r.path == l.path {
      return "\(l.path):\(r.start)" + (r.end.map { $0 != r.start ? "–\($0)" : "" } ?? "")
    }
    guard let line = l.line, !(block.kind == .read && line <= 1) else { return l.path }
    return "\(l.path):\(line)"
  }
  if block.kind == .search, let content = block.content {
    let lines: [String] = switch content {
    case .list(let items): items
    case .text(let text): text.components(separatedBy: "\n")
    default: []
    }
    for line in lines { if let hit = fileHit(line.trimmingCharacters(in: .whitespaces)) { files.append(hit) } }
  }
  if files.isEmpty, block.kind == .read, let target = block.target, let hit = fileHit(target) { files.append(hit) }
  var seen = Set<String>()
  return files.filter { seen.insert($0).inserted }
}

/// splitHit: a reference as the name shown and its faint line note
public func splitHit(_ hit: String) -> (main: String, aside: String?) {
  if let m = hit.firstMatch(of: FilePatterns.rangeSuffix) {
    return (lastComponent(String(m.1)), "L\(m.2)")
  }
  if hit.firstMatch(of: FilePatterns.explicitPath) != nil { return (lastComponent(hit), nil) }
  if let url = URL(string: hit), let host = url.host(), url.scheme != nil {
    return (url.path().isEmpty || url.path() == "/" ? host : host + url.path(), host)
  }
  return (hit, nil)
}

func lastComponent(_ path: String) -> String {
  path.split(whereSeparator: { $0 == "/" || $0 == "\\" }).last.map(String.init) ?? path
}

public func filePath(_ block: ToolCallBlock) -> String {
  if let path = block.locations?.first?.path { return path }
  return block.target ?? ""
}

public func fileName(_ block: ToolCallBlock) -> String {
  let name = lastComponent(filePath(block))
  return name.isEmpty ? block.target ?? "" : name
}

/// groupNames: the files a group names in its head, first-seen order
public func groupNames(_ blocks: [ToolCallBlock]) -> [String] {
  var seen = Set<String>()
  return blocks.map(fileName).filter { !$0.isEmpty && seen.insert($0).inserted }
}

/// editEntries: consecutive edits of one file share an entry
public func editEntries(_ blocks: [ToolCallBlock]) -> [(key: String, blocks: [ToolCallBlock])] {
  var order: [String] = []
  var map: [String: [ToolCallBlock]] = [:]
  for block in blocks {
    let key = filePath(block).isEmpty ? block.id : filePath(block)
    if map[key] == nil { order.append(key) }
    map[key, default: []].append(block)
  }
  return order.map { ($0, map[$0]!) }
}

/// editSpan: first to last changed line, e.g. L535–549
public func editSpan(_ blocks: [ToolCallBlock]) -> String? {
  var lo = Int.max, hi = Int.min
  for block in blocks {
    for case .diff(let lines, _) in visibleToolContents(block) {
      let adds = lines.filter { $0.kind == .add }.compactMap(\.newLine)
      for n in adds.isEmpty ? lines.filter({ $0.kind == .del }).compactMap(\.oldLine) : adds {
        lo = min(lo, n); hi = max(hi, n)
      }
    }
  }
  if lo == .max { return nil }
  return lo == hi ? "L\(lo)" : "L\(lo)–\(hi)"
}

public func diffStatOf(_ blocks: [ToolCallBlock]) -> DiffStat? {
  guard blocks.contains(where: { $0.diffStat != nil }) else { return nil }
  return blocks.reduce(DiffStat(add: 0, del: 0)) { DiffStat(add: $0.add + ($1.diffStat?.add ?? 0), del: $0.del + ($1.diffStat?.del ?? 0)) }
}

/// namesLabel: two names, the rest as a count
public func namesLabel(_ names: [String]) -> String {
  let sep = t("turns.nameSep")
  return names.count <= 2 ? names.joined(separator: sep)
    : t("turns.groupFiles", ["names": names.prefix(2).joined(separator: sep), "n": names.count])
}

// MARK: - Labels

/// toolVerb: 已读取 / 正在运行 / 运行失败 …, rendered from the semantic kind
public func toolVerb(_ block: ToolCallBlock) -> String {
  if block.observation == .unknown { return t("chatgpt.unknownTool") }
  let key = switch block.status {
  case .pending: "fold.queued"
  case .in_progress: "fold.pending"
  case .completed: "fold.done"
  case .failed: "fold.failed"
  case .cancelled: "fold.cancelled"
  }
  return t(key, ["verb": t(block.verbKey ?? "verb.\(block.kind.rawValue)")])
}

/// folding.ts clockStart: where a turn's clock starts, its first streamed output (the send time on older transcripts);
/// a turn that has shown nothing yet has no clock
public func clockStart(_ turn: AgentTurn) -> Double? {
  if let responded = turn.respondedAt { return responded }
  return !turn.blocks.isEmpty || turn.endedAt != nil ? turn.startedAt : nil
}

/// folding.ts durationText: "5m 35s" for a span in ms, shared by the finished head and the live counter
public func durationText(ms: Double) -> String {
  let seconds = max(0, Int((ms / 1000).rounded()))
  let minutes = seconds / 60, rest = seconds % 60
  if minutes > 0 { return rest > 0 ? t("turns.elapsed.ms", ["m": minutes, "s": rest]) : t("turns.elapsed.m", ["m": minutes]) }
  return t("turns.elapsed.s", ["s": rest])
}

public func elapsedDuration(_ turn: AgentTurn) -> String {
  guard let start = clockStart(turn), let end = turn.endedAt else { return "" }
  return durationText(ms: end - start)
}

/// The finished head's time: the bare duration ("5m 35s"), as the webview dropped its "took {t}" wrapper key
public func elapsedLabel(_ turn: AgentTurn) -> String {
  guard turn.startedAt != nil, turn.endedAt != nil else { return t("turns.done") }
  return elapsedDuration(turn)
}

/// commandDuration: tenths below ten seconds once finished; `whole` is the live clock
public func commandDuration(ms: Double, whole: Bool = false) -> String {
  let clamped = max(0, ms)
  if !whole && clamped < 10_000 {
    return t("turns.elapsed.s", ["s": clamped < 100 ? "<0.1" : String(format: "%.1f", clamped / 1000)])
  }
  let seconds = whole ? Int(clamped / 1000) : Int((clamped / 1000).rounded())
  let m = seconds / 60, s = seconds % 60
  return m > 0 ? (s > 0 ? t("turns.elapsed.ms", ["m": m, "s": s]) : t("turns.elapsed.m", ["m": m])) : t("turns.elapsed.s", ["s": s])
}

/// turnOutcome: a line for a short stop, or for a normal end with nothing to show (a slash command's receipt names
/// the setting changes it reported); a turn whose observation was lost says so first
public func turnOutcome(_ turn: AgentTurn) -> String? {
  if turn.observation == .unknown { return t("chatgpt.unknownTurn") }
  switch turn.stop {
  case .error: return t("turns.stop.error")
  case .refusal: return t("turns.stop.refusal")
  case .max_tokens: return t("turns.stop.maxTokens")
  case .max_turn_requests: return t("turns.stop.maxTurns")
  case .cancelled: return t("turns.stop.cancelled")
  case .end_turn:
    let empty = turn.blocks.allSatisfy { if case .text(let t) = $0 { t.markdown.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty } else { false } }
    guard empty else { return nil }
    guard let command = turn.command else { return t("turns.stop.empty") }
    let changes = [command.mode.map { t("turns.command.mode", ["mode": $0]) } ?? ""]
      + (command.options ?? []).map { t("turns.command.option", ["name": $0.name, "value": $0.value]) }
    let said = changes.filter { !$0.isEmpty }
    return said.isEmpty ? t("turns.command.empty") : said.joined(separator: t("common.listSep"))
  default: return nil
  }
}

/// The live head of a working turn (Turns.tsx liveActivity)
public struct LiveActivity {
  public enum Lead { case orb, hand, question, plan }
  public var label: String
  public var active: Bool
  public var lead: Lead
  /// The turn reads as done while a plan waits on its card: the clock stops
  public var quiet = false
}

public func liveActivity(_ turn: AgentTurn) -> LiveActivity {
  if turn.blocks.contains(where: { if case .permission(let p) = $0 { p.planId == nil } else { false } }) {
    return LiveActivity(label: t("host.awaitingApproval"), active: false, lead: .hand)
  }
  // A plan approval waits on its own card (build / revise there), so the turn reads as done
  if turn.blocks.contains(where: { if case .permission = $0 { true } else { false } }) {
    return LiveActivity(label: t("plan.awaitingBuild"), active: false, lead: .plan, quiet: true)
  }
  if turn.blocks.contains(where: { if case .question(let q) = $0 { q.outcome == nil } else { false } }) {
    return LiveActivity(label: t("host.awaitingAnswers"), active: false, lead: .question)
  }
  if let retry = turn.retry {
    let label = retry.attempt != nil && retry.max != nil
      ? t("turns.retryingAttempt", ["attempt": retry.attempt!, "max": retry.max!]) : t("turns.retrying")
    return LiveActivity(label: label, active: true, lead: .orb)
  }
  return LiveActivity(label: t("host.working"), active: true, lead: .orb)
}

// MARK: - Exchanges

/// A user prompt and the agent turns that answer it; the transcript scrolls by exchange so the prompt card can stick
public struct Exchange: Identifiable {
  public var id: Int
  public var user: UserTurn?
  public var agents: [(index: Int, turn: AgentTurn)]
}

public func exchanges(_ turns: [Turn]) -> [Exchange] {
  var out: [Exchange] = []
  for (i, turn) in turns.enumerated() {
    switch turn {
    case .user(let u):
      out.append(Exchange(id: i, user: u, agents: []))
    case .agent(let a):
      if out.isEmpty { out.append(Exchange(id: i, user: nil, agents: [])) }
      out[out.count - 1].agents.append((i, a))
    }
  }
  return out
}

/// The entrance name of a process item, apart from the ids rows use inside (a thought's own, a tool's `:tool`)
func itemEntrance(_ id: String) -> String { "item:\(id)" }
