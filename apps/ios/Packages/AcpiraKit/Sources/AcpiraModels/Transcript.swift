import Foundation

// Codable mirror of packages/shared/transcript.ts: the session view the engine pushes and the webview renders.
// Decoding is tolerant: unknown enum strings fall back to an `unknown` case and unknown block types decode to
// `.unknown` instead of failing the whole view, so an engine one release ahead still renders what this app knows.

public typealias AgentId = String

// MARK: - Tolerant string enums

/// A string enum that keeps unknown values instead of throwing
public protocol TolerantEnum: RawRepresentable, Codable, Hashable, Sendable where RawValue == String {
  static var unknownCase: Self { get }
}

extension TolerantEnum {
  public init(from decoder: Decoder) throws {
    let raw = try decoder.singleValueContainer().decode(String.self)
    self = Self(rawValue: raw) ?? Self.unknownCase
  }
  public func encode(to encoder: Encoder) throws {
    var c = encoder.singleValueContainer()
    try c.encode(rawValue)
  }
}

public enum SessionStatus: String, TolerantEnum {
  case starting, ready, auth_required, readonly, error, closed, unknown
  public static var unknownCase: Self { .unknown }
}

public enum ToolKind: String, TolerantEnum {
  case read, edit, delete, move, search, execute, think, fetch, switch_mode, other
  public static var unknownCase: Self { .other }
}

public enum ToolStatus: String, TolerantEnum {
  case pending, in_progress, completed, failed, cancelled
  public static var unknownCase: Self { .completed }
}

public enum PlanStatus: String, TolerantEnum {
  case pending, in_progress, completed
  public static var unknownCase: Self { .pending }
}

public enum PermissionKind: String, TolerantEnum {
  case allow_once, allow_always, reject_once, reject_always, unknown
  public static var unknownCase: Self { .unknown }
}

public enum CompactionStatus: String, TolerantEnum {
  case in_progress, completed, failed, cancelled
  public static var unknownCase: Self { .completed }
}

public enum QuestionKind: String, TolerantEnum {
  case single, multiple, text
  public static var unknownCase: Self { .text }
}

public enum QuestionOutcome: String, TolerantEnum {
  case answered, skipped, cancelled
  public static var unknownCase: Self { .cancelled }
}

public enum TurnStop: String, TolerantEnum {
  case end_turn, max_tokens, max_turn_requests, refusal, cancelled, error, unknown
  public static var unknownCase: Self { .unknown }
}

public enum FailureAction: String, TolerantEnum {
  case retry, login, new_session, unknown
  public static var unknownCase: Self { .unknown }
}

public enum SessionState: String, TolerantEnum {
  // background: the turn ended but a background task it started still runs
  case working, waiting, unread, error, background, unknown
  public static var unknownCase: Self { .unknown }
}

/// `observation: 'unknown'` on a tool call or turn: the engine lost sight of it; not a receipt that the remote work stopped
public enum ObservationState: String, TolerantEnum {
  case unknown
  public static var unknownCase: Self { .unknown }
}

/// `limited` is Codex's usageLimited / budgetLimited
public enum GoalStatus: String, TolerantEnum, CaseIterable {
  case active, paused, blocked, limited, complete, unknown
  public static var unknownCase: Self { .unknown }
}

/// A goal control the agent advertises at initialize; `set` is also what `/goal <objective>` does
public enum GoalAction: String, TolerantEnum, CaseIterable {
  case set, pause, resume, clear, unknown
  public static var unknownCase: Self { .unknown }
}

public enum GoalEvent: String, TolerantEnum, CaseIterable {
  case set, paused, resumed, blocked, limited, complete, cleared, unknown
  public static var unknownCase: Self { .unknown }
}

// MARK: - Agents and accounts

public struct AgentInstall: Codable, Hashable, Sendable {
  public var command: String?
  public var docs: String?
}

public struct QuotaWindow: Codable, Hashable, Sendable {
  public var id: String
  public var remaining: Double
  public var resetsAt: String?
}

public struct AccountQuota: Codable, Hashable, Sendable {
  public var windows: [QuotaWindow]
  public var onDemandBalanceUsd: Double?
  public var fetchedAt: String
}

public struct LocalAccountInfo: Codable, Hashable, Sendable {
  public var label: String
  public var detail: String?
  public var status: String
  public var quota: AccountQuota?
}

public struct AgentInfo: Codable, Hashable, Sendable, Identifiable {
  public var id: AgentId
  public var name: String
  public var external: Bool?
  public var accounts: Bool?
  public var credentialsLocked: Bool?
  public var localAccount: LocalAccountInfo?
  public var available: Bool?
  public var disabled: Bool?
  /// Commands a probe looked for and did not find, and the directories it searched, for the install hint
  public var missing: [String]?
  public var searched: [String]?
  public var install: AgentInstall?
  public init(id: AgentId, name: String) { self.id = id; self.name = name }
}

/// Why an account shows no quota bars: the last read failed and the next one waits
public enum AccountQuotaIssue: String, TolerantEnum {
  case rate_limited, expired, unavailable, unknown
  public static var unknownCase: Self { .unknown }
}

