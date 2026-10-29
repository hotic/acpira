import Foundation

// The host ↔ page messages the app exchanges (packages/shared/protocol.ts, Rust acpira-shared protocol.rs), as the
// subset the phone needs: WebviewMsg actions it sends and HostMsg pushes it renders. Case names and associated value
// labels are the TypeScript `type` literals and field names (HostMsg `init` is `initState`, `init` being Swift's);
// optional TypeScript fields are optionals, encoded only when set, as JSON.stringify drops `undefined`.

// MARK: - Payload types

/// Agent → option id → the option values hidden from its pickers (settings.ts HiddenMap)
public typealias HiddenMap = [AgentId: [String: [String]]]

/// A model option id → the dependent controls (effort, Fast …) that model came with (modelShapes.ts)
public typealias ModelShapes = [String: [ConfigControl]]

/// The appearance axes (appearance.ts Appearance): axis → value, every value a string (`density: "airy"`,
/// `motion: "none"`). Kept as a map so an axis added on the desktop never fails the decode
public typealias Appearance = [String: String]

/// One answer per question id: an option id or free text, or the option ids of a multi-select
public typealias QuestionAnswers = [String: QuestionAnswer]

/// Where a shared text came from (another tool's selection, a picked web page)
public struct TextContext: Codable, Hashable, Sendable {
  public var sourceId: String
  public var sourceName: String
  public var uri: String?
  public var comment: String?

  public init(sourceId: String, sourceName: String, uri: String? = nil, comment: String? = nil) {
    self.sourceId = sourceId; self.sourceName = sourceName; self.uri = uri; self.comment = comment
  }
}

/// What the composer attaches to a prompt before the engine has seen it (transcript.ts Draft): images and dropped text
/// carry their payload, a file its URI and, for bytes only the client holds, `data` as base64 that the engine stages
public enum Draft: Codable, Hashable, Sendable {
  /// data: base64 bytes
  case image(mimeType: String, data: String, name: String? = nil)
  case text(name: String, text: String, context: TextContext? = nil)
  /// data: base64 bytes of a file only the client holds; the engine stages it and links the staged path
  case file(uri: String, name: String, data: String? = nil)
  /// Lines are 1-based and inclusive
  case selection(uri: String, name: String, startLine: Int, endLine: Int, text: String)
  case quote(text: String, comment: String? = nil)
  /// A kind this build does not know (decoding only)
  case unknown(String)

  /// The `kind` literals this build mirrors
  public static let kinds = ["image", "text", "file", "selection", "quote"]

  private enum Keys: String, CodingKey { case kind, mimeType, data, name, text, context, uri, startLine, endLine, comment }
  public init(from decoder: Decoder) throws {
    let c = try decoder.container(keyedBy: Keys.self)
    switch try c.decode(String.self, forKey: .kind) {
    case "image":
      self = .image(mimeType: try c.decode(String.self, forKey: .mimeType), data: try c.decode(String.self, forKey: .data),
                    name: try c.decodeIfPresent(String.self, forKey: .name))
    case "text":
      self = .text(name: try c.decode(String.self, forKey: .name), text: try c.decode(String.self, forKey: .text),
                   context: try c.decodeIfPresent(TextContext.self, forKey: .context))
    case "file":
      self = .file(uri: try c.decode(String.self, forKey: .uri), name: try c.decode(String.self, forKey: .name),
                   data: try c.decodeIfPresent(String.self, forKey: .data))
    case "selection":
      self = .selection(uri: try c.decode(String.self, forKey: .uri), name: try c.decode(String.self, forKey: .name),
                        startLine: try c.decode(Int.self, forKey: .startLine), endLine: try c.decode(Int.self, forKey: .endLine),
                        text: try c.decode(String.self, forKey: .text))
    case "quote":
      self = .quote(text: try c.decode(String.self, forKey: .text), comment: try c.decodeIfPresent(String.self, forKey: .comment))
    case let kind: self = .unknown(kind)
    }
  }
  public func encode(to encoder: Encoder) throws {
    var c = encoder.container(keyedBy: Keys.self)
    switch self {
    case let .image(mimeType, data, name):
      try c.encode("image", forKey: .kind)
      try c.encode(mimeType, forKey: .mimeType)
      try c.encode(data, forKey: .data)
      try c.encodeIfPresent(name, forKey: .name)
    case let .text(name, text, context):
      try c.encode("text", forKey: .kind)
      try c.encode(name, forKey: .name)
      try c.encode(text, forKey: .text)
      try c.encodeIfPresent(context, forKey: .context)
    case let .file(uri, name, data):
      try c.encode("file", forKey: .kind)
      try c.encode(uri, forKey: .uri)
      try c.encode(name, forKey: .name)
      try c.encodeIfPresent(data, forKey: .data)
    case let .selection(uri, name, startLine, endLine, text):
      try c.encode("selection", forKey: .kind)
      try c.encode(uri, forKey: .uri)
      try c.encode(name, forKey: .name)
      try c.encode(startLine, forKey: .startLine)
      try c.encode(endLine, forKey: .endLine)
      try c.encode(text, forKey: .text)
    case let .quote(text, comment):
      try c.encode("quote", forKey: .kind)
      try c.encode(text, forKey: .text)
      try c.encodeIfPresent(comment, forKey: .comment)
    case .unknown(let kind):
      try c.encode(kind, forKey: .kind)
    }
  }
}

/// The model a plan is built with (buildPlan `model`): a model control's id and the option picked
public struct ModelPick: Codable, Hashable, Sendable {
  public var configId: String
  public var value: String

  public init(configId: String, value: String) { self.configId = configId; self.value = value }
}

/// A user-made group of sessions inside one project
public struct SessionCategory: Codable, Hashable, Sendable, Identifiable {
  public var id: String
  public var name: String
  public var cwd: String
  public var collapsed: Bool?
}

/// A user-made top-level section of a project shell's list, holding whole projects
public struct SessionSection: Codable, Hashable, Sendable, Identifiable {
  public var id: String
  public var name: String
  /// Project folders in list order
  public var projects: [String]
  public var collapsed: Bool?
}

/// Every category in list order plus the project list's own state; shared by every window on the machine
public struct SessionCategories: Codable, Hashable, Sendable {
  public var categories: [SessionCategory]
  public var collapsedProjects: [String]
  /// Project folders in the order a project shell lists them; absent until a project shell first connects
  public var projects: [String]?
  /// Folders taken off a project shell's list (their sessions stay)
  public var removedProjects: [String]?
  public var sections: [SessionSection]?

  public init(from decoder: Decoder) throws {
    let c = try decoder.container(keyedBy: CodingKeys.self)
    categories = try c.decode(.categories, or: [])
    collapsedProjects = try c.decode(.collapsedProjects, or: [])
    projects = try c.decodeIfPresent([String].self, forKey: .projects)
    removedProjects = try c.decodeIfPresent([String].self, forKey: .removedProjects)
    sections = try c.decodeIfPresent([SessionSection].self, forKey: .sections)
  }
}

/// How an account action comes in (AccountActionVia): import the CLI's own login, log in a new one in a terminal,
/// `auto` (import if not yet, else log in), or unlock the credential store
public enum AccountActionVia: String, TolerantEnum {
  case `import`, login, auto, unlock, unknown
  public static var unknownCase: Self { .unknown }
}

public enum AccountActionStatus: String, TolerantEnum {
  case pending, success, missing, cancelled, error, unknown
  public static var unknownCase: Self { .unknown }
}

/// An account import / login / unlock the engine runs, kept engine-side so every page shows the same progress
public struct AccountAction: Codable, Hashable, Sendable {
  public var agent: AgentId
  public var via: AccountActionVia
  public var status: AccountActionStatus
  public var error: String?
}

/// A session whose saved conversation holds every term of a history search, with the text around the first match
public struct SessionHit: Codable, Hashable, Sendable, Identifiable {
  public var id: String
  public var snippet: String
}

