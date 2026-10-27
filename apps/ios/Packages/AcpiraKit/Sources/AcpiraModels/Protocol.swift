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

/// The settings the engine pushes (settings.ts SettingsView), as the subset the phone reads; a key missing from the
/// push reads as DEFAULT_SETTINGS has it. Not mirrored: the desktop / IDE-only keys (sessionScope, sessionListPosition,
/// font sizes and smoothing, diffMarkers, shareEditorSelection, agentCpuCap, proxy, openFilesWith, desktopLayout,
/// panel*) and the subagent personas (subagents, subagentGroups)
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
/// for tests and scripts (absent: the view's current session); the app always passes them
public enum WebviewMsg: Codable, Hashable, Sendable {
  /// The page is mounted: the engine answers with `init`. patches: the page applies `sessionPatch` (the app sends true)
  case ready(patches: Bool? = nil)
  /// A sessionPatch did not fit the view the page holds: the engine sends the whole view again
  case resync
  /// steer: join the running turn instead of queueing behind it (SessionView.canSteer)
  case send(sessionId: String?, text: String, attachments: [Draft]? = nil, steer: Bool? = nil)
  case stop(sessionId: String?)
  case permission(sessionId: String, blockId: String, optionId: String)
  /// The question card closed: `answers` holds the answered questions only; skip tells the agent to go on without them
  case answer(sessionId: String, blockId: String, answers: QuestionAnswers, skip: Bool? = nil)
  /// Build a reviewed plan: optionId picks the card's option, model the model it is built with
  case buildPlan(sessionId: String, planId: String, optionId: String? = nil, model: ModelPick? = nil)
  case setMode(sessionId: String?, id: String)
  case setConfig(sessionId: String?, configId: String, value: String)
  case selectSession(id: String)
  /// category: file it under that category from the start; cwd: the project folder to start in
  case newSession(agent: AgentId? = nil, category: String? = nil, cwd: String? = nil)
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
  case steerQueued(sessionId: String, id: String)
  case editQueued(sessionId: String, id: String, text: String, retainedAttachments: [Int], attachments: [Draft])
  /// A type this build does not know (decoding only)
  case unknown(String)