public struct AccountInfo: Codable, Hashable, Sendable, Identifiable {
  public var id: String
  public var agent: AgentId
  public var label: String
  public var detail: String?
  public var addedAt: String?
  public var lastUsedAt: String?
  public var quota: AccountQuota?
  /// Absent while `quota` is set
  public var quotaIssue: AccountQuotaIssue?
}

// MARK: - Controls

public struct SessionOption: Codable, Hashable, Sendable, Identifiable {
  public struct Group: Codable, Hashable, Sendable { public var id: String; public var name: String }
  /// A provider verified by the agent adapter (official / custom), never inferred from a display name
  public struct Source: Codable, Hashable, Sendable {
    public var id: String; public var name: String; public var kind: String
    public init(id: String, name: String, kind: String) { self.id = id; self.name = name; self.kind = kind }
  }
  public var id: String
  public var name: String
  public var description: String?
  public var group: Group?
  public var source: Source?
  public var kind: String?
  public init(id: String, name: String, description: String? = nil) {
    self.id = id; self.name = name; self.description = description
  }
}

public struct ConfigControl: Codable, Hashable, Sendable, Identifiable {
  public var id: String
  public var name: String
  public var category: String?
  public var type: String?
  public var options: [SessionOption]
  public var value: String?
  public init(id: String, name: String, category: String? = nil, type: String? = nil, options: [SessionOption] = [], value: String? = nil) {
    self.id = id; self.name = name; self.category = category; self.type = type; self.options = options; self.value = value
  }
}

public struct SessionControls: Codable, Hashable, Sendable {
  public var modes: [SessionOption]
  public var modeId: String?
  public var modeConfigId: String?
  public var options: [ConfigControl]
  public init(modes: [SessionOption] = [], modeId: String? = nil, options: [ConfigControl] = []) {
    self.modes = modes; self.modeId = modeId; self.options = options
  }
  public init(from decoder: Decoder) throws {
    let c = try decoder.container(keyedBy: CodingKeys.self)
    modes = try c.decodeIfPresent([SessionOption].self, forKey: .modes) ?? []
    modeId = try c.decodeIfPresent(String.self, forKey: .modeId)
    modeConfigId = try c.decodeIfPresent(String.self, forKey: .modeConfigId)
    options = try c.decodeIfPresent([ConfigControl].self, forKey: .options) ?? []
  }
}

public struct SlashCommand: Codable, Hashable, Sendable {
  public struct Input: Codable, Hashable, Sendable { public var hint: String }
  public var name: String
  public var description: String
  public var input: Input?
}

// MARK: - Blocks

public struct ImageRef: Codable, Hashable, Sendable {
  public var blob: String?
  public var mimeType: String
  public var uri: String?
}

public struct DiffLine: Codable, Hashable, Sendable {
  public enum Kind: String, TolerantEnum {
    case hunk, add, del, ctx
    public static var unknownCase: Self { .ctx }
  }
  public var kind: Kind
  public var text: String
  public var oldLine: Int?
  public var newLine: Int?
  public init(kind: Kind, text: String, oldLine: Int? = nil, newLine: Int? = nil) {
    self.kind = kind; self.text = text; self.oldLine = oldLine; self.newLine = newLine
  }
}

/// The whole files behind a diff, as the `diffSource` push answers with them
public struct FullDiffSource: Codable, Hashable, Sendable {
  public var path: String
  public var oldText: String
  public var newText: String

  public init(path: String, oldText: String, newText: String) { self.path = path; self.oldText = oldText; self.newText = newText }
}

/// What a diff was taken from (DiffSource). Records keep both texts; a page view carries the path only (`omitted`), and
/// the page asks for the texts when it needs them (WebviewMsg `diffSource`)
public enum DiffSource: Codable, Hashable, Sendable {
  case full(FullDiffSource)
  case omitted(path: String)

  public var path: String {
    switch self {
    case .full(let source): source.path
    case .omitted(let path): path
    }
  }

  private enum Keys: String, CodingKey { case path, oldText, newText, omitted }
  public init(from decoder: Decoder) throws {
    let c = try decoder.container(keyedBy: Keys.self)
    let path = try c.decode(String.self, forKey: .path)
    // Without both texts it reads as omitted
    if try c.decodeIfPresent(Bool.self, forKey: .omitted) != true, let old = try c.decodeIfPresent(String.self, forKey: .oldText),
       let new = try c.decodeIfPresent(String.self, forKey: .newText) {
      self = .full(FullDiffSource(path: path, oldText: old, newText: new))
    } else {
      self = .omitted(path: path)
    }
  }
  public func encode(to encoder: Encoder) throws {
    switch self {
    case .full(let source): try source.encode(to: encoder)
    case .omitted(let path):
      var c = encoder.container(keyedBy: Keys.self)
      try c.encode(path, forKey: .path)
      try c.encode(true, forKey: .omitted)
    }
  }
}

public enum ToolContent: Codable, Hashable, Sendable {
  case text(String)
  /// source: the files behind it (the path alone in a page view)
  case diff([DiffLine], source: DiffSource? = nil)
  case list([String])
  case image(ImageRef)
  case unknown