/// How an account comes in (AddAccountVia): import the CLI's own login, log in a new one in a terminal on the engine's
/// machine, or `auto` (import if not yet imported, else log in)
public enum AddAccountVia: String, TolerantEnum, CaseIterable {
  case `import`, login, auto, unknown
  public static var unknownCase: Self { .unknown }
}

/// One edit of the category list, applied by the engine to what is on disk so concurrent windows compose (transcript.ts
/// CategoryOp), as the subset the phone sends: the project list's own ops
public enum CategoryOp: Codable, Hashable, Sendable {
  /// Register a folder at the end of the project list (undoing a removal)
  case addProject(cwd: String)
  /// Take a folder off the list; its sessions stay
  case removeProject(cwd: String)
  /// Move a folder before `before`, or to the end
  case reorderProject(cwd: String, before: String? = nil)
  case collapseProject(cwd: String, collapsed: Bool)
  /// An op this build does not know (decoding only)
  case unknown(String)

  /// The `op` literals this build mirrors
  public static let ops = ["addProject", "removeProject", "reorderProject", "collapseProject"]

  /// The wire `op`
  public var op: String {
    switch self {
    case .addProject: "addProject"
    case .removeProject: "removeProject"
    case .reorderProject: "reorderProject"
    case .collapseProject: "collapseProject"
    case .unknown(let op): op
    }
  }

  private enum Keys: String, CodingKey { case op, cwd, before, collapsed }
  public init(from decoder: Decoder) throws {
    let c = try decoder.container(keyedBy: Keys.self)
    func cwd() throws -> String { try c.decode(String.self, forKey: .cwd) }
    switch try c.decode(String.self, forKey: .op) {
    case "addProject": self = .addProject(cwd: try cwd())
    case "removeProject": self = .removeProject(cwd: try cwd())
    case "reorderProject": self = .reorderProject(cwd: try cwd(), before: try c.decodeIfPresent(String.self, forKey: .before))
    case "collapseProject": self = .collapseProject(cwd: try cwd(), collapsed: try c.decode(Bool.self, forKey: .collapsed))
    case let op: self = .unknown(op)
    }
  }
  public func encode(to encoder: Encoder) throws {
    var c = encoder.container(keyedBy: Keys.self)
    try c.encode(op, forKey: .op)
    switch self {
    case .addProject(let cwd), .removeProject(let cwd):
      try c.encode(cwd, forKey: .cwd)
    case let .reorderProject(cwd, before):
      try c.encode(cwd, forKey: .cwd)
      try c.encodeIfPresent(before, forKey: .before)
    case let .collapseProject(cwd, collapsed):
      try c.encode(cwd, forKey: .cwd)
      try c.encode(collapsed, forKey: .collapsed)
    case .unknown:
      break
    }
  }
}

/// A rewrite of a sent prompt (WebviewMsg editTurn): the user turn at `turnIndex` and everything after it give way to
/// `text` with the kept attachments (by index) and new drafts, sent with `settings`. turnCount, originalText and turnId
/// are the view the page edited; the engine refuses the edit as stale once they no longer match. intent `continue`: keep
/// the native session (which still holds the original) instead of rebuilding the context
public struct EditTurnRequest: Codable, Hashable, Sendable {
  public enum Intent: String, TolerantEnum, CaseIterable {
    case replace, `continue`, unknown
    public static var unknownCase: Self { .unknown }
  }
  public var sessionId: String
  public var turnIndex: Int
  public var turnCount: Int
  public var originalText: String
  public var turnId: String?
  public var text: String
  public var retainedAttachments: [Int]
  public var attachments: [Draft]
  public var settings: TurnSettings
  public var intent: Intent?

  public init(sessionId: String, turnIndex: Int, turnCount: Int, originalText: String, turnId: String? = nil, text: String,
              retainedAttachments: [Int] = [], attachments: [Draft] = [], settings: TurnSettings, intent: Intent? = nil) {
    self.sessionId = sessionId; self.turnIndex = turnIndex; self.turnCount = turnCount; self.originalText = originalText
    self.turnId = turnId; self.text = text; self.retainedAttachments = retainedAttachments; self.attachments = attachments
    self.settings = settings; self.intent = intent
  }
}

/// One hit of the @ file search: the file's URI and the workspace-relative path the list shows
public struct FileHit: Codable, Hashable, Sendable {
  public var uri: String
  public var path: String

  public init(uri: String, path: String) { self.uri = uri; self.path = path }
}

// The reads of a session's project (rust/crates/acpira-host/src/project_files.rs). Paths are relative to the project
// folder with `/` separators

/// An entry of a project folder
public struct ProjectEntry: Codable, Hashable, Sendable {
  public var name: String
  public var dir: Bool?
}

/// A changed stretch of a file as it is now: `add` lines from line `at` (1-based) are new, the `del` lines stood just
/// before line `at` (one past the last line for lines removed at the end)
public struct ProjectHunk: Codable, Hashable, Sendable {
  public var at: Int
  public var add: Int
  public var del: [String]?
  public init(at: Int, add: Int, del: [String]? = nil) { self.at = at; self.add = add; self.del = del }
}

/// A file the session changed against what it held before the session's first edit; absolute when outside the project
public struct ProjectChange: Codable, Hashable, Sendable {
  public var path: String
  public var add: Int
  public var del: Int
  public var gone: Bool?
  public init(path: String, add: Int, del: Int, gone: Bool? = nil) { self.path = path; self.add = add; self.del = del; self.gone = gone }
}

/// What a file's hunks are measured against: before the session's first edit of it, or the last commit
public enum ProjectBase: String, TolerantEnum, CaseIterable {
  case session, head, unknown
  public static var unknownCase: Self { .unknown }
}

/// A path's git mark; a folder as a whole ends in `/`
public enum GitMark: String, TolerantEnum, CaseIterable {
  case modified = "M", added = "A", deleted = "D", untracked = "U", ignored = "I", unknown
  public static var unknownCase: Self { .unknown }
}

/// Why a project listing or read failed (`ProjectError | string`): a known reason, or the system's own message
public enum ProjectError: Codable, Hashable, Sendable {
  case noProject, outside, missing, binary, tooLarge
  /// Anything else: the system's own message
  case message(String)

  /// The reasons this build knows (ProjectError's literals)
  public static let reasons: [ProjectError] = [.noProject, .outside, .missing, .binary, .tooLarge]

  /// The wire string
  public var rawValue: String {
    switch self {
    case .noProject: "noProject"
    case .outside: "outside"
    case .missing: "missing"
    case .binary: "binary"
    case .tooLarge: "tooLarge"
    case .message(let text): text
    }
  }

  public init(rawValue: String) {
    self = Self.reasons.first { $0.rawValue == rawValue } ?? .message(rawValue)
  }
  public init(from decoder: Decoder) throws {
    self.init(rawValue: try decoder.singleValueContainer().decode(String.self))
  }
  public func encode(to encoder: Encoder) throws {
    var c = encoder.singleValueContainer()
    try c.encode(rawValue)
  }
}

/// How a terminal's shell ended: its exit code, or the signal that ended it
public struct TerminalExit: Codable, Hashable, Sendable {
  public var code: Int
  public var signal: String?

  public init(code: Int, signal: String? = nil) { self.code = code; self.signal = signal }
}

/// A cross-harness subagent from the settings (subagents.ts SubagentPersona): any session summons it through
/// `ask_agent`, or the user names it with `@name` in the composer
public struct SubagentPersona: Codable, Hashable, Sendable, Identifiable {
  /// Stable slug: the tool's `agent` value
  public var id: String
  public var name: String
  /// The CLI it runs in
  public var agent: AgentId
  /// A value of that agent's model select; absent: the CLI's default
  public var model: String?
  /// A value of its reasoning-effort select; absent: the default
  public var effort: String?
  public var mode: RelayMode
  /// Tells the model when to summon it
  public var when: String
  public var brief: String?
  public var enabled: Bool
  /// general | scout | executor | reviewer; absent: general
  public var duty: String?
  /// Every new conversation starts with the handoff pack of the session's changes
  public var changes: Bool?

