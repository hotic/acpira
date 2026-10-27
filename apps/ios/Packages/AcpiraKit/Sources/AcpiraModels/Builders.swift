import Foundation

// Memberwise initializers for the values the app builds itself (a sent prompt, a new session, the local demo reply)
// rather than decodes. The structs declare a custom or synthesized Codable conformance, which leaves their memberwise
// initializers internal or absent, so they are spelled out here.

extension AgentTurn {
  public init(blocks: [AgentBlock] = [], startedAt: Double? = nil, endedAt: Double? = nil, stop: TurnStop? = nil,
              activity: TurnActivity? = nil, usage: TurnUsage? = nil, error: TurnError? = nil) {
    self.blocks = blocks; self.startedAt = startedAt; self.endedAt = endedAt; self.stop = stop
    self.activity = activity; self.usage = usage; self.error = error
  }
}

extension TextBlock {
  public init(markdown: String, id: String? = nil, streaming: Bool? = nil) {
    self.markdown = markdown; self.id = id; self.streaming = streaming
  }
}

extension ThoughtBlock {
  public init(text: String, startedAt: Double? = nil, durationSec: Double? = nil) {
    self.text = text; self.startedAt = startedAt; self.durationSec = durationSec
  }
}

extension ToolCallBlock {
  public init(id: String, kind: ToolKind, verb: String, target: String? = nil, targetMono: Bool? = nil,
              status: ToolStatus, meta: String? = nil) {
    self.id = id; self.kind = kind; self.verb = verb; self.target = target; self.targetMono = targetMono
    self.status = status; self.meta = meta
  }
}

extension QueuedPrompt {
  public init(id: String, text: String, attachments: [Attachment]? = nil) {
    self.id = id; self.text = text; self.attachments = attachments
  }
}

extension SessionSummary {
  public init(id: String, title: String, agent: AgentId, cwd: String, updatedAt: String, accountId: String? = nil,
              pinned: Bool? = nil, state: SessionState? = nil, unread: Bool? = nil) {
    self.id = id; self.title = title; self.agent = agent; self.cwd = cwd; self.updatedAt = updatedAt
    self.accountId = accountId; self.pinned = pinned; self.state = state; self.unread = unread
  }
}

extension SessionView {
  public init(id: String, agent: AgentId, title: String, cwd: String, accountId: String? = nil,
              status: SessionStatus = .ready, turns: [Turn] = [], running: Bool = false,
              controls: SessionControls = SessionControls(), usage: Usage? = nil, commands: [SlashCommand] = [],
              queued: [QueuedPrompt]? = nil, canSteer: Bool? = nil, createdAt: String = "", updatedAt: String = "") {
    self.id = id; self.agent = agent; self.title = title; self.cwd = cwd; self.accountId = accountId
    self.status = status; self.turns = turns; self.running = running; self.controls = controls; self.usage = usage
    self.commands = commands; self.queued = queued; self.canSteer = canSteer
    self.createdAt = createdAt; self.updatedAt = updatedAt
  }
}

extension Date {
  /// JavaScript's toISOString: `2026-10-09T09:05:18.000Z`
  public var iso: String { formatted(Date.ISO8601FormatStyle(includingFractionalSeconds: true)) }
}