  private enum Keys: String, CodingKey { case type, text, lines, source, items, blob, mimeType, uri }
  public init(from decoder: Decoder) throws {
    let c = try decoder.container(keyedBy: Keys.self)
    switch try c.decode(String.self, forKey: .type) {
    case "text": self = .text(try c.decode(String.self, forKey: .text))
    case "diff": self = .diff(try c.decode([DiffLine].self, forKey: .lines), source: try c.decodeIfPresent(DiffSource.self, forKey: .source))
    case "list": self = .list(try c.decode([String].self, forKey: .items))
    case "image": self = .image(try ImageRef(from: decoder))
    default: self = .unknown
    }
  }
  public func encode(to encoder: Encoder) throws {
    var c = encoder.container(keyedBy: Keys.self)
    switch self {
    case .text(let t): try c.encode("text", forKey: .type); try c.encode(t, forKey: .text)
    case let .diff(l, source): try c.encode("diff", forKey: .type); try c.encode(l, forKey: .lines); try c.encodeIfPresent(source, forKey: .source)
    case .list(let i): try c.encode("list", forKey: .type); try c.encode(i, forKey: .items)
    case .image(let r): try c.encode("image", forKey: .type); try r.encode(to: encoder)
    case .unknown: try c.encode("unknown", forKey: .type)
    }
  }
}

public struct DiffStat: Codable, Hashable, Sendable {
  public var add: Int
  public var del: Int
  public init(add: Int, del: Int) { self.add = add; self.del = del }
}

/// An AIR async task: background work the adapter reports apart from the prompt lifecycle. state: running | paused |
/// completed | failed | stopped, the last three final
public struct AsyncTaskInfo: Codable, Hashable, Sendable {
  public struct Usage: Codable, Hashable, Sendable {
    public var totalTokens: Int?
    public var toolUses: Int?
    public var durationMs: Double?
  }
  public var id: String
  public var state: String
  public var canStop: Bool?
  public var taskType: String?
  public var name: String?
  public var description: String?
  public var summary: String?
  public var lastToolName: String?
  public var outputFilePath: String?
  public var usage: Usage?
  /// A stop the engine asked for that the adapter's own state has not confirmed yet
  public var stopRequested: Bool?
}

public struct ToolLocation: Codable, Hashable, Sendable {
  public var path: String
  public var line: Int?
}

public struct ReadRange: Codable, Hashable, Sendable {
  public var path: String
  public var start: Int
  public var end: Int?
}

public struct PlanEntry: Codable, Hashable, Sendable {
  public var title: String
  public var status: PlanStatus
  public var priority: String?
}

public struct ToolCallBlock: Codable, Hashable, Sendable {
  /// Observation lost: the row must not pass for running work
  public var observation: ObservationState?
  public var id: String
  public var kind: ToolKind
  public var verb: String
  public var verbKey: String?
  /// The MCP server the call went to, as a label: the row reads "Call <server>" with the tool's title as its target
  public var mcpServer: String?
  public var target: String?
  public var targetMono: Bool?
  public var locations: [ToolLocation]?
  public var readRange: ReadRange?
  public var status: ToolStatus
  public var background: Bool?
  public var asyncTask: AsyncTaskInfo?
  public var startedAt: Double?
  public var endedAt: Double?
  public var meta: String?
  public var diffStat: DiffStat?
  public var todoEntries: [PlanEntry]?
  public var content: ToolContent?
  public var contents: [ToolContent]?
  public var subagentId: String?

  /// Every content item, `content` first (the contract carries either or both)
  public var allContents: [ToolContent] {
    (content.map { [$0] } ?? []) + (contents ?? [])
  }
}

public struct ThoughtBlock: Codable, Hashable, Sendable {
  public var text: String
  public var startedAt: Double?
  public var durationSec: Double?
  public var streaming: Bool?
}

public struct PlanBlock: Codable, Hashable, Sendable {
  public var entries: [PlanEntry]
  public var changed: Bool?
}

public struct TextBlock: Codable, Hashable, Sendable {
  public enum Phase: String, TolerantEnum {
    case commentary, final
    public static var unknownCase: Self { .commentary }
  }
  public var id: String?
  public var phase: Phase?
  public var markdown: String
  public var streaming: Bool?
  public var images: [ImageRef]?
}

public struct PermissionOption: Codable, Hashable, Sendable, Identifiable {
  public var id: String
  public var label: String
  public var kind: PermissionKind
  public var detail: String?
}

public struct PermissionBlock: Codable, Hashable, Sendable {
  public var id: String
  public var title: String
  public var command: String?
  public var description: String?
  public var planId: String?
  public var defaultToNo: Bool?
  public var options: [PermissionOption]
}

public struct PlanDocumentBlock: Codable, Hashable, Sendable {
  public var id: String
  public var title: String
  public var markdown: String
  public var path: String?
  public var toolCallId: String
  public var approvalToolCallId: String?
  /// draft | ready | approved | rejected | executing
  public var status: String
}

public struct CompactionBlock: Codable, Hashable, Sendable {
  public var id: String
  public var status: CompactionStatus
  public var error: String?
}

public struct QuestionOption: Codable, Hashable, Sendable, Identifiable {
  public var id: String
  public var label: String
  public var description: String?
}

public struct Question: Codable, Hashable, Sendable, Identifiable {
  public var id: String
  public var title: String?
  public var text: String
  public var kind: QuestionKind
  public var options: [QuestionOption]
  public var other: Bool?
  public var numeric: Bool?
  public var required: Bool?
}

