import Foundation

// Mirror of packages/shared/usage.ts and rust/crates/acpira-shared/src/usage.rs: hour-bucketed usage across every
// session, folded on the client into local days, summary tiles, trend columns and model / agent / project breakdowns.

/// A session the usage rows index into; `cwd` is its project folder
public struct UsageSession: Codable, Hashable, Sendable {
  public var agent: AgentId
  public var cwd: String

  public init(agent: AgentId, cwd: String) {
    self.agent = agent
    self.cwd = cwd
  }
}

/// A model the usage rows index into: the wire id and the family name shown in pickers
public struct UsageModel: Codable, Hashable, Sendable {
  public var id: String
  public var name: String

  public init(id: String, name: String) {
    self.id = id
    self.name = name
  }
}

/// One session's turns in one model within one UTC clock hour; zero fields are omitted on the wire
public struct UsageRow: Codable, Hashable, Sendable {
  /// The hour's start, Unix milliseconds (whole UTC hours)
  public var at: Int64
  /// Index into `UsageView.sessions`
  public var session: Int
  /// Index into `UsageView.models`; absent when nothing named the model
  public var model: Int?
  public var tokens: Int64?
  public var input: Int64?
  public var output: Int64?
  public var cachedRead: Int64?
  public var turns: Int?
  public var prompts: Int?
  public var workMs: Int64?
  public var longestMs: Int64?
  public var tools: Int?
  public var added: Int?
  public var removed: Int?

  public init(at: Int64, session: Int, model: Int? = nil, tokens: Int64? = nil, input: Int64? = nil, output: Int64? = nil,
              cachedRead: Int64? = nil, turns: Int? = nil, prompts: Int? = nil, workMs: Int64? = nil, longestMs: Int64? = nil,
              tools: Int? = nil, added: Int? = nil, removed: Int? = nil) {
    self.at = at
    self.session = session
    self.model = model
    self.tokens = tokens
    self.input = input
    self.output = output
    self.cachedRead = cachedRead
    self.turns = turns
    self.prompts = prompts
    self.workMs = workMs
    self.longestMs = longestMs
    self.tools = tools
    self.added = added
    self.removed = removed
  }
}

/// Usage across every session on the engine (`HostMsg.usage`)
public struct UsageView: Codable, Hashable, Sendable {
  public var sessions: [UsageSession]
  public var models: [UsageModel]
  /// Ordered by `at`
  public var rows: [UsageRow]

  public init(sessions: [UsageSession] = [], models: [UsageModel] = [], rows: [UsageRow] = []) {
    self.sessions = sessions
    self.models = models
    self.rows = rows
  }
}
