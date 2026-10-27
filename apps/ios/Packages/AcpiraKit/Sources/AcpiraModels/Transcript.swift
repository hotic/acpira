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
  public var install: AgentInstall?
  public init(id: AgentId, name: String) { self.id = id; self.name = name }
}

public struct AccountInfo: Codable, Hashable, Sendable, Identifiable {
  public var id: String
  public var agent: AgentId
  public var label: String
  public var detail: String?
  public var addedAt: String?
  public var lastUsedAt: String?
  public var quota: AccountQuota?
}

// MARK: - Controls

public struct SessionOption: Codable, Hashable, Sendable, Identifiable {
  public struct Group: Codable, Hashable, Sendable { public var id: String; public var name: String }
  /// A provider verified by the agent adapter (official / custom), never inferred from a display name
  public struct Source: Codable, Hashable, Sendable { public var id: String; public var name: String; public var kind: String }
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
}

public enum ToolContent: Codable, Hashable, Sendable {
  case text(String)
  case diff([DiffLine])
  case list([String])
  case image(ImageRef)
  case unknown

  private enum Keys: String, CodingKey { case type, text, lines, items, blob, mimeType, uri }
  public init(from decoder: Decoder) throws {
    let c = try decoder.container(keyedBy: Keys.self)
    switch try c.decode(String.self, forKey: .type) {
    case "text": self = .text(try c.decode(String.self, forKey: .text))
    case "diff": self = .diff(try c.decode([DiffLine].self, forKey: .lines))
    case "list": self = .list(try c.decode([String].self, forKey: .items))
    case "image": self = .image(try ImageRef(from: decoder))
    default: self = .unknown
    }
  }
  public func encode(to encoder: Encoder) throws {
    var c = encoder.container(keyedBy: Keys.self)
    switch self {
    case .text(let t): try c.encode("text", forKey: .type); try c.encode(t, forKey: .text)
    case .diff(let l): try c.encode("diff", forKey: .type); try c.encode(l, forKey: .lines)
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

public struct AsyncTaskInfo: Codable, Hashable, Sendable {
  public var id: String
  public var state: String
  public var canStop: Bool?
  public var name: String?
  public var description: String?
  public var summary: String?
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
  public var id: String
  public var kind: ToolKind
  public var verb: String
  public var verbKey: String?
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
  public var title: String
  public var details: String?
  public var actions: [FailureAction]?
}

public struct SteerBlock: Codable, Hashable, Sendable {
  public var id: String
  public var text: String
  public var attachments: [Attachment]?
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
  /// A block type this build does not know; carries the type string for diagnostics
  case unknown(String)

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
    case .thought, .plan, .unknown: nil
    }
  }

  public var toolCall: ToolCallBlock? { if case .toolCall(let b) = self { b } else { nil } }
}

// MARK: - Turns

public enum Attachment: Codable, Hashable, Sendable {
  case image(blob: String?, mimeType: String, name: String?)
  case text(blob: String?, name: String)
  case file(uri: String, name: String)
  case selection(uri: String, name: String, startLine: Int, endLine: Int)
  case quote(text: String, comment: String?)
  case unknown

  private enum Keys: String, CodingKey { case kind, blob, mimeType, name, uri, startLine, endLine, text, comment }
  public init(from decoder: Decoder) throws {
    let c = try decoder.container(keyedBy: Keys.self)
    switch try c.decode(String.self, forKey: .kind) {
    case "image": self = .image(blob: try c.decodeIfPresent(String.self, forKey: .blob), mimeType: try c.decode(String.self, forKey: .mimeType), name: try c.decodeIfPresent(String.self, forKey: .name))
    case "text": self = .text(blob: try c.decodeIfPresent(String.self, forKey: .blob), name: try c.decode(String.self, forKey: .name))
    case "file": self = .file(uri: try c.decode(String.self, forKey: .uri), name: try c.decode(String.self, forKey: .name))
    case "selection": self = .selection(uri: try c.decode(String.self, forKey: .uri), name: try c.decode(String.self, forKey: .name), startLine: try c.decode(Int.self, forKey: .startLine), endLine: try c.decode(Int.self, forKey: .endLine))
    case "quote": self = .quote(text: try c.decode(String.self, forKey: .text), comment: try c.decodeIfPresent(String.self, forKey: .comment))
    default: self = .unknown
    }
  }
  public func encode(to encoder: Encoder) throws {
    var c = encoder.container(keyedBy: Keys.self)
    switch self {
    case let .image(blob, mime, name): try c.encode("image", forKey: .kind); try c.encodeIfPresent(blob, forKey: .blob); try c.encode(mime, forKey: .mimeType); try c.encodeIfPresent(name, forKey: .name)
    case let .text(blob, name): try c.encode("text", forKey: .kind); try c.encodeIfPresent(blob, forKey: .blob); try c.encode(name, forKey: .name)
    case let .file(uri, name): try c.encode("file", forKey: .kind); try c.encode(uri, forKey: .uri); try c.encode(name, forKey: .name)
    case let .selection(uri, name, s, e): try c.encode("selection", forKey: .kind); try c.encode(uri, forKey: .uri); try c.encode(name, forKey: .name); try c.encode(s, forKey: .startLine); try c.encode(e, forKey: .endLine)
    case let .quote(text, comment): try c.encode("quote", forKey: .kind); try c.encode(text, forKey: .text); try c.encodeIfPresent(comment, forKey: .comment)
    case .unknown: try c.encode("unknown", forKey: .kind)
    }
  }
}

public struct UserTurn: Codable, Hashable, Sendable {
  public var id: String?
  public var text: String
  public var command: String?
  public var attachments: [Attachment]?
  public var edited: Bool?
  public var planId: String?
  public var auto: Bool?
  public var autoReason: String?