/// One answer: a single string or a list of option ids
public enum QuestionAnswer: Codable, Hashable, Sendable {
  case one(String)
  case many([String])
  public init(from decoder: Decoder) throws {
    let c = try decoder.singleValueContainer()
    if let s = try? c.decode(String.self) { self = .one(s) } else { self = .many(try c.decode([String].self)) }
  }
  public func encode(to encoder: Encoder) throws {
    var c = encoder.singleValueContainer()
    switch self { case .one(let s): try c.encode(s); case .many(let a): try c.encode(a) }
  }
}

public struct QuestionBlock: Codable, Hashable, Sendable {
  public var id: String
  public var toolCallId: String?
  public var message: String?
  public var questions: [Question]
  public var outcome: QuestionOutcome?
  public var answers: [String: QuestionAnswer]?
}

public struct ImageBlock: Codable, Hashable, Sendable {
  public var id: String
  public var blob: String?
  public var mimeType: String
  public var uri: String?
}

public struct NoticeBlock: Codable, Hashable, Sendable {
  public var id: String
  public var revision: Int?
  public var category: String?
  public var severity: String
  /// An informational ACP notice, stored as a warning plus this flag
  public var info: Bool?
  public var title: String
  public var details: String?
  public var actions: [FailureAction]?
}

public struct SteerBlock: Codable, Hashable, Sendable {
  public var id: String
  public var text: String
  public var attachments: [Attachment]?
}

/// A goal milestone: the dock shows the present goal, these rows keep its history. Counters are the snapshot's at that
/// moment
public struct GoalBlock: Codable, Hashable, Sendable {
  public var event: GoalEvent
  public var objective: String?
  public var iterations: Int?
  public var tokensUsed: Int?
  public var timeUsedSeconds: Double?

  public init(event: GoalEvent, objective: String? = nil, iterations: Int? = nil, tokensUsed: Int? = nil, timeUsedSeconds: Double? = nil) {
    self.event = event; self.objective = objective; self.iterations = iterations; self.tokensUsed = tokensUsed
    self.timeUsedSeconds = timeUsedSeconds
  }
}

/// One block of an agent turn, tagged by `type`
public enum AgentBlock: Codable, Hashable, Sendable {
  case thought(ThoughtBlock)
  case plan(PlanBlock)
  case toolCall(ToolCallBlock)
  case text(TextBlock)
  case permission(PermissionBlock)
  case compaction(CompactionBlock)
  case planDocument(PlanDocumentBlock)
  case question(QuestionBlock)
  case image(ImageBlock)
  case notice(NoticeBlock)
  case steer(SteerBlock)
  case goal(GoalBlock)
  /// A block type this build does not know; carries the type string for diagnostics
  case unknown(String)

  /// The `type` literals this build mirrors
  public static let types = [
    "thought", "plan", "tool_call", "text", "permission", "compaction", "plan_document", "question", "image", "notice",
    "steer", "goal",
  ]

  private enum Keys: String, CodingKey { case type }
  public init(from decoder: Decoder) throws {
    let type = try decoder.container(keyedBy: Keys.self).decode(String.self, forKey: .type)
    switch type {
    case "thought": self = .thought(try ThoughtBlock(from: decoder))
    case "plan": self = .plan(try PlanBlock(from: decoder))
    case "tool_call": self = .toolCall(try ToolCallBlock(from: decoder))
    case "text": self = .text(try TextBlock(from: decoder))
    case "permission": self = .permission(try PermissionBlock(from: decoder))
    case "compaction": self = .compaction(try CompactionBlock(from: decoder))
    case "plan_document": self = .planDocument(try PlanDocumentBlock(from: decoder))
    case "question": self = .question(try QuestionBlock(from: decoder))
    case "image": self = .image(try ImageBlock(from: decoder))
    case "notice": self = .notice(try NoticeBlock(from: decoder))
    case "steer": self = .steer(try SteerBlock(from: decoder))
    case "goal": self = .goal(try GoalBlock(from: decoder))
    default: self = .unknown(type)
    }
  }
  public func encode(to encoder: Encoder) throws {
    var c = encoder.container(keyedBy: Keys.self)
    try c.encode(typeName, forKey: .type)
    switch self {
    case .thought(let b): try b.encode(to: encoder)
    case .plan(let b): try b.encode(to: encoder)
    case .toolCall(let b): try b.encode(to: encoder)
    case .text(let b): try b.encode(to: encoder)
    case .permission(let b): try b.encode(to: encoder)
    case .compaction(let b): try b.encode(to: encoder)
    case .planDocument(let b): try b.encode(to: encoder)
    case .question(let b): try b.encode(to: encoder)
    case .image(let b): try b.encode(to: encoder)
    case .notice(let b): try b.encode(to: encoder)
    case .steer(let b): try b.encode(to: encoder)
    case .goal(let b): try b.encode(to: encoder)
    case .unknown: break
    }
  }

  /// The wire `type` string
  public var typeName: String {
    switch self {
    case .thought: "thought"
    case .plan: "plan"
    case .toolCall: "tool_call"
    case .text: "text"
    case .permission: "permission"
    case .compaction: "compaction"
    case .planDocument: "plan_document"
    case .question: "question"
    case .image: "image"
    case .notice: "notice"
    case .steer: "steer"
    case .goal: "goal"
    case .unknown(let t): t
    }
  }