  /// The `type` literals this build mirrors: the actions the app can send
  public static let types = [
    "ready", "resync", "send", "stop", "permission", "answer", "buildPlan", "setMode", "setConfig", "selectSession",
    "newSession", "renameSession", "deleteSession", "restoreSession", "pinSession", "markAllRead", "searchSessions",
    "forkSession", "selectAccount", "refreshQuota", "compact", "retry", "takeOverSession", "dequeue", "sendQueued",
    "steerQueued", "editQueued",
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
    case .unknown(let t): t
    }
  }

  private enum Keys: String, CodingKey {
    case type, patches, sessionId, text, attachments, steer, blockId, optionId, answers, skip, planId, model, id, configId
    case value, agent, category, cwd, title, pinned, query, seq, turnIndex, retainedAttachments
  }

  public init(from decoder: Decoder) throws {
    let c = try decoder.container(keyedBy: Keys.self)
    func str(_ k: Keys) throws -> String { try c.decode(String.self, forKey: k) }
    func opt(_ k: Keys) throws -> String? { try c.decodeIfPresent(String.self, forKey: k) }
    switch try str(.type) {
    case "ready": self = .ready(patches: try c.decodeIfPresent(Bool.self, forKey: .patches))
    case "resync": self = .resync
    case "send":
      self = .send(sessionId: try opt(.sessionId), text: try str(.text), attachments: try c.decodeIfPresent([Draft].self, forKey: .attachments),
                   steer: try c.decodeIfPresent(Bool.self, forKey: .steer))
    case "stop": self = .stop(sessionId: try opt(.sessionId))
    case "permission": self = .permission(sessionId: try str(.sessionId), blockId: try str(.blockId), optionId: try str(.optionId))
    case "answer":
      self = .answer(sessionId: try str(.sessionId), blockId: try str(.blockId), answers: try c.decode(QuestionAnswers.self, forKey: .answers),
                     skip: try c.decodeIfPresent(Bool.self, forKey: .skip))
    case "buildPlan":
      self = .buildPlan(sessionId: try str(.sessionId), planId: try str(.planId), optionId: try opt(.optionId),
                        model: try c.decodeIfPresent(ModelPick.self, forKey: .model))
    case "setMode": self = .setMode(sessionId: try opt(.sessionId), id: try str(.id))
    case "setConfig": self = .setConfig(sessionId: try opt(.sessionId), configId: try str(.configId), value: try str(.value))
    case "selectSession": self = .selectSession(id: try str(.id))
    case "newSession": self = .newSession(agent: try opt(.agent), category: try opt(.category), cwd: try opt(.cwd))
    case "renameSession": self = .renameSession(id: try str(.id), title: try str(.title))
    case "deleteSession": self = .deleteSession(id: try str(.id))
    case "restoreSession": self = .restoreSession(id: try str(.id))
    case "pinSession": self = .pinSession(id: try str(.id), pinned: try c.decode(Bool.self, forKey: .pinned))
    case "markAllRead": self = .markAllRead
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
    case "steerQueued": self = .steerQueued(sessionId: try str(.sessionId), id: try str(.id))
    case "editQueued":
      self = .editQueued(sessionId: try str(.sessionId), id: try str(.id), text: try str(.text),
                         retainedAttachments: try c.decode([Int].self, forKey: .retainedAttachments),
                         attachments: try c.decode([Draft].self, forKey: .attachments))
    case let type: self = .unknown(type)
    }
  }

  public func encode(to encoder: Encoder) throws {
    var c = encoder.container(keyedBy: Keys.self)
    try c.encode(type, forKey: .type)
    switch self {
    case .ready(let patches):
      try c.encodeIfPresent(patches, forKey: .patches)
    case let .send(sessionId, text, attachments, steer):
      try c.encodeIfPresent(sessionId, forKey: .sessionId)
      try c.encode(text, forKey: .text)
      try c.encodeIfPresent(attachments, forKey: .attachments)
      try c.encodeIfPresent(steer, forKey: .steer)
    case .stop(let sessionId), .compact(let sessionId), .retry(let sessionId), .takeOverSession(let sessionId):
      try c.encodeIfPresent(sessionId, forKey: .sessionId)
    case let .permission(sessionId, blockId, optionId):
      try c.encode(sessionId, forKey: .sessionId)
      try c.encode(blockId, forKey: .blockId)
      try c.encode(optionId, forKey: .optionId)
    case let .answer(sessionId, blockId, answers, skip):
      try c.encode(sessionId, forKey: .sessionId)
      try c.encode(blockId, forKey: .blockId)
      try c.encode(answers, forKey: .answers)
      try c.encodeIfPresent(skip, forKey: .skip)
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
    case let .newSession(agent, category, cwd):
      try c.encodeIfPresent(agent, forKey: .agent)
      try c.encodeIfPresent(category, forKey: .category)
      try c.encodeIfPresent(cwd, forKey: .cwd)
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
    case let .dequeue(sessionId, id), let .sendQueued(sessionId, id), let .steerQueued(sessionId, id):
      try c.encode(sessionId, forKey: .sessionId)
      try c.encode(id, forKey: .id)
    case let .editQueued(sessionId, id, text, retainedAttachments, attachments):
      try c.encode(sessionId, forKey: .sessionId)
      try c.encode(id, forKey: .id)
      try c.encode(text, forKey: .text)
      try c.encode(retainedAttachments, forKey: .retainedAttachments)
      try c.encode(attachments, forKey: .attachments)
    case .resync, .markAllRead, .unknown:
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
  /// A push this build does not mirror; carries the type for diagnostics
  case unknown(String)

  /// The `type` literals this build mirrors
  public static let types = [
    "init", "agents", "sessions", "categories", "noSession", "session", "sessionPatch", "accounts", "accountActions",
    "hidden", "settings", "controls", "sessionHits",
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
    case .unknown(let t): t
    }
  }

  private enum Keys: String, CodingKey {
    case type, state, agents, sessions, categories, session, patch, accounts, actions, hidden, settings, locale, agent
    case controls, shapes, seq, hits
  }

  public init(from decoder: Decoder) throws {
    let c = try decoder.container(keyedBy: Keys.self)
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
    case .noSession, .unknown:
      break
    }
  }
}