  public init(id: String, name: String, agent: AgentId, model: String? = nil, effort: String? = nil, mode: RelayMode = .consult,
              when: String = "", enabled: Bool = true) {
    self.id = id; self.name = name; self.agent = agent; self.model = model; self.effort = effort; self.mode = mode
    self.when = when; self.enabled = enabled
  }
}

/// Personas summoned together by one `@name` (subagents.ts SubagentGroup)
public struct SubagentGroup: Codable, Hashable, Sendable, Identifiable {
  public var id: String
  public var name: String
  /// Persona ids, in the order their rounds start
  public var members: [String]
  public var when: String
  public var enabled: Bool

  public init(id: String, name: String, members: [String], when: String = "", enabled: Bool = true) {
    self.id = id; self.name = name; self.members = members; self.when = when; self.enabled = enabled
  }
}

/// The settings the engine pushes (settings.ts SettingsView), as the subset the phone reads; a key missing from the
/// push reads as DEFAULT_SETTINGS has it. Not mirrored: the desktop / IDE-only keys (sessionScope, sessionListPosition,
/// font sizes and smoothing, diffMarkers, shareEditorSelection, agentCpuCap, proxy, openFilesWith, desktopLayout,
/// panel*)
public struct SettingsView: Codable, Hashable, Sendable {
  /// `auto` or a Locale
  public var language: String
  /// The language resolved against the client's
  public var locale: String
  public var defaultAgent: AgentId
  /// Agent order of every list (ids not listed follow in registry order)
  public var agentOrder: [AgentId]
  /// Agents kept out of the new-session entry points; their sessions still open
  public var disabledAgents: [AgentId]
  public var autoCompact: Bool
  public var compactAtTokens: Int
  public var hiddenOptions: HiddenMap
  /// off | earliestReset | mostRemaining | listOrder
  public var accountSwitch: String
  /// auto | light | dark
  public var theme: String
  /// A queued prompt's send button steers it into the running turn where the agent supports it
  public var steerQueued: Bool
  public var notifySound: Bool
  public var notifySystem: Bool
  /// Agents whose Plan mode approves tool requests without a card
  public var planAutoApprove: [AgentId]
  /// The cross-harness subagents and their groups (~/.acpira/subagents.json): the composer's `@` summons
  public var subagents: [SubagentPersona]
  public var subagentGroups: [SubagentGroup]

  public init(from decoder: Decoder) throws {
    let c = try decoder.container(keyedBy: CodingKeys.self)
    language = try c.decode(.language, or: "auto")
    locale = try c.decode(.locale, or: "en")
    defaultAgent = try c.decode(.defaultAgent, or: "acpira")
    agentOrder = try c.decode(.agentOrder, or: [])
    disabledAgents = try c.decode(.disabledAgents, or: [])
    autoCompact = try c.decode(.autoCompact, or: true)
    compactAtTokens = try c.decode(.compactAtTokens, or: 300_000)
    hiddenOptions = try c.decode(.hiddenOptions, or: [:])
    accountSwitch = try c.decode(.accountSwitch, or: "off")
    theme = try c.decode(.theme, or: "auto")
    steerQueued = try c.decode(.steerQueued, or: false)
    notifySound = try c.decode(.notifySound, or: false)
    notifySystem = try c.decode(.notifySystem, or: false)
    planAutoApprove = try c.decode(.planAutoApprove, or: [])
    subagents = try c.decode(.subagents, or: [])
    subagentGroups = try c.decode(.subagentGroups, or: [])
  }
}

/// Everything a page needs to start (protocol.ts InitState), sent once after `ready`. Not mirrored: agentInstalls
/// (in-app agent installs, a desktop settings page)
public struct InitState: Codable, Hashable, Sendable {
  public var host: WebviewHost
  public var appearance: Appearance
  public var agents: [AgentInfo]
  public var accounts: [AccountInfo]
  public var accountActions: [AccountAction]?
  public var hidden: HiddenMap
  public var sessions: [SessionSummary]
  /// Absent from an older engine
  public var categories: SessionCategories?
  /// The session the view opens on, whole: the base the next sessionPatch applies to
  public var active: SessionView?
  public var settings: SettingsView
  public var locale: String
  /// The engine machine's home directory and the view's folder, for shortening paths
  public var home: String
  public var cwd: String
  /// Where an attachment blob loads from: `${blobBase}/${sessionId}/${blob}`
  public var blobBase: String?
  /// Where chats' folders live: a session whose cwd is directly inside is a chat, not a project
  public var chats: String?

  public init(from decoder: Decoder) throws {
    let c = try decoder.container(keyedBy: CodingKeys.self)
    host = try c.decode(.host, or: .sidebar)
    appearance = try c.decode(.appearance, or: [:])
    agents = try c.decode(.agents, or: [])
    accounts = try c.decode(.accounts, or: [])
    accountActions = try c.decodeIfPresent([AccountAction].self, forKey: .accountActions)
    hidden = try c.decode(.hidden, or: [:])
    sessions = try c.decode(.sessions, or: [])
    categories = try c.decodeIfPresent(SessionCategories.self, forKey: .categories)
    active = try c.decodeIfPresent(SessionView.self, forKey: .active)
    let settings = try c.decode(SettingsView.self, forKey: .settings)
    self.settings = settings
    locale = try c.decode(.locale, or: settings.locale)
    home = try c.decode(.home, or: "")
    cwd = try c.decode(.cwd, or: "")
    blobBase = try c.decodeIfPresent(String.self, forKey: .blobBase)
    chats = try c.decodeIfPresent(String.self, forKey: .chats)
  }
}

/// The changes since the session view the page was sent last (sessionPatch.ts): keep `turns[0..<keep]` of the view at
/// rev `base` and append `turns`; with `keepBlocks`, `turns[0]` is the head of turn `keep` (its blocks empty) and that
/// turn's blocks are the base turn's first `keepBlocks` followed by `blocks`. `view` is the whole view but its turns.
/// When the page does not hold the view at `base`, it answers with `resync`
public struct SessionPatch: Codable, Hashable, Sendable {
  public var id: String
  public var base: Int
  public var view: SessionView
  public var keep: Int
  public var turns: [Turn]
  public var keepBlocks: Int?
  public var blocks: [AgentBlock]?

  public init(id: String, base: Int, view: SessionView, keep: Int, turns: [Turn], keepBlocks: Int? = nil, blocks: [AgentBlock]? = nil) {
    self.id = id; self.base = base; self.view = view; self.keep = keep; self.turns = turns
    self.keepBlocks = keepBlocks; self.blocks = blocks
  }
}

// MARK: - Page → host