  /// The block's own id when the contract gives it one
  public var blockId: String? {
    switch self {
    case .toolCall(let b): b.id
    case .text(let b): b.id
    case .permission(let b): b.id
    case .compaction(let b): b.id
    case .planDocument(let b): b.id
    case .question(let b): b.id
    case .image(let b): b.id
    case .notice(let b): b.id
    case .steer(let b): b.id
    case .thought, .plan, .goal, .unknown: nil
    }
  }

  public var toolCall: ToolCallBlock? { if case .toolCall(let b) = self { b } else { nil } }
}

// MARK: - Turns

/// An attachment as a sent turn keeps it: images, dropped text and selections live in the session's blob directory
/// (`blob` absent when the write failed), files are paths the agent reads itself
public enum Attachment: Codable, Hashable, Sendable {
  case image(blob: String?, mimeType: String, name: String?)
  /// context: where a shared text came from (absent on dropped text)
  case text(blob: String?, name: String, context: TextContext? = nil)
  case file(uri: String, name: String)
  /// blob: the selected text; lines are 1-based and inclusive
  case selection(uri: String, name: String, startLine: Int, endLine: Int, blob: String? = nil)
  case quote(text: String, comment: String?)
  case unknown

  private enum Keys: String, CodingKey { case kind, blob, mimeType, name, context, uri, startLine, endLine, text, comment }
  public init(from decoder: Decoder) throws {
    let c = try decoder.container(keyedBy: Keys.self)
    switch try c.decode(String.self, forKey: .kind) {
    case "image": self = .image(blob: try c.decodeIfPresent(String.self, forKey: .blob), mimeType: try c.decode(String.self, forKey: .mimeType), name: try c.decodeIfPresent(String.self, forKey: .name))
    case "text": self = .text(blob: try c.decodeIfPresent(String.self, forKey: .blob), name: try c.decode(String.self, forKey: .name), context: try c.decodeIfPresent(TextContext.self, forKey: .context))
    case "file": self = .file(uri: try c.decode(String.self, forKey: .uri), name: try c.decode(String.self, forKey: .name))
    case "selection": self = .selection(uri: try c.decode(String.self, forKey: .uri), name: try c.decode(String.self, forKey: .name), startLine: try c.decode(Int.self, forKey: .startLine), endLine: try c.decode(Int.self, forKey: .endLine), blob: try c.decodeIfPresent(String.self, forKey: .blob))
    case "quote": self = .quote(text: try c.decode(String.self, forKey: .text), comment: try c.decodeIfPresent(String.self, forKey: .comment))
    default: self = .unknown
    }
  }
  public func encode(to encoder: Encoder) throws {
    var c = encoder.container(keyedBy: Keys.self)
    switch self {
    case let .image(blob, mime, name): try c.encode("image", forKey: .kind); try c.encodeIfPresent(blob, forKey: .blob); try c.encode(mime, forKey: .mimeType); try c.encodeIfPresent(name, forKey: .name)
    case let .text(blob, name, context): try c.encode("text", forKey: .kind); try c.encodeIfPresent(blob, forKey: .blob); try c.encode(name, forKey: .name); try c.encodeIfPresent(context, forKey: .context)
    case let .file(uri, name): try c.encode("file", forKey: .kind); try c.encode(uri, forKey: .uri); try c.encode(name, forKey: .name)
    case let .selection(uri, name, s, e, blob): try c.encode("selection", forKey: .kind); try c.encode(uri, forKey: .uri); try c.encode(name, forKey: .name); try c.encode(s, forKey: .startLine); try c.encode(e, forKey: .endLine); try c.encodeIfPresent(blob, forKey: .blob)
    case let .quote(text, comment): try c.encode("quote", forKey: .kind); try c.encode(text, forKey: .text); try c.encodeIfPresent(comment, forKey: .comment)
    case .unknown: try c.encode("unknown", forKey: .kind)
    }
  }
}

/// The ACP selections a prompt was sent with: the mode and each config option's value
public struct TurnSettings: Codable, Hashable, Sendable {
  public var modeId: String?
  public var config: [String: String]

  public init(modeId: String? = nil, config: [String: String] = [:]) { self.modeId = modeId; self.config = config }
  public init(from decoder: Decoder) throws {
    let c = try decoder.container(keyedBy: CodingKeys.self)
    modeId = try c.decodeIfPresent(String.self, forKey: .modeId)
    config = try c.decode(.config, or: [:])
  }
}

public struct UserTurn: Codable, Hashable, Sendable {
  public var id: String?
  public var text: String
  public var command: String?
  public var attachments: [Attachment]?
  /// The selections the prompt was sent with; older records omit it
  public var settings: TurnSettings?
  /// Retrying a failed edited turn rebuilds its preceding context as well
  public var edited: Bool?
  public var planId: String?
  /// Sent by Acpira itself (compaction, the continue after an account switch or a Retry …); never a bubble
  public var auto: Bool?
  /// compact | accountSwitch | retry | gate | goal | subagents; absent on an auto turn: the over-threshold /compact
  public var autoReason: String?
  /// With autoReason `goal`: the goal's next turn, sent because the goal was still active when the turn before ended
  public var goalContinue: Bool?

  public init(text: String, attachments: [Attachment]? = nil) {
    self.text = text; self.attachments = attachments
  }
}