  public init(text: String, attachments: [Attachment]? = nil) {
    self.text = text; self.attachments = attachments
  }
}

public struct TurnError: Codable, Hashable, Sendable {
  public var message: String
  public var code: Int?
  public var kind: String?
  public var retryable: Bool?
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

public struct AgentTurn: Codable, Hashable, Sendable {
  public var blocks: [AgentBlock]
  public var usage: TurnUsage?
  public var startedAt: Double?
  /// First streamed output of the prompt; the turn's duration counts from here (absent on older transcripts)
  public var respondedAt: Double?
  public var endedAt: Double?
  public var activity: TurnActivity?
  public var stop: TurnStop?
  public var error: TurnError?
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
  public var id: String
  public var external: Bool?
  public var title: String
  public var agent: AgentId
  public var accountId: String?
  public var cwd: String
  public var updatedAt: String
  public var pinned: Bool?
  public var category: String?
  public var state: SessionState?
  /// Only beside `.background`: a turn ended there that nobody has looked at yet
  public var unread: Bool?
}

public struct Usage: Codable, Hashable, Sendable {
  public var used: Int
  public var size: Int
  public var cost: Double?
}

public struct QueuedPrompt: Codable, Hashable, Sendable, Identifiable {
  public var id: String
  public var text: String
  public var attachments: [Attachment]?
  public var sending: Bool?
}

public struct SessionView: Codable, Hashable, Sendable, Identifiable {
  public var id: String
  public var agent: AgentId
  public var accountId: String?
  public var title: String
  public var cwd: String
  public var status: SessionStatus
  public var error: String?
  public var turns: [Turn]
  public var running: Bool
  public var rev: Int?
  public var controls: SessionControls
  public var usage: Usage?
  public var commands: [SlashCommand]
  public var queued: [QueuedPrompt]?
  public var canSteer: Bool?
  public var createdAt: String
  public var updatedAt: String

  public init(from decoder: Decoder) throws {
    let c = try decoder.container(keyedBy: CodingKeys.self)
    id = try c.decode(String.self, forKey: .id)
    agent = try c.decode(String.self, forKey: .agent)
    accountId = try c.decodeIfPresent(String.self, forKey: .accountId)
    title = try c.decodeIfPresent(String.self, forKey: .title) ?? ""
    cwd = try c.decodeIfPresent(String.self, forKey: .cwd) ?? ""
    status = try c.decodeIfPresent(SessionStatus.self, forKey: .status) ?? .ready
    error = try c.decodeIfPresent(String.self, forKey: .error)
    turns = try c.decodeIfPresent([Turn].self, forKey: .turns) ?? []
    running = try c.decodeIfPresent(Bool.self, forKey: .running) ?? false
    rev = try c.decodeIfPresent(Int.self, forKey: .rev)
    controls = try c.decodeIfPresent(SessionControls.self, forKey: .controls) ?? SessionControls()
    usage = try c.decodeIfPresent(Usage.self, forKey: .usage)
    commands = try c.decodeIfPresent([SlashCommand].self, forKey: .commands) ?? []
    queued = try c.decodeIfPresent([QueuedPrompt].self, forKey: .queued)
    canSteer = try c.decodeIfPresent(Bool.self, forKey: .canSteer)
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