/// The actions the phone sends (WebviewMsg), wrapped in a webviewMessage envelope. A session action carries the id of
/// the session the view shows; the engine refuses one for another session. TypeScript marks some of those ids optional
/// for tests and scripts (absent: the view's current session); the app always passes them. clientId (send, stop,
/// permission, answer, steerQueued): set by a paired device, which resends with the same id after a lost link; the
/// engine running the session runs each id once
public enum WebviewMsg: Codable, Hashable, Sendable {
  /// The page is mounted: the engine answers with `init`. patches: the page applies `sessionPatch` (the app sends true)
  case ready(patches: Bool? = nil)
  /// A sessionPatch did not fit the view the page holds: the engine sends the whole view again
  case resync
  /// steer: join the running turn instead of queueing behind it (SessionView.canSteer)
  case send(sessionId: String?, text: String, attachments: [Draft]? = nil, steer: Bool? = nil, clientId: String? = nil)
  case stop(sessionId: String?, clientId: String? = nil)
  case permission(sessionId: String, blockId: String, optionId: String, clientId: String? = nil)
  /// The question card closed: `answers` holds the answered questions only; skip tells the agent to go on without them
  case answer(sessionId: String, blockId: String, answers: QuestionAnswers, skip: Bool? = nil, clientId: String? = nil)
  /// Build a reviewed plan: optionId picks the card's option, model the model it is built with
  case buildPlan(sessionId: String, planId: String, optionId: String? = nil, model: ModelPick? = nil)
  case setMode(sessionId: String?, id: String)
  case setConfig(sessionId: String?, configId: String, value: String)
  case selectSession(id: String)
  /// category: file it under that category from the start; cwd: the project folder to start in
  /// `chat`: a session in a fresh folder of its own under InitState.chats (project shells), `cwd` ignored
  case newSession(agent: AgentId? = nil, category: String? = nil, cwd: String? = nil, chat: Bool? = nil)
  case renameSession(id: String, title: String)
  case deleteSession(id: String)
  case restoreSession(id: String)
  case pinSession(id: String, pinned: Bool)
  /// Every unread mark goes, for every engine and window on the machine
  case markAllRead
  /// Full-text search over saved conversations, answered with `sessionHits` echoing seq
  case searchSessions(query: String, seq: Int)
  /// A new session of the same project holding this session's turns up to and including `turnIndex` (an agent turn);
  /// agent: continue it on another agent
  case forkSession(sessionId: String, turnIndex: Int, agent: AgentId? = nil)
  /// Rebind the session to this account (also the agent's default account from then on)
  case selectAccount(sessionId: String?, id: String)
  /// An account list came into view: refresh that agent's quotas
  case refreshQuota(agent: AgentId)
  case compact(sessionId: String?)
  case retry(sessionId: String?)
  /// SessionView.canTakeOver: end the agent another engine left holding the session, or ask the engine that has this
  /// read-only copy open to hand it over
  case takeOverSession(sessionId: String?)
  /// Queued prompts: drop one, send it now, steer it into the running turn, or replace it in place (kept attachments
  /// by index, new drafts alongside)
  case dequeue(sessionId: String, id: String)
  case sendQueued(sessionId: String, id: String)
  case steerQueued(sessionId: String, id: String, clientId: String? = nil)
  case editQueued(sessionId: String, id: String, text: String, retainedAttachments: [Int], attachments: [Draft])
  /// An edit of the project list; `file`: with a create, file that session under the new category in the same step
  case categoryOp(op: CategoryOp, file: String? = nil)
  /// Re-home a session into another project folder (absent: the view's workspace folder), the agent working there from
  /// the next open on; `category`, one of that project's, files it there in the same step
  case moveSession(id: String, cwd: String? = nil, category: String? = nil)
  /// "Copy as Markdown": the export's Markdown as text, answered with `sessionMarkdown` echoing seq
  case sessionMarkdown(id: String, seq: Int)
  case addAccount(agent: AgentId, via: AddAccountVia)
  case removeAccount(id: String)
  /// AgentInfo.credentialsLocked: unlock the credential store in a terminal, then reconnect the agent's sessions stuck
  /// on sign-in
  case unlockCredentials(agent: AgentId)
  /// Sign in with one of SessionView.authMethods
  case login(sessionId: String?, methodId: String? = nil)
  /// Drop the agent process and resume the same native session (prompts keep failing on the live connection)
  case reconnect(sessionId: String?)
  /// Send the last user turn again after its agent turn ended in error or short; both turns are dropped first
  case retryTurn(sessionId: String?)
  /// Rewrite a sent prompt, answered with `editTurnResult` echoing requestId
  case editTurn(requestId: String, edit: EditTurnRequest)
  /// Follow / stop following one subagent's transcript (the `subagent` push); per viewer, one at a time
  case observeSubagent(sessionId: String, subagentId: String)
  case unobserveSubagent(sessionId: String, subagentId: String)
  /// Ask the agent to cancel one child
  case cancelSubagent(sessionId: String, subagentId: String)
  /// Ask the adapter to stop one AIR async task
  case stopAsyncTask(sessionId: String, taskId: String)
  /// A goal control (SessionView.goalActions): turn-starting ones go out as a `/goal …` prompt, which queues behind a
  /// running turn like any prompt
  case goal(sessionId: String, action: GoalAction, objective: String? = nil)
  /// @ mention: fuzzy search over the workspace's files, answered with `files` echoing seq
  case searchFiles(query: String, seq: Int)
  /// Write a setting (settings.ts SettingKey); the engine sanitizes the value
  case setSetting(key: String, value: JSONValue)
  /// An agent's model controls, answered with `controls`; fresh: re-read them from a throwaway process
  case controls(agent: AgentId, fresh: Bool? = nil)
  /// The session's project, each answered by the push of the same name echoing seq: a folder (`dir` relative, "" for
  /// the project itself), its git marks, a file with its hunks against `base` (none: no hunks), the files the session's
  /// edits left changed
  case projectDir(sessionId: String, dir: String, seq: Int)
  case projectStatus(sessionId: String, seq: Int)
  case projectFile(sessionId: String, path: String, base: ProjectBase? = nil, seq: Int)
  case sessionChanges(sessionId: String, seq: Int)
  /// The whole files behind the `nth` diff of a tool call (counted in `contents`, or `content` alone), answered with
  /// `diffSource` echoing seq; subagentId: the subagent transcript the diff was shown in
  case diffSource(sessionId: String, toolCallId: String, nth: Int, subagentId: String? = nil, seq: Int)
  /// Stage an image file path into the session's blob store, answered with `imageBlob` echoing seq
  case imageBlob(sessionId: String, path: String, seq: Int)
  /// Open a plan document; the engine passes it on as the openPlanDocument platform request
  case openPlan(sessionId: String, planId: String)
  /// The engine's terminals (terminal.rs): open terminal `id` (a login shell in `cwd`) or view it again; keys and
  /// pastes (`binary`: each character one byte); the laid-out size; how much output (UTF-16 units) the page has
  /// rendered, since output pauses while too much is not; stop viewing it (it runs on a while for a reload); end its
  /// shell (the tab closed or restarts)
  case terminalOpen(id: String, cwd: String? = nil, cols: Int? = nil, rows: Int? = nil)
  case terminalInput(id: String, data: String, binary: Bool? = nil)
  case terminalResize(id: String, cols: Int, rows: Int)
  case terminalAck(id: String, chars: Int)
  case terminalClose(id: String)
  case terminalKill(id: String)
  /// The SSH hosts' state, answered with a `connections` push and pushed again on every change
  case connections
  /// A type this build does not know (decoding only)
  case unknown(String)

  /// The `type` literals this build mirrors: the actions the app can send
  public static let types = [
    "ready", "resync", "send", "stop", "permission", "answer", "buildPlan", "setMode", "setConfig", "selectSession",
    "newSession", "renameSession", "deleteSession", "restoreSession", "pinSession", "markAllRead", "searchSessions",
    "forkSession", "selectAccount", "refreshQuota", "compact", "retry", "takeOverSession", "dequeue", "sendQueued",
    "steerQueued", "editQueued", "categoryOp", "moveSession", "sessionMarkdown", "addAccount", "removeAccount",
    "unlockCredentials", "login", "reconnect", "retryTurn", "editTurn", "observeSubagent", "unobserveSubagent",
    "cancelSubagent", "stopAsyncTask", "goal", "searchFiles", "setSetting", "controls", "projectDir", "projectStatus",
    "projectFile", "sessionChanges", "diffSource", "imageBlob", "openPlan", "terminalOpen", "terminalInput",
    "terminalResize", "terminalAck", "terminalClose", "terminalKill", "connections",
  ]