public struct TurnError: Codable, Hashable, Sendable {
  public var message: String
  public var code: Int?
  public var kind: String?
  public var retryable: Bool?
  /// An AIR sessionFailure: its id (the turn's NoticeBlock with that id is not drawn again) and the actions it declared
  public var failureId: String?
  public var actions: [FailureAction]?
}

public struct TurnUsage: Codable, Hashable, Sendable {
  public struct Context: Codable, Hashable, Sendable { public var used: Int; public var size: Int }
  public var input: Int?
  public var output: Int?
  public var cachedRead: Int?
  public var cachedWrite: Int?
  public var reasoning: Int?
  public var total: Int?
  public var modelCalls: Int?
  public var model: String?
  /// The vendor's id for this request
  public var requestId: String?
  public var context: Context?
}

public struct TurnRetry: Codable, Hashable, Sendable {
  public var attempt: Int?
  public var max: Int?
  public var detail: String
}

public struct TurnActivity: Codable, Hashable, Sendable {
  public var kind: ToolKind
  public var label: String
}

/// A slash command's receipt, kept apart from the agent's blocks (a request can finish without prose), with only the
/// setting changes ACP reported
public struct TurnCommand: Codable, Hashable, Sendable {
  public struct Option: Codable, Hashable, Sendable {
    public var name: String
    public var value: String
  }
  public var name: String
  public var mode: String?
  public var options: [Option]?
}

public struct AgentTurn: Codable, Hashable, Sendable {
  /// The turn's execution state can no longer be established
  public var observation: ObservationState?
  public var blocks: [AgentBlock]
  public var usage: TurnUsage?
  public var startedAt: Double?
  /// First streamed output of the prompt; the turn's duration counts from here (absent on older transcripts)
  public var respondedAt: Double?
  public var endedAt: Double?
  public var activity: TurnActivity?
  public var stop: TurnStop?
  public var error: TurnError?
  public var command: TurnCommand?
  public var retry: TurnRetry?
}

public enum Turn: Codable, Hashable, Sendable {
  case user(UserTurn)
  case agent(AgentTurn)

  private enum Keys: String, CodingKey { case role }
  public init(from decoder: Decoder) throws {
    let role = try decoder.container(keyedBy: Keys.self).decode(String.self, forKey: .role)
    self = role == "user" ? .user(try UserTurn(from: decoder)) : .agent(try AgentTurn(from: decoder))
  }
  public func encode(to encoder: Encoder) throws {
    var c = encoder.container(keyedBy: Keys.self)
    switch self {
    case .user(let t): try c.encode("user", forKey: .role); try t.encode(to: encoder)
    case .agent(let t): try c.encode("agent", forKey: .role); try t.encode(to: encoder)
    }
  }
}

// MARK: - Sessions

public struct SessionSummary: Codable, Hashable, Sendable, Identifiable {
  /// The model a session last ran: the wire id is its identity (the vendor is read from it), the name its label
  public struct Model: Codable, Hashable, Sendable {
    public var id: String
    public var name: String
  }
  public var id: String
  public var external: Bool?
  public var title: String
  public var agent: AgentId
  public var accountId: String?
  /// The agent's own session id
  public var acpSessionId: String?
  public var cwd: String
  public var updatedAt: String
  /// When the session started (ISO), for the list's "created" order; absent on older index entries
  public var createdAt: String?
  public var pinned: Bool?
  public var category: String?
  public var state: SessionState?
  /// Only beside `.background`: a turn ended there that nobody has looked at yet
  public var unread: Bool?
  /// Absent while no model control is known and on older index entries
  public var model: Model?
}

public struct Usage: Codable, Hashable, Sendable {
  public var used: Int
  public var size: Int
  public var cost: Double?
  /// A compaction finished and the agent has not reported its context since: `used` still predates it and this is that
  /// figure; the next report drops it
  public var compactedFrom: Int?
}

public struct QueuedPrompt: Codable, Hashable, Sendable, Identifiable {
  public var id: String
  public var text: String
  public var attachments: [Attachment]?
  public var sending: Bool?
}

/// A way to sign in (`login` methodId). `terminal`: the engine runs the agent binary with these args / env in a terminal
/// on its machine instead of calling `authenticate`
public struct AuthMethodInfo: Codable, Hashable, Sendable, Identifiable {
  public struct Terminal: Codable, Hashable, Sendable {
    public var args: [String]
    public var env: [String: String]?
  }
  public var id: String
  public var name: String
  public var description: String?
  public var terminal: Terminal?
}

/// A conversation driven from outside (the ChatGPT bridge) rather than by an agent process
public struct ExternalSessionInfo: Codable, Hashable, Sendable {
  public enum State: String, TolerantEnum, CaseIterable {
    case unbound, receiving, idle, stale, unknown
    public static var unknownCase: Self { .unknown }
  }
  /// `chatgpt`
  public var source: String
  public var sourceKey: String
  public var connectionPrompt: String?
  public var state: State
  public var activeTurnId: String?
  public var lastEventAt: String
}

/// A fork whose copied turns are still in its transcript: the source session, the last copied turn's index, and the
/// source's agent when the fork moved to another one; the thread draws the boundary under that turn
public struct ForkOrigin: Codable, Hashable, Sendable {
  public var sessionId: String
  public var turnIndex: Int
  public var agent: AgentId?
}