  /// The wire `type`
  public var type: String {
    switch self {
    case .ready: "ready"
    case .resync: "resync"
    case .send: "send"
    case .stop: "stop"
    case .permission: "permission"
    case .answer: "answer"
    case .buildPlan: "buildPlan"
    case .setMode: "setMode"
    case .setConfig: "setConfig"
    case .selectSession: "selectSession"
    case .newSession: "newSession"
    case .renameSession: "renameSession"
    case .deleteSession: "deleteSession"
    case .restoreSession: "restoreSession"
    case .pinSession: "pinSession"
    case .markAllRead: "markAllRead"
    case .connections: "connections"
    case .searchSessions: "searchSessions"
    case .forkSession: "forkSession"
    case .selectAccount: "selectAccount"
    case .refreshQuota: "refreshQuota"
    case .compact: "compact"
    case .retry: "retry"
    case .takeOverSession: "takeOverSession"
    case .dequeue: "dequeue"
    case .sendQueued: "sendQueued"
    case .steerQueued: "steerQueued"
    case .editQueued: "editQueued"
    case .categoryOp: "categoryOp"
    case .moveSession: "moveSession"
    case .sessionMarkdown: "sessionMarkdown"
    case .addAccount: "addAccount"
    case .removeAccount: "removeAccount"
    case .unlockCredentials: "unlockCredentials"
    case .login: "login"
    case .reconnect: "reconnect"
    case .retryTurn: "retryTurn"
    case .editTurn: "editTurn"
    case .observeSubagent: "observeSubagent"
    case .unobserveSubagent: "unobserveSubagent"
    case .cancelSubagent: "cancelSubagent"
    case .stopAsyncTask: "stopAsyncTask"
    case .goal: "goal"
    case .searchFiles: "searchFiles"
    case .setSetting: "setSetting"
    case .controls: "controls"
    case .projectDir: "projectDir"
    case .projectStatus: "projectStatus"
    case .projectFile: "projectFile"
    case .sessionChanges: "sessionChanges"
    case .diffSource: "diffSource"
    case .imageBlob: "imageBlob"
    case .openPlan: "openPlan"
    case .terminalOpen: "terminalOpen"
    case .terminalInput: "terminalInput"
    case .terminalResize: "terminalResize"
    case .terminalAck: "terminalAck"
    case .terminalClose: "terminalClose"
    case .terminalKill: "terminalKill"
    case .unknown(let t): t
    }
  }

  private enum Keys: String, CodingKey {
    case type, patches, sessionId, text, attachments, steer, blockId, optionId, answers, skip, planId, model, id, configId
    case value, agent, category, cwd, chat, title, pinned, query, seq, turnIndex, retainedAttachments, clientId
    case file, via, methodId, requestId, edit, subagentId, taskId, action, objective, key, fresh, dir, path, base
    case toolCallId, nth, cols, rows, data, binary, chars
  }

  public init(from decoder: Decoder) throws {
    let c = try decoder.container(keyedBy: Keys.self)
    func str(_ k: Keys) throws -> String { try c.decode(String.self, forKey: k) }
    func opt(_ k: Keys) throws -> String? { try c.decodeIfPresent(String.self, forKey: k) }
    func int(_ k: Keys) throws -> Int { try c.decode(Int.self, forKey: k) }
    switch try str(.type) {
    case "ready": self = .ready(patches: try c.decodeIfPresent(Bool.self, forKey: .patches))
    case "resync": self = .resync
    case "send":
      self = .send(sessionId: try opt(.sessionId), text: try str(.text), attachments: try c.decodeIfPresent([Draft].self, forKey: .attachments),
                   steer: try c.decodeIfPresent(Bool.self, forKey: .steer), clientId: try opt(.clientId))
    case "stop": self = .stop(sessionId: try opt(.sessionId), clientId: try opt(.clientId))
    case "permission":
      self = .permission(sessionId: try str(.sessionId), blockId: try str(.blockId), optionId: try str(.optionId), clientId: try opt(.clientId))
    case "answer":
      self = .answer(sessionId: try str(.sessionId), blockId: try str(.blockId), answers: try c.decode(QuestionAnswers.self, forKey: .answers),
                     skip: try c.decodeIfPresent(Bool.self, forKey: .skip), clientId: try opt(.clientId))
    case "buildPlan":
      self = .buildPlan(sessionId: try str(.sessionId), planId: try str(.planId), optionId: try opt(.optionId),
                        model: try c.decodeIfPresent(ModelPick.self, forKey: .model))
    case "setMode": self = .setMode(sessionId: try opt(.sessionId), id: try str(.id))
    case "setConfig": self = .setConfig(sessionId: try opt(.sessionId), configId: try str(.configId), value: try str(.value))
    case "selectSession": self = .selectSession(id: try str(.id))
    case "newSession": self = .newSession(agent: try opt(.agent), category: try opt(.category), cwd: try opt(.cwd), chat: try c.decodeIfPresent(Bool.self, forKey: .chat))
    case "renameSession": self = .renameSession(id: try str(.id), title: try str(.title))
    case "deleteSession": self = .deleteSession(id: try str(.id))
    case "restoreSession": self = .restoreSession(id: try str(.id))
    case "pinSession": self = .pinSession(id: try str(.id), pinned: try c.decode(Bool.self, forKey: .pinned))
    case "markAllRead": self = .markAllRead
    case "connections": self = .connections
    case "searchSessions": self = .searchSessions(query: try str(.query), seq: try c.decode(Int.self, forKey: .seq))
    case "forkSession":
      self = .forkSession(sessionId: try str(.sessionId), turnIndex: try c.decode(Int.self, forKey: .turnIndex), agent: try opt(.agent))
    case "selectAccount": self = .selectAccount(sessionId: try opt(.sessionId), id: try str(.id))
    case "refreshQuota": self = .refreshQuota(agent: try str(.agent))
    case "compact": self = .compact(sessionId: try opt(.sessionId))
    case "retry": self = .retry(sessionId: try opt(.sessionId))
    case "takeOverSession": self = .takeOverSession(sessionId: try opt(.sessionId))
    case "dequeue": self = .dequeue(sessionId: try str(.sessionId), id: try str(.id))
    case "sendQueued": self = .sendQueued(sessionId: try str(.sessionId), id: try str(.id))
    case "steerQueued": self = .steerQueued(sessionId: try str(.sessionId), id: try str(.id), clientId: try opt(.clientId))
    case "editQueued":
      self = .editQueued(sessionId: try str(.sessionId), id: try str(.id), text: try str(.text),
                         retainedAttachments: try c.decode([Int].self, forKey: .retainedAttachments),
                         attachments: try c.decode([Draft].self, forKey: .attachments))
    case "categoryOp": self = .categoryOp(op: try CategoryOp(from: decoder), file: try opt(.file))
    case "moveSession": self = .moveSession(id: try str(.id), cwd: try opt(.cwd), category: try opt(.category))
    case "sessionMarkdown": self = .sessionMarkdown(id: try str(.id), seq: try int(.seq))
    case "addAccount": self = .addAccount(agent: try str(.agent), via: try c.decode(AddAccountVia.self, forKey: .via))
    case "removeAccount": self = .removeAccount(id: try str(.id))
    case "unlockCredentials": self = .unlockCredentials(agent: try str(.agent))
    case "login": self = .login(sessionId: try opt(.sessionId), methodId: try opt(.methodId))
    case "reconnect": self = .reconnect(sessionId: try opt(.sessionId))
    case "retryTurn": self = .retryTurn(sessionId: try opt(.sessionId))
    case "editTurn": self = .editTurn(requestId: try str(.requestId), edit: try c.decode(EditTurnRequest.self, forKey: .edit))
    case "observeSubagent": self = .observeSubagent(sessionId: try str(.sessionId), subagentId: try str(.subagentId))
    case "unobserveSubagent": self = .unobserveSubagent(sessionId: try str(.sessionId), subagentId: try str(.subagentId))
    case "cancelSubagent": self = .cancelSubagent(sessionId: try str(.sessionId), subagentId: try str(.subagentId))
    case "stopAsyncTask": self = .stopAsyncTask(sessionId: try str(.sessionId), taskId: try str(.taskId))
    case "goal":
      self = .goal(sessionId: try str(.sessionId), action: try c.decode(GoalAction.self, forKey: .action), objective: try opt(.objective))
    case "searchFiles": self = .searchFiles(query: try str(.query), seq: try int(.seq))
    case "setSetting": self = .setSetting(key: try str(.key), value: try c.decode(JSONValue.self, forKey: .value))
    case "controls": self = .controls(agent: try str(.agent), fresh: try c.decodeIfPresent(Bool.self, forKey: .fresh))
    case "projectDir": self = .projectDir(sessionId: try str(.sessionId), dir: try str(.dir), seq: try int(.seq))
    case "projectStatus": self = .projectStatus(sessionId: try str(.sessionId), seq: try int(.seq))
    case "projectFile":
      self = .projectFile(sessionId: try str(.sessionId), path: try str(.path), base: try c.decodeIfPresent(ProjectBase.self, forKey: .base),
                          seq: try int(.seq))
    case "sessionChanges": self = .sessionChanges(sessionId: try str(.sessionId), seq: try int(.seq))
    case "diffSource":
      self = .diffSource(sessionId: try str(.sessionId), toolCallId: try str(.toolCallId), nth: try int(.nth),
                         subagentId: try opt(.subagentId), seq: try int(.seq))
    case "imageBlob": self = .imageBlob(sessionId: try str(.sessionId), path: try str(.path), seq: try int(.seq))
    case "openPlan": self = .openPlan(sessionId: try str(.sessionId), planId: try str(.planId))
    case "terminalOpen":
      self = .terminalOpen(id: try str(.id), cwd: try opt(.cwd), cols: try c.decodeIfPresent(Int.self, forKey: .cols),
                           rows: try c.decodeIfPresent(Int.self, forKey: .rows))
    case "terminalInput":
      self = .terminalInput(id: try str(.id), data: try str(.data), binary: try c.decodeIfPresent(Bool.self, forKey: .binary))
    case "terminalResize": self = .terminalResize(id: try str(.id), cols: try int(.cols), rows: try int(.rows))
    case "terminalAck": self = .terminalAck(id: try str(.id), chars: try int(.chars))
    case "terminalClose": self = .terminalClose(id: try str(.id))
    case "terminalKill": self = .terminalKill(id: try str(.id))
    case let type: self = .unknown(type)
    }
  }

  public func encode(to encoder: Encoder) throws {
    var c = encoder.container(keyedBy: Keys.self)
    try c.encode(type, forKey: .type)
    switch self {
    case .ready(let patches):
      try c.encodeIfPresent(patches, forKey: .patches)
    case let .send(sessionId, text, attachments, steer, clientId):
      try c.encodeIfPresent(sessionId, forKey: .sessionId)
      try c.encode(text, forKey: .text)
      try c.encodeIfPresent(attachments, forKey: .attachments)
      try c.encodeIfPresent(steer, forKey: .steer)
      try c.encodeIfPresent(clientId, forKey: .clientId)
    case let .stop(sessionId, clientId):
      try c.encodeIfPresent(sessionId, forKey: .sessionId)
      try c.encodeIfPresent(clientId, forKey: .clientId)
    case .compact(let sessionId), .retry(let sessionId), .takeOverSession(let sessionId), .reconnect(let sessionId),
         .retryTurn(let sessionId):
      try c.encodeIfPresent(sessionId, forKey: .sessionId)
    case let .permission(sessionId, blockId, optionId, clientId):
      try c.encode(sessionId, forKey: .sessionId)
      try c.encode(blockId, forKey: .blockId)
      try c.encode(optionId, forKey: .optionId)
      try c.encodeIfPresent(clientId, forKey: .clientId)
    case let .answer(sessionId, blockId, answers, skip, clientId):
      try c.encode(sessionId, forKey: .sessionId)
      try c.encode(blockId, forKey: .blockId)
      try c.encode(answers, forKey: .answers)
      try c.encodeIfPresent(skip, forKey: .skip)
      try c.encodeIfPresent(clientId, forKey: .clientId)
    case let .buildPlan(sessionId, planId, optionId, model):
      try c.encode(sessionId, forKey: .sessionId)
      try c.encode(planId, forKey: .planId)
      try c.encodeIfPresent(optionId, forKey: .optionId)
      try c.encodeIfPresent(model, forKey: .model)
    case let .setMode(sessionId, id), let .selectAccount(sessionId, id):
      try c.encodeIfPresent(sessionId, forKey: .sessionId)
      try c.encode(id, forKey: .id)
    case let .setConfig(sessionId, configId, value):
      try c.encodeIfPresent(sessionId, forKey: .sessionId)
      try c.encode(configId, forKey: .configId)
      try c.encode(value, forKey: .value)
    case .selectSession(let id), .deleteSession(let id), .restoreSession(let id):
      try c.encode(id, forKey: .id)
    case let .newSession(agent, category, cwd, chat):
      try c.encodeIfPresent(agent, forKey: .agent)
      try c.encodeIfPresent(category, forKey: .category)
      try c.encodeIfPresent(cwd, forKey: .cwd)
      try c.encodeIfPresent(chat, forKey: .chat)
    case let .renameSession(id, title):
      try c.encode(id, forKey: .id)
      try c.encode(title, forKey: .title)
    case let .pinSession(id, pinned):
      try c.encode(id, forKey: .id)
      try c.encode(pinned, forKey: .pinned)
    case let .searchSessions(query, seq):
      try c.encode(query, forKey: .query)
      try c.encode(seq, forKey: .seq)
    case let .forkSession(sessionId, turnIndex, agent):
      try c.encode(sessionId, forKey: .sessionId)
      try c.encode(turnIndex, forKey: .turnIndex)
      try c.encodeIfPresent(agent, forKey: .agent)
    case .refreshQuota(let agent):
      try c.encode(agent, forKey: .agent)
    case let .dequeue(sessionId, id), let .sendQueued(sessionId, id):
      try c.encode(sessionId, forKey: .sessionId)
      try c.encode(id, forKey: .id)
    case let .steerQueued(sessionId, id, clientId):
      try c.encode(sessionId, forKey: .sessionId)
      try c.encode(id, forKey: .id)
      try c.encodeIfPresent(clientId, forKey: .clientId)
    case let .editQueued(sessionId, id, text, retainedAttachments, attachments):
      try c.encode(sessionId, forKey: .sessionId)
      try c.encode(id, forKey: .id)
      try c.encode(text, forKey: .text)
      try c.encode(retainedAttachments, forKey: .retainedAttachments)
      try c.encode(attachments, forKey: .attachments)
    case let .categoryOp(op, file):
      try op.encode(to: encoder)
      try c.encodeIfPresent(file, forKey: .file)
    case let .moveSession(id, cwd, category):
      try c.encode(id, forKey: .id)
      try c.encodeIfPresent(cwd, forKey: .cwd)
      try c.encodeIfPresent(category, forKey: .category)
    case let .sessionMarkdown(id, seq):
      try c.encode(id, forKey: .id)
      try c.encode(seq, forKey: .seq)
    case let .addAccount(agent, via):
      try c.encode(agent, forKey: .agent)
      try c.encode(via, forKey: .via)
    case .removeAccount(let id), .terminalClose(let id), .terminalKill(let id):
      try c.encode(id, forKey: .id)
    case .unlockCredentials(let agent):
      try c.encode(agent, forKey: .agent)
    case let .login(sessionId, methodId):
      try c.encodeIfPresent(sessionId, forKey: .sessionId)
      try c.encodeIfPresent(methodId, forKey: .methodId)
    case let .editTurn(requestId, edit):
      try c.encode(requestId, forKey: .requestId)
      try c.encode(edit, forKey: .edit)
    case let .observeSubagent(sessionId, subagentId), let .unobserveSubagent(sessionId, subagentId),
         let .cancelSubagent(sessionId, subagentId):
      try c.encode(sessionId, forKey: .sessionId)
      try c.encode(subagentId, forKey: .subagentId)
    case let .stopAsyncTask(sessionId, taskId):
      try c.encode(sessionId, forKey: .sessionId)
      try c.encode(taskId, forKey: .taskId)
    case let .goal(sessionId, action, objective):
      try c.encode(sessionId, forKey: .sessionId)
      try c.encode(action, forKey: .action)
      try c.encodeIfPresent(objective, forKey: .objective)
    case let .searchFiles(query, seq):
      try c.encode(query, forKey: .query)
      try c.encode(seq, forKey: .seq)
    case let .setSetting(key, value):
      try c.encode(key, forKey: .key)
      try c.encode(value, forKey: .value)
    case let .controls(agent, fresh):
      try c.encode(agent, forKey: .agent)
      try c.encodeIfPresent(fresh, forKey: .fresh)
    case let .projectDir(sessionId, dir, seq):
      try c.encode(sessionId, forKey: .sessionId)
      try c.encode(dir, forKey: .dir)
      try c.encode(seq, forKey: .seq)
    case let .projectStatus(sessionId, seq), let .sessionChanges(sessionId, seq):
      try c.encode(sessionId, forKey: .sessionId)
      try c.encode(seq, forKey: .seq)
    case let .projectFile(sessionId, path, base, seq):
      try c.encode(sessionId, forKey: .sessionId)
      try c.encode(path, forKey: .path)
      try c.encodeIfPresent(base, forKey: .base)
      try c.encode(seq, forKey: .seq)
    case let .diffSource(sessionId, toolCallId, nth, subagentId, seq):
      try c.encode(sessionId, forKey: .sessionId)
      try c.encode(toolCallId, forKey: .toolCallId)
      try c.encode(nth, forKey: .nth)
      try c.encodeIfPresent(subagentId, forKey: .subagentId)
      try c.encode(seq, forKey: .seq)
    case let .imageBlob(sessionId, path, seq):
      try c.encode(sessionId, forKey: .sessionId)
      try c.encode(path, forKey: .path)
      try c.encode(seq, forKey: .seq)
    case let .openPlan(sessionId, planId):
      try c.encode(sessionId, forKey: .sessionId)
      try c.encode(planId, forKey: .planId)
    case let .terminalOpen(id, cwd, cols, rows):
      try c.encode(id, forKey: .id)
      try c.encodeIfPresent(cwd, forKey: .cwd)
      try c.encodeIfPresent(cols, forKey: .cols)
      try c.encodeIfPresent(rows, forKey: .rows)
    case let .terminalInput(id, data, binary):
      try c.encode(id, forKey: .id)
      try c.encode(data, forKey: .data)
      try c.encodeIfPresent(binary, forKey: .binary)
    case let .terminalResize(id, cols, rows):
      try c.encode(id, forKey: .id)
      try c.encode(cols, forKey: .cols)
      try c.encode(rows, forKey: .rows)
    case let .terminalAck(id, chars):
      try c.encode(id, forKey: .id)
      try c.encode(chars, forKey: .chars)
    case .resync, .markAllRead, .connections, .unknown:
      break
    }
  }
}