/// The agent's goal (AIR goal extension): it keeps working toward `objective` across turns until it judges it met.
/// Counters are what the agent reports: Claude its stop-hook rounds, Codex time and tokens against a budget
public struct SessionGoal: Codable, Hashable, Sendable {
  public var objective: String
  public var status: GoalStatus
  public var iterations: Int?
  public var tokenBudget: Int?
  public var tokensUsed: Int?
  public var timeUsedSeconds: Double?
  /// Epoch ms of the agent's last goal change: timeUsedSeconds counts up to here
  public var updatedAt: Double?
}

// MARK: - Subagents

/// What Acpira observes of a child: `session` its own updates under its own session id, `nested` its tool calls on the
/// parent session with a parent link, `receipt` only the delegation call and its result
public enum SubagentVisibility: String, TolerantEnum, CaseIterable {
  case session, nested, receipt, unknown
  public static var unknownCase: Self { .unknown }
}

public enum SubagentState: String, TolerantEnum, CaseIterable {
  case running, completed, failed, cancelled, disconnected, unknown
  public static var unknownCase: Self { .unknown }
}

/// A summoned child's access: `bypass` works without approvals; `inherit` (a persona setting only) takes the caller's
public enum RelayMode: String, TolerantEnum, CaseIterable {
  case consult, work, bypass, inherit, unknown
  public static var unknownCase: Self { .unknown }
}

/// A child Acpira itself runs in another CLI (a summoned persona, whose name is the node's `role`); every round of one
/// conversation with the same native session is a node of its own
public struct SubagentHarness: Codable, Hashable, Sendable {
  public var agent: AgentId
  public var persona: String?
  public var mode: RelayMode
  public var thread: String
  public var round: Int
  /// The child CLI's own session id
  public var sessionId: String?
  /// The reasoning effort the child session really runs; absent: none reported
  public var effort: String?
  /// The group the round was summoned through
  public var group: String?
}

/// One delegated child node (subagents.ts SubagentSummary), whatever the wire dialect; its transcript comes with the
/// `subagent` push once observed
public struct SubagentSummary: Codable, Hashable, Sendable, Identifiable {
  public enum StateSource: String, TolerantEnum, CaseIterable {
    /// local: Acpira synthesized the state (disconnected on connection end, the parent prompt returned first)
    case agent, local, unknown
    public static var unknownCase: Self { .unknown }
  }
  public struct Controls: Codable, Hashable, Sendable {
    /// Only when the agent said this child can be cancelled
    public var cancel: Bool
    public init(cancel: Bool = false) { self.cancel = cancel }
    public init(from decoder: Decoder) throws {
      cancel = try decoder.container(keyedBy: CodingKeys.self).decode(.cancel, or: false)
    }
  }
  public struct Usage: Codable, Hashable, Sendable {
    public var used: Int
    public var size: Int
  }
  /// The agent's own identifiers, for diagnostics and replay; sidechainId: a Claude workflow agent's own agentId
  public struct Peer: Codable, Hashable, Sendable {
    public var sessionId: String?
    public var agentId: String?
    public var toolCallId: String?
    public var sidechainId: String?
  }
  /// Acpira's own stable id, never a peer id
  public var id: String
  /// Another summary's id; absent: a child of the root session
  public var parentId: String?
  /// The root agent turn (index in SessionView.turns) during which it was announced
  public var turnIndex: Int
  public var visibility: SubagentVisibility
  public var title: String?
  /// The delegated task text
  public var task: String?
  /// The agent's profile / subagent type; absent when not structured
  public var role: String?
  public var state: SubagentState
  public var stateSource: StateSource
  public var controls: Controls
  public var cancelRequested: Bool?
  /// The agent said the child runs detached
  public var background: Bool?
  public var announcedAt: Double
  public var endedAt: Double?
  /// Only when reported
  public var model: String?
  /// Only when a usage update was attributed to this child
  public var usage: Usage?
  public var peer: Peer
  /// The latest activity label, kept engine-side
  public var activity: String?
  public var toolCount: Int
  /// The final result text the parent received (for `session` visibility the child's own last text)
  public var result: String?
  /// A receipt node whose transcript has content of its own (a Claude workflow agent's sidechain log)
  public var hasProcess: Bool?
  /// The child's pending permission cards and open question, mirrored so the root view can show them
  public var permissions: [PermissionBlock]?
  public var question: QuestionBlock?
  /// Set when Acpira itself runs the child in another CLI
  public var harness: SubagentHarness?

  public init(from decoder: Decoder) throws {
    let c = try decoder.container(keyedBy: CodingKeys.self)
    id = try c.decode(String.self, forKey: .id)
    parentId = try c.decodeIfPresent(String.self, forKey: .parentId)
    turnIndex = try c.decode(.turnIndex, or: 0)
    visibility = try c.decode(.visibility, or: .unknown)
    title = try c.decodeIfPresent(String.self, forKey: .title)
    task = try c.decodeIfPresent(String.self, forKey: .task)
    role = try c.decodeIfPresent(String.self, forKey: .role)
    state = try c.decode(.state, or: .unknown)
    stateSource = try c.decode(.stateSource, or: .unknown)
    controls = try c.decodeIfPresent(Controls.self, forKey: .controls) ?? Controls()
    cancelRequested = try c.decodeIfPresent(Bool.self, forKey: .cancelRequested)
    background = try c.decodeIfPresent(Bool.self, forKey: .background)
    announcedAt = try c.decode(.announcedAt, or: 0)
    endedAt = try c.decodeIfPresent(Double.self, forKey: .endedAt)
    model = try c.decodeIfPresent(String.self, forKey: .model)
    usage = try c.decodeIfPresent(Usage.self, forKey: .usage)
    peer = try c.decode(.peer, or: Peer())
    activity = try c.decodeIfPresent(String.self, forKey: .activity)
    toolCount = try c.decode(.toolCount, or: 0)
    result = try c.decodeIfPresent(String.self, forKey: .result)
    hasProcess = try c.decodeIfPresent(Bool.self, forKey: .hasProcess)
    permissions = try c.decodeIfPresent([PermissionBlock].self, forKey: .permissions)
    question = try c.decodeIfPresent(QuestionBlock.self, forKey: .question)
    harness = try c.decodeIfPresent(SubagentHarness.self, forKey: .harness)
  }
}

public struct SessionView: Codable, Hashable, Sendable, Identifiable {
  public var id: String
  public var external: ExternalSessionInfo?
  public var agent: AgentId
  public var accountId: String?
  public var title: String
  public var cwd: String
  public var status: SessionStatus
  public var error: String?
  /// `error` is a session lock an agent of another engine still holds, or this is a read-only copy of a session another
  /// engine has open: takeOverSession ends that agent, or asks that engine to hand the session over
  public var canTakeOver: Bool?
  /// The ways to sign in while `status` is auth_required
  public var authMethods: [AuthMethodInfo]?
  public var turns: [Turn]
  public var running: Bool
  public var rev: Int?
  public var controls: SessionControls
  /// The agent's remembered per-model parameters, for switching the model of an edited prompt
  public var modelShapes: ModelShapes?
  public var usage: Usage?
  public var commands: [SlashCommand]
  public var queued: [QueuedPrompt]?
  public var canSteer: Bool?
  /// The agent's current goal; a completed one stays only until its turn ends
  public var goal: SessionGoal?
  /// The goal controls the running agent advertises; absent without goal support (or no process yet)
  public var goalActions: [GoalAction]?
  /// Delegated children announced during this session
  public var subagents: [SubagentSummary]?
  public var forkedFrom: ForkOrigin?
  public var createdAt: String
  public var updatedAt: String

  public init(from decoder: Decoder) throws {
    let c = try decoder.container(keyedBy: CodingKeys.self)
    id = try c.decode(String.self, forKey: .id)
    external = try c.decodeIfPresent(ExternalSessionInfo.self, forKey: .external)
    agent = try c.decode(String.self, forKey: .agent)
    accountId = try c.decodeIfPresent(String.self, forKey: .accountId)
    title = try c.decodeIfPresent(String.self, forKey: .title) ?? ""
    cwd = try c.decodeIfPresent(String.self, forKey: .cwd) ?? ""
    status = try c.decodeIfPresent(SessionStatus.self, forKey: .status) ?? .ready
    error = try c.decodeIfPresent(String.self, forKey: .error)
    canTakeOver = try c.decodeIfPresent(Bool.self, forKey: .canTakeOver)
    authMethods = try c.decodeIfPresent([AuthMethodInfo].self, forKey: .authMethods)
    turns = try c.decodeIfPresent([Turn].self, forKey: .turns) ?? []
    running = try c.decodeIfPresent(Bool.self, forKey: .running) ?? false
    rev = try c.decodeIfPresent(Int.self, forKey: .rev)
    controls = try c.decodeIfPresent(SessionControls.self, forKey: .controls) ?? SessionControls()
    modelShapes = try c.decodeIfPresent(ModelShapes.self, forKey: .modelShapes)
    usage = try c.decodeIfPresent(Usage.self, forKey: .usage)
    commands = try c.decodeIfPresent([SlashCommand].self, forKey: .commands) ?? []
    queued = try c.decodeIfPresent([QueuedPrompt].self, forKey: .queued)
    canSteer = try c.decodeIfPresent(Bool.self, forKey: .canSteer)
    goal = try c.decodeIfPresent(SessionGoal.self, forKey: .goal)
    goalActions = try c.decodeIfPresent([GoalAction].self, forKey: .goalActions)
    subagents = try c.decodeIfPresent([SubagentSummary].self, forKey: .subagents)
    forkedFrom = try c.decodeIfPresent(ForkOrigin.self, forKey: .forkedFrom)
    createdAt = try c.decodeIfPresent(String.self, forKey: .createdAt) ?? ""
    updatedAt = try c.decodeIfPresent(String.self, forKey: .updatedAt) ?? ""
  }
}

// MARK: - Timestamps

extension Date {
  /// `2026-10-09T09:05:18.000Z` (JavaScript's toISOString) or the same without fractional seconds
  public init?(iso string: String) {
    let fractional = Date.ISO8601FormatStyle(includingFractionalSeconds: true)
    guard let date = (try? fractional.parse(string)) ?? (try? Date.ISO8601FormatStyle().parse(string)) else { return nil }
    self = date
  }
}