// MARK: - Host → page

/// The pushes the phone renders (HostMsg), each inside a hostMessage envelope. Anything else decodes to `.unknown`
public enum HostMsg: Codable, Hashable, Sendable {
  /// `init`: everything the page needs, the answer to `ready`
  case initState(state: InitState)
  case agents(agents: [AgentInfo])
  case sessions(sessions: [SessionSummary])
  /// The category list changed (here or in another window)
  case categories(categories: SessionCategories)
  /// The view shows no session (a project shell with no project left)
  case noSession
  /// The whole session view: the first push of a session, a session switch, or the answer to `resync`
  case session(session: SessionView)
  /// The changes since the view sent last; only to pages that said `patches` in ready
  case sessionPatch(patch: SessionPatch)
  case accounts(accounts: [AccountInfo])
  case accountActions(actions: [AccountAction])
  case hidden(hidden: HiddenMap)
  /// The settings view and the resolved locale (a language change swaps both at once)
  case settings(settings: SettingsView, locale: String)
  /// An agent's model controls, answering the settings page's `controls` request; shapes: its remembered per-model ones
  case controls(agent: AgentId, controls: [ConfigControl], shapes: ModelShapes? = nil)
  /// The answer to searchSessions with the same seq; a lower seq is stale
  case sessionHits(seq: Int, hits: [SessionHit])
  /// One observed subagent's transcript, pushed to the viewer that asked for it; rev grows with every change
  case subagent(sessionId: String, subagentId: String, rev: Int, running: Bool, turns: [Turn])
  /// The answer to searchFiles with the same seq
  case files(seq: Int, files: [FileHit])
  /// The answer to diffSource: the whole files behind one diff, absent when the session or the diff is gone
  case diffSource(seq: Int, source: FullDiffSource? = nil)
  /// The answer to imageBlob: the session blob name for the image file, absent when unreadable
  case imageBlob(seq: Int, blob: String? = nil)
  /// The answer to sessionMarkdown: the session as Markdown, or why it could not be read
  case sessionMarkdown(seq: Int, markdown: String? = nil, error: String? = nil)
  /// The answers to the project reads, each echoing its request's seq. projectStatus: the project folder and the git
  /// marks by relative path, empty outside a work tree
  case projectDir(seq: Int, entries: [ProjectEntry], error: ProjectError? = nil)
  case projectStatus(seq: Int, root: String? = nil, marks: [String: GitMark])
  case projectFile(seq: Int, text: String? = nil, hunks: [ProjectHunk]? = nil, error: ProjectError? = nil)
  /// The files the session's edits left changed, in the order it first edited them
  case sessionChanges(seq: Int, files: [ProjectChange])
  /// A terminal's output as it comes, to every page viewing it
  case terminalOutput(id: String, data: String)
  /// A terminal's state. Answering terminalOpen it carries the size, the title, the output so far (`replay`, shown
  /// instead of what the page had) and the exit if the shell ended; later only what changed. error: the shell could not
  /// start
  case terminalState(id: String, title: String? = nil, cols: Int? = nil, rows: Int? = nil, replay: String? = nil,
                     exit: TerminalExit? = nil, error: String? = nil)
  /// The answer to editTurn with the same requestId; error: why the edit was refused
  case editTurnResult(requestId: String, error: String? = nil)
  /// The appearance axes changed
  case appearance(appearance: Appearance)
  /// The SSH hosts, after `connections` was asked and on every change since
  case connections(connections: [ConnectionView])
  /// A push this build does not mirror; carries the type for diagnostics
  case unknown(String)

  /// The `type` literals this build mirrors
  public static let types = [
    "init", "agents", "sessions", "categories", "noSession", "session", "sessionPatch", "accounts", "accountActions",
    "hidden", "settings", "controls", "sessionHits", "subagent", "files", "diffSource", "imageBlob", "sessionMarkdown",
    "projectDir", "projectStatus", "projectFile", "sessionChanges", "terminalOutput", "terminalState", "editTurnResult",
    "appearance", "connections",
  ]

  /// The wire `type`
  public var type: String {
    switch self {
    case .initState: "init"
    case .agents: "agents"
    case .sessions: "sessions"
    case .categories: "categories"
    case .noSession: "noSession"
    case .session: "session"
    case .sessionPatch: "sessionPatch"
    case .accounts: "accounts"
    case .accountActions: "accountActions"
    case .hidden: "hidden"
    case .settings: "settings"
    case .controls: "controls"
    case .sessionHits: "sessionHits"
    case .subagent: "subagent"
    case .files: "files"
    case .diffSource: "diffSource"
    case .imageBlob: "imageBlob"
    case .sessionMarkdown: "sessionMarkdown"
    case .projectDir: "projectDir"
    case .projectStatus: "projectStatus"
    case .projectFile: "projectFile"
    case .sessionChanges: "sessionChanges"
    case .terminalOutput: "terminalOutput"
    case .terminalState: "terminalState"
    case .editTurnResult: "editTurnResult"
    case .appearance: "appearance"
    case .connections: "connections"
    case .unknown(let t): t
    }
  }

  private enum Keys: String, CodingKey {
    case type, state, agents, sessions, categories, session, patch, accounts, actions, hidden, settings, locale, agent
    case controls, shapes, seq, hits
    case sessionId, subagentId, rev, running, turns, files, source, blob, markdown, error, entries, root, marks, text
    case hunks, id, data, title, cols, rows, replay, exit, requestId, appearance, connections
  }

  public init(from decoder: Decoder) throws {
    let c = try decoder.container(keyedBy: Keys.self)
    func str(_ k: Keys) throws -> String { try c.decode(String.self, forKey: k) }
    func opt(_ k: Keys) throws -> String? { try c.decodeIfPresent(String.self, forKey: k) }
    func int(_ k: Keys) throws -> Int { try c.decode(Int.self, forKey: k) }
    switch try c.decode(String.self, forKey: .type) {
    case "init": self = .initState(state: try c.decode(InitState.self, forKey: .state))
    case "agents": self = .agents(agents: try c.decode([AgentInfo].self, forKey: .agents))
    case "sessions": self = .sessions(sessions: try c.decode([SessionSummary].self, forKey: .sessions))
    case "categories": self = .categories(categories: try c.decode(SessionCategories.self, forKey: .categories))
    case "noSession": self = .noSession
    case "session": self = .session(session: try c.decode(SessionView.self, forKey: .session))
    case "sessionPatch": self = .sessionPatch(patch: try c.decode(SessionPatch.self, forKey: .patch))
    case "accounts": self = .accounts(accounts: try c.decode([AccountInfo].self, forKey: .accounts))
    case "accountActions": self = .accountActions(actions: try c.decode([AccountAction].self, forKey: .actions))
    case "hidden": self = .hidden(hidden: try c.decode(HiddenMap.self, forKey: .hidden))
    case "settings":
      self = .settings(settings: try c.decode(SettingsView.self, forKey: .settings), locale: try c.decode(String.self, forKey: .locale))
    case "controls":
      self = .controls(agent: try c.decode(String.self, forKey: .agent), controls: try c.decode([ConfigControl].self, forKey: .controls),
                       shapes: try c.decodeIfPresent(ModelShapes.self, forKey: .shapes))
    case "sessionHits": self = .sessionHits(seq: try c.decode(Int.self, forKey: .seq), hits: try c.decode([SessionHit].self, forKey: .hits))
    case "subagent":
      self = .subagent(sessionId: try str(.sessionId), subagentId: try str(.subagentId), rev: try int(.rev),
                       running: try c.decode(Bool.self, forKey: .running), turns: try c.decode([Turn].self, forKey: .turns))
    case "files": self = .files(seq: try int(.seq), files: try c.decode([FileHit].self, forKey: .files))
    case "diffSource": self = .diffSource(seq: try int(.seq), source: try c.decodeIfPresent(FullDiffSource.self, forKey: .source))
    case "imageBlob": self = .imageBlob(seq: try int(.seq), blob: try opt(.blob))
    case "sessionMarkdown": self = .sessionMarkdown(seq: try int(.seq), markdown: try opt(.markdown), error: try opt(.error))
    case "projectDir":
      self = .projectDir(seq: try int(.seq), entries: try c.decode([ProjectEntry].self, forKey: .entries),
                         error: try c.decodeIfPresent(ProjectError.self, forKey: .error))
    case "projectStatus":
      self = .projectStatus(seq: try int(.seq), root: try opt(.root), marks: try c.decode([String: GitMark].self, forKey: .marks))
    case "projectFile":
      self = .projectFile(seq: try int(.seq), text: try opt(.text), hunks: try c.decodeIfPresent([ProjectHunk].self, forKey: .hunks),
                          error: try c.decodeIfPresent(ProjectError.self, forKey: .error))
    case "sessionChanges": self = .sessionChanges(seq: try int(.seq), files: try c.decode([ProjectChange].self, forKey: .files))
    case "terminalOutput": self = .terminalOutput(id: try str(.id), data: try str(.data))
    case "terminalState":
      self = .terminalState(id: try str(.id), title: try opt(.title), cols: try c.decodeIfPresent(Int.self, forKey: .cols),
                            rows: try c.decodeIfPresent(Int.self, forKey: .rows), replay: try opt(.replay),
                            exit: try c.decodeIfPresent(TerminalExit.self, forKey: .exit), error: try opt(.error))
    case "editTurnResult": self = .editTurnResult(requestId: try str(.requestId), error: try opt(.error))
    case "appearance": self = .appearance(appearance: try c.decode(Appearance.self, forKey: .appearance))
    case "connections": self = .connections(connections: try c.decode([ConnectionView].self, forKey: .connections))
    case let type: self = .unknown(type)
    }
  }

  public func encode(to encoder: Encoder) throws {
    var c = encoder.container(keyedBy: Keys.self)
    try c.encode(type, forKey: .type)
    switch self {
    case .initState(let state): try c.encode(state, forKey: .state)
    case .agents(let agents): try c.encode(agents, forKey: .agents)
    case .sessions(let sessions): try c.encode(sessions, forKey: .sessions)
    case .categories(let categories): try c.encode(categories, forKey: .categories)
    case .session(let session): try c.encode(session, forKey: .session)
    case .sessionPatch(let patch): try c.encode(patch, forKey: .patch)
    case .accounts(let accounts): try c.encode(accounts, forKey: .accounts)
    case .accountActions(let actions): try c.encode(actions, forKey: .actions)
    case .hidden(let hidden): try c.encode(hidden, forKey: .hidden)
    case let .settings(settings, locale):
      try c.encode(settings, forKey: .settings)
      try c.encode(locale, forKey: .locale)
    case let .controls(agent, controls, shapes):
      try c.encode(agent, forKey: .agent)
      try c.encode(controls, forKey: .controls)
      try c.encodeIfPresent(shapes, forKey: .shapes)
    case let .sessionHits(seq, hits):
      try c.encode(seq, forKey: .seq)
      try c.encode(hits, forKey: .hits)
    case let .subagent(sessionId, subagentId, rev, running, turns):
      try c.encode(sessionId, forKey: .sessionId)
      try c.encode(subagentId, forKey: .subagentId)
      try c.encode(rev, forKey: .rev)
      try c.encode(running, forKey: .running)
      try c.encode(turns, forKey: .turns)
    case let .files(seq, files):
      try c.encode(seq, forKey: .seq)
      try c.encode(files, forKey: .files)
    case let .diffSource(seq, source):
      try c.encode(seq, forKey: .seq)
      try c.encodeIfPresent(source, forKey: .source)
    case let .imageBlob(seq, blob):
      try c.encode(seq, forKey: .seq)
      try c.encodeIfPresent(blob, forKey: .blob)
    case let .sessionMarkdown(seq, markdown, error):
      try c.encode(seq, forKey: .seq)
      try c.encodeIfPresent(markdown, forKey: .markdown)
      try c.encodeIfPresent(error, forKey: .error)
    case let .projectDir(seq, entries, error):
      try c.encode(seq, forKey: .seq)
      try c.encode(entries, forKey: .entries)
      try c.encodeIfPresent(error, forKey: .error)
    case let .projectStatus(seq, root, marks):
      try c.encode(seq, forKey: .seq)
      try c.encodeIfPresent(root, forKey: .root)
      try c.encode(marks, forKey: .marks)
    case let .projectFile(seq, text, hunks, error):
      try c.encode(seq, forKey: .seq)
      try c.encodeIfPresent(text, forKey: .text)
      try c.encodeIfPresent(hunks, forKey: .hunks)
      try c.encodeIfPresent(error, forKey: .error)
    case let .sessionChanges(seq, files):
      try c.encode(seq, forKey: .seq)
      try c.encode(files, forKey: .files)
    case let .terminalOutput(id, data):
      try c.encode(id, forKey: .id)
      try c.encode(data, forKey: .data)
    case let .terminalState(id, title, cols, rows, replay, exit, error):
      try c.encode(id, forKey: .id)
      try c.encodeIfPresent(title, forKey: .title)
      try c.encodeIfPresent(cols, forKey: .cols)
      try c.encodeIfPresent(rows, forKey: .rows)
      try c.encodeIfPresent(replay, forKey: .replay)
      try c.encodeIfPresent(exit, forKey: .exit)
      try c.encodeIfPresent(error, forKey: .error)
    case let .editTurnResult(requestId, error):
      try c.encode(requestId, forKey: .requestId)
      try c.encodeIfPresent(error, forKey: .error)
    case .appearance(let appearance):
      try c.encode(appearance, forKey: .appearance)
    case .connections(let connections):
      try c.encode(connections, forKey: .connections)
    case .noSession, .unknown:
      break
    }
  }
}
