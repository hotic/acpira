import AcpiraModels
import Foundation
import Testing

// Pins the Swift mirrors to the TypeScript contract they copy. Runs wherever the repository is readable (`swift test`
// on the Mac); a test process that cannot see the source tree skips it instead of failing.
//
// For every mirrored message: its `type` literal is a member of the TypeScript union, a sample with every field set
// encodes to exactly that member's field names, dropping a field TypeScript marks optional still decodes, and dropping a
// required one does not. Structs are compared field by field (names and optionality, read by reflection).
struct ContractTests {
  /// The repository root, seven levels above this file
  static let root = (0..<7).reduce(URL(filePath: #filePath)) { url, _ in url.deletingLastPathComponent() }
  static let sidecarSource = root.appending(path: "packages/shared/sidecar.ts")
  static var readable: Bool { FileManager.default.fileExists(atPath: sidecarSource.path) }

  static func source(_ file: String) throws -> String {
    TS.stripComments(try String(contentsOf: root.appending(path: "packages/shared/\(file)"), encoding: .utf8))
  }

  @Test(.enabled(if: readable))
  func protocolVersionMatchesTypeScript() throws {
    let source = try String(contentsOf: Self.sidecarSource, encoding: .utf8)
    let match = try #require(source.firstMatch(of: /SIDECAR_PROTOCOL_VERSION = (\d+)/))
    #expect(Int(match.1) == SidecarProtocol.version)
  }

  // MARK: Unions

  @Test(.enabled(if: readable))
  func shellEnvelopes() throws {
    try checkUnion("ShellMsg", in: Self.source("sidecar.ts"), mirrored: ShellMsg.types, samples: ContractSamples.shell) {
      if case .unknown = $0 { false } else { true }
    }
  }

  @Test(.enabled(if: readable))
  func sidecarEnvelopes() throws {
    try checkUnion("SidecarMsg", in: Self.source("sidecar.ts"), mirrored: SidecarMsg.types,
                   samples: try ContractSamples.sidecar.map { try decode($0) as SidecarMsg }) {
      if case .unknown = $0 { false } else { true }
    }
  }

  @Test(.enabled(if: readable))
  func platformRequests() throws {
    try checkUnion("PlatformRequest", tag: "method", in: Self.source("sidecar.ts"), mirrored: PlatformRequest.methods,
                   samples: ContractSamples.platformRequests) {
      if case .unknown = $0 { false } else { true }
    }
  }

  @Test(.enabled(if: readable))
  func webviewActions() throws {
    let categoryOp = try need(TS.alias(Self.source("transcript.ts"), "CategoryOp"))
    try checkUnion("WebviewMsg", in: Self.source("protocol.ts"), mirrored: WebviewMsg.types, samples: ContractSamples.actions,
                   intersections: ["categoryOp": (categoryOp, "op")]) {
      if case .unknown = $0 { false } else { true }
    }
  }

  @Test(.enabled(if: readable))
  func categoryOps() throws {
    try checkUnion("CategoryOp", tag: "op", in: Self.source("transcript.ts"), mirrored: CategoryOp.ops, samples: ContractSamples.categoryOps) {
      if case .unknown = $0 { false } else { true }
    }
  }

  /// AgentBlock is a union of named interfaces; NoticeBlock reads its revision, category and actions leniently
  @Test(.enabled(if: readable))
  func agentBlocks() throws {
    let transcript = try Self.source("transcript.ts")
    let members = try need(TS.interfaceUnion(transcript, "AgentBlock"), "AgentBlock")
    try checkMembers("AgentBlock", members: members, mirrored: AgentBlock.types,
                     samples: try ContractSamples.blocks.map { try decode($0) as AgentBlock },
                     lenient: ["notice": ["revision", "category", "actions"]]) {
      if case .unknown = $0 { false } else { true }
    }
  }

  @Test(.enabled(if: readable))
  func hostPushes() throws {
    try checkUnion("HostMsg", in: Self.source("protocol.ts"), mirrored: HostMsg.types,
                   samples: try ContractSamples.pushes.map { try decode($0) as HostMsg }) {
      if case .unknown = $0 { false } else { true }
    }
  }

  @Test(.enabled(if: readable))
  func drafts() throws {
    try checkUnion("Draft", tag: "kind", in: Self.source("transcript.ts"), mirrored: Draft.kinds, samples: ContractSamples.drafts) {
      if case .unknown = $0 { false } else { true }
    }
  }

  // MARK: Structs

  @Test(.enabled(if: readable))
  func envelopePayloads() throws {
    let sidecar = try Self.source("sidecar.ts")
    let shell = try need(TS.members(try need(TS.alias(sidecar, "ShellMsg")))["hello"])
    let hello = try need(ContractSamples.shell.compactMap { if case .hello(let h) = $0 { h } else { nil } }.first)
    try checkStruct(hello.client, TS.inline(try need(shell.first { $0.name == "client" }).type))
    try checkStruct(hello.env, try need(TS.interface(sidecar, "ShellEnv")))
    let ok = try need(TS.members(try need(TS.alias(sidecar, "SidecarMsg")))["helloOk"])
    let helloOk: SidecarMsg = try decode(ContractSamples.sidecar[0])
    guard case .helloOk(let payload) = helloOk else { Issue.record("not helloOk"); return }
    try checkStruct(payload, ok)
    try checkStruct(payload.sidecar, TS.inline(try need(ok.first { $0.name == "sidecar" }).type))
    let toast = try need(TS.members(try need(TS.alias(sidecar, "PlatformRequest")), tag: "method")["toast"])
    let open = PlatformRequest.ToastOpen(label: "Open", path: "/w/a.md")
    try checkStruct(open, TS.inline(try need(toast.first { $0.name == "open" }).type))
  }

  @Test(.enabled(if: readable))
  func protocolPayloads() throws {
    let proto = try Self.source("protocol.ts"), transcript = try Self.source("transcript.ts")
    let actions = TS.members(try need(TS.alias(proto, "WebviewMsg")))
    let build = try need(actions["buildPlan"])
    try checkStruct(ModelPick(configId: "model", value: "opus"), TS.inline(try need(build.first { $0.name == "model" }).type))
    try checkStruct(TextContext(sourceId: "browser", sourceName: "Browser"), try need(TS.interface(transcript, "TextContext")))

    guard case .initState(let state) = try decode(ContractSamples.pushes[0]) as HostMsg else { Issue.record("not init"); return }
    try checkStruct(state, try need(TS.interface(proto, "InitState")), leftOut: ["agentInstalls"])
    try checkStruct(state.settings, try need(TS.interface(Self.source("settings.ts"), "SettingsView")), leftOut: [
      "sessionScope", "sessionListPosition", "uiFontSize", "codeFontSize", "diffMarkers", "fontSmoothing", "shareEditorSelection",
      "agentCpuCap", "proxy", "openFilesWith", "desktopLayout", "panelScope", "panelFiles", "panelTerminal", "panelBrowser",
    ])
    let subagents = try Self.source("subagents.ts")
    try checkStruct(try need(state.settings.subagents.first), try need(TS.interface(subagents, "SubagentPersona")))
    try checkStruct(try need(state.settings.subagentGroups.first), try need(TS.interface(subagents, "SubagentGroup")))
    let categories = try need(state.categories)
    try checkStruct(categories, try need(TS.interface(transcript, "SessionCategories")))
    try checkStruct(try need(categories.categories.first), try need(TS.interface(transcript, "SessionCategory")))
    try checkStruct(try need(categories.sections?.first), try need(TS.interface(transcript, "SessionSection")))
    try checkStruct(try need(state.accountActions?.first), try need(TS.interface(proto, "AccountAction")))

    guard case .sessionHits(_, let hits) = try decode(ContractSamples.pushes[12]) as HostMsg else { Issue.record("not sessionHits"); return }
    try checkStruct(try need(hits.first), try need(TS.interface(proto, "SessionHit")))
    guard case .sessionPatch(let patch) = try decode(ContractSamples.pushes[6]) as HostMsg else { Issue.record("not sessionPatch"); return }
    try checkStruct(patch, try need(TS.interface(Self.source("sessionPatch.ts"), "SessionPatch")))
  }

  /// The transcript structs the messages carry: SessionView, SessionSummary and AgentInfo whole, AccountInfo as a subset
  /// (it reads addedAt leniently)
  @Test(.enabled(if: readable))
  func transcriptFieldsExist() throws {
    let transcript = try Self.source("transcript.ts")
    guard case .initState(let state) = try decode(ContractSamples.pushes[0]) as HostMsg else { Issue.record("not init"); return }
    try checkStruct(try need(state.active), try need(TS.interface(transcript, "SessionView")))
    try checkStruct(try need(state.sessions.first), try need(TS.interface(transcript, "SessionSummary")))
    try checkStruct(try need(state.accounts.first), try need(TS.interface(transcript, "AccountInfo")), subset: true)
    try checkStruct(try need(state.agents.first), try need(TS.interface(transcript, "AgentInfo")))
  }

  /// The payloads of the parity actions and pushes
  @Test(.enabled(if: readable))
  func parityPayloads() throws {
    let proto = try Self.source("protocol.ts"), transcript = try Self.source("transcript.ts")
    try checkStruct(ContractSamples.edit, try need(TS.interface(proto, "EditTurnRequest")))
    try checkStruct(ContractSamples.edit.settings, try need(TS.interface(transcript, "TurnSettings")))
    let pushes = try ContractSamples.pushes.map { try decode($0) as HostMsg }
    for push in pushes {
      switch push {
      case .files(_, let files): try checkStruct(try need(files.first), try need(TS.interface(proto, "FileHit")))
      case .diffSource(_, let source):
        try checkStruct(try need(source), try need(TS.interface(transcript, "FullDiffSource")), leftOut: ["omitted"])
      case .projectDir(_, let entries, _): try checkStruct(try need(entries.first), try need(TS.interface(proto, "ProjectEntry")))
      case .projectFile(_, _, let hunks, _): try checkStruct(try need(hunks?.first), try need(TS.interface(proto, "ProjectHunk")))
      case .sessionChanges(_, let files): try checkStruct(try need(files.first), try need(TS.interface(proto, "ProjectChange")))
      case .terminalState(_, _, _, _, _, let exit, _): try checkStruct(try need(exit), try need(TS.interface(proto, "TerminalExit")))
      default: break
      }
    }
  }

  /// A session view with every field set, down to the goal, the subagents and both kinds of turn
  @Test(.enabled(if: readable))
  func sessionViewParts() throws {
    let transcript = try Self.source("transcript.ts"), subagents = try Self.source("subagents.ts")
    let view: SessionView = try decode(ContractSamples.richView)
    func inline(_ fields: [TSField], _ name: String) throws -> [TSField] {
      try TS.inline(try need(fields.first { $0.name == name }, name).type)
    }
    try checkStruct(view, try need(TS.interface(transcript, "SessionView")))
    try checkStruct(try need(view.external), try need(TS.interface(transcript, "ExternalSessionInfo")))
    let auth = try need(view.authMethods?.first), authFields = try need(TS.interface(transcript, "AuthMethodInfo"))
    try checkStruct(auth, authFields)
    try checkStruct(try need(auth.terminal), try inline(authFields, "terminal"))
    try checkStruct(try need(view.usage), try need(TS.interface(transcript, "Usage")))
    try checkStruct(try need(view.goal), try need(TS.interface(transcript, "SessionGoal")))
    try checkStruct(try need(view.forkedFrom), try need(TS.interface(transcript, "ForkOrigin")))

    let summary = try need(view.subagents?.first), summaryFields = try need(TS.interface(subagents, "SubagentSummary"))
    try checkStruct(summary, summaryFields)
    try checkStruct(summary.controls, try inline(summaryFields, "controls"))
    try checkStruct(try need(summary.usage), try inline(summaryFields, "usage"))
    try checkStruct(summary.peer, try inline(summaryFields, "peer"))
    try checkStruct(try need(summary.harness), try need(TS.interface(subagents, "SubagentHarness")))

    guard case .user(let user) = view.turns.first, case .agent(let agent) = view.turns.last else {
      Issue.record("not a user turn then an agent turn"); return
    }
    try checkStruct(user, try need(TS.interface(transcript, "UserTurn")), leftOut: ["role"])
    try checkStruct(try need(user.settings), try need(TS.interface(transcript, "TurnSettings")))
    let agentFields = try need(TS.interface(transcript, "AgentTurn"))
    try checkStruct(agent, agentFields, leftOut: ["role"])
    try checkStruct(try need(agent.usage), try need(TS.interface(transcript, "TurnUsage")))
    try checkStruct(try need(agent.error), try need(TS.interface(transcript, "TurnError")))
    try checkStruct(try need(agent.command), try inline(agentFields, "command"))

    let block: AgentBlock = try decode(ContractSamples.blocks[2])
    let task = try need(block.toolCall?.asyncTask), taskFields = try need(TS.interface(transcript, "AsyncTaskInfo"))
    try checkStruct(task, taskFields, lenient: ["canStop"])
    try checkStruct(try need(task.usage), try inline(taskFields, "usage"))
  }

  /// The string enums added for the parity work name exactly their TypeScript literals (`unknown` is the app's own
  /// fallback)
  @Test(.enabled(if: readable))
  func stringUnions() throws {
    let transcript = try Self.source("transcript.ts"), proto = try Self.source("protocol.ts")
    let subagents = try Self.source("subagents.ts")
    func check<E: TolerantEnum & CaseIterable>(_: E.Type, _ union: String?, _ name: String) throws {
      let literals = TS.literals(try need(union, name))
      #expect(Set(E.allCases.filter { $0 != E.unknownCase }.map(\.rawValue)) == literals, "\(name) is \(literals.sorted())")
    }
    func field(_ source: String, _ interface: String, _ name: String) -> String? {
      TS.interface(source, interface)?.first { $0.name == name }?.type
    }
    try check(GoalStatus.self, TS.alias(transcript, "GoalStatus"), "GoalStatus")
    try check(GoalAction.self, TS.alias(transcript, "GoalAction"), "GoalAction")
    try check(GoalEvent.self, TS.alias(transcript, "GoalEvent"), "GoalEvent")
    try check(AddAccountVia.self, TS.alias(proto, "AddAccountVia"), "AddAccountVia")
    try check(ProjectBase.self, TS.alias(proto, "ProjectBase"), "ProjectBase")
    try check(GitMark.self, TS.alias(proto, "GitMark"), "GitMark")
    try check(EditTurnRequest.Intent.self, field(proto, "EditTurnRequest", "intent"), "EditTurnRequest.intent")
    try check(ExternalSessionInfo.State.self, field(transcript, "ExternalSessionInfo", "state"), "ExternalSessionInfo.state")
    try check(SubagentState.self, TS.alias(subagents, "SubagentState"), "SubagentState")
    try check(SubagentVisibility.self, TS.alias(subagents, "SubagentVisibility"), "SubagentVisibility")
    try check(SubagentSummary.StateSource.self, field(subagents, "SubagentSummary", "stateSource"), "SubagentSummary.stateSource")
    try check(RelayMode.self, TS.alias(subagents, "RelayMode"), "RelayMode")
    let reasons = TS.literals(try need(TS.alias(proto, "ProjectError"), "ProjectError"))
    #expect(Set(ProjectError.reasons.map(\.rawValue)) == reasons, "ProjectError is \(reasons.sorted())")
  }

  /// The turns the engine's golden suite records (test/fixtures/engine-*.json, the input the webview suites render) hold
  /// no block this build does not know, and encode back to the same JSON: the Turn mirror misses no field the engine
  /// writes there
  @Test(.enabled(if: readable))
  func goldenTurnsRoundTrip() throws {
    var turns: [(String, [String: Any])] = []
    for file in ["engine-builtin.json", "engine-normalize.json"] {
      let data = try Data(contentsOf: Self.root.appending(path: "test/fixtures/\(file)"))
      for entry in try #require(JSONSerialization.jsonObject(with: data) as? [[String: Any]]) {
        let name = "\(file) \(entry["name"] ?? "")"
        let lists = (entry["turns"].map { [$0] } ?? []) + ((entry["steps"] as? [[String: Any]]) ?? []).compactMap { $0["turns"] }
        for case let list as [[String: Any]] in lists { turns += list.map { (name, $0) } }
      }
    }
    #expect(!turns.isEmpty)
    for (name, object) in turns {
      let turn = try JSONDecoder().decode(Turn.self, from: JSONObject.data(object))
      if case .agent(let agent) = turn {
        for case .unknown(let type) in agent.blocks { Issue.record("\(name): unknown block \(type)") }
      }
      let encoded = try JSONObject.of(turn)
      #expect(JSONObject.same(encoded, object), "\(name): \(object) re-encodes as \(encoded)")
    }
  }

  // MARK: Helpers

  struct Missing: Error, CustomStringConvertible { let description: String }

  /// `value`, or a failure naming what is missing (`#require` does not nest)
  func need<T>(_ value: T?, _ what: String = "a value", sourceLocation: SourceLocation = #_sourceLocation) throws -> T {
    guard let value else { throw Missing(description: "\(what) not found at \(sourceLocation)") }
    return value
  }

  func decode<T: Decodable>(_ json: String) throws -> T {
    try JSONDecoder().decode(T.self, from: Data(json.utf8))
  }

  /// `samples` cover exactly the `mirrored` literals of union `name`; each encodes to its member's fields, decodes back
  /// equal, still decodes (to a known case) without any one optional field, and fails without any one required field.
  /// `intersections`: members written `{ type: 'x' } & Other`, which also carry the fields of the `Other` member their
  /// sample picks by that union's tag
  func checkUnion<T: Codable & Equatable>(
    _ name: String, tag: String = "type", in source: String, mirrored: [String], samples: [T],
    intersections: [String: (union: String, tag: String)] = [:], known: (T) -> Bool
  ) throws {
    let members = TS.members(try #require(TS.alias(source, name), "\(name) not found"), tag: tag)
    try checkMembers(name, tag: tag, members: members, mirrored: mirrored, samples: samples, intersections: intersections, known: known)
  }

  /// checkUnion against members already read; `lenient` names, per literal, required fields the Swift side still
  /// decodes without
  func checkMembers<T: Codable & Equatable>(
    _ name: String, tag: String = "type", members: [String: [TSField]], mirrored: [String], samples: [T],
    intersections: [String: (union: String, tag: String)] = [:], lenient: [String: Set<String>] = [:], known: (T) -> Bool
  ) throws {
    for literal in mirrored {
      #expect(members[literal] != nil, "\(name) has no member '\(literal)'")
    }
    var covered: Set<String> = []
    for sample in samples {
      let object = try JSONObject.of(sample)
      let literal = try #require(object[tag] as? String)
      covered.insert(literal)
      guard var fields = members[literal] else { continue }
      if let (union, inner) = intersections[literal] {
        let picked = try #require(object[inner] as? String, "\(name) '\(literal)' has no '\(inner)'")
        let more = try #require(TS.members(union, tag: inner)[picked], "\(name) '\(literal)': no '\(inner)' member '\(picked)'")
        fields += [TSField(name: inner, optional: false, type: "'\(picked)'")] + more
      }
      #expect(Set(object.keys).subtracting([tag]) == Set(fields.map(\.name)), "\(name) '\(literal)' encodes other fields")
      #expect(try JSONDecoder().decode(T.self, from: JSONObject.data(object)) == sample, "\(name) '\(literal)' does not round-trip")
      for field in fields {
        var without = object
        without[field.name] = nil
        let decoded = try? JSONDecoder().decode(T.self, from: JSONObject.data(without))
        if field.optional || lenient[literal]?.contains(field.name) == true {
          #expect(decoded.map(known) == true, "\(name) '\(literal)' needs optional '\(field.name)'")
        } else {
          #expect(decoded == nil, "\(name) '\(literal)' takes no '\(field.name)' though TypeScript requires it")
        }
      }
    }
    #expect(covered == Set(mirrored), "\(name) samples cover \(covered.sorted()), not \(mirrored.sorted())")
  }

  /// The struct's stored properties are the TypeScript fields (minus `leftOut`) with the same optionality, but for
  /// `lenient` ones that Swift reads as optional though TypeScript requires them; with `subset`, only that every
  /// property names a TypeScript field
  func checkStruct(_ value: Any, _ fields: [TSField], leftOut: Set<String> = [], lenient: Set<String> = [], subset: Bool = false) throws {
    let swift = Dictionary(uniqueKeysWithValues: Mirror(reflecting: value).children.compactMap { child in
      child.label.map { ($0, Mirror(reflecting: child.value).displayStyle == .optional) }
    })
    let ts = Dictionary(uniqueKeysWithValues: fields.map { ($0.name, $0.optional) })
    let type = String(describing: Swift.type(of: value))
    #expect(Set(swift.keys).isSubset(of: ts.keys), "\(type) fields missing from TypeScript: \(Set(swift.keys).subtracting(ts.keys).sorted())")
    #expect(leftOut.isSubset(of: ts.keys), "\(type) leaves out fields TypeScript does not have: \(leftOut.subtracting(ts.keys).sorted())")
    if subset { return }
    #expect(Set(swift.keys) == Set(ts.keys).subtracting(leftOut), "\(type) does not mirror \(Set(ts.keys).subtracting(leftOut).subtracting(swift.keys).sorted())")
    for (name, optional) in swift where ts[name] != nil && !lenient.contains(name) {
      #expect(ts[name] == optional, "\(type).\(name) is \(optional ? "" : "not ")optional, TypeScript says otherwise")
    }
  }
}

/// A JSON object as JSONSerialization reads it
enum JSONObject {
  static func of<T: Encodable>(_ value: T) throws -> [String: Any] {
    try #require(JSONSerialization.jsonObject(with: JSONEncoder().encode(value)) as? [String: Any])
  }
  static func parse(_ json: String) throws -> [String: Any] {
    try #require(JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
  }
  static func data(_ object: [String: Any]) throws -> Data {
    try JSONSerialization.data(withJSONObject: object)
  }
  /// The same JSON, compared as values (key order and number spelling aside)
  static func same(_ a: [String: Any], _ b: [String: Any]) -> Bool {
    NSDictionary(dictionary: a).isEqual(to: b)
  }
}

/// A field of a TypeScript object type: its name, whether it is optional (`?:`), and its type as written
struct TSField: Hashable {
  let name: String
  let optional: Bool
  let type: String
}

/// Just enough of a TypeScript reader for the contract files: object types, unions of them, interfaces
enum TS {
  /// `//` comments removed (the contract files hold no `//` inside a string in the parts read here)
  static func stripComments(_ source: String) -> String {
    source.split(separator: "\n", omittingEmptySubsequences: false)
      .map { line in line.range(of: "//").map { String(line[..<$0.lowerBound]) } ?? String(line) }
      .joined(separator: "\n")
  }

  /// The text of `export type <name> = …` up to its closing `;`
  static func alias(_ source: String, _ name: String) -> String? {
    guard let head = source.range(of: "export type \(name) =") else { return nil }
    var depth = 0
    var text = ""
    for ch in source[head.upperBound...] {
      if "{([".contains(ch) { depth += 1 }
      if "})]".contains(ch) { depth -= 1 }
      if ch == ";" && depth == 0 { return text }
      text.append(ch)
    }
    return nil
  }

  /// The fields of `export interface <name> { … }`
  static func interface(_ source: String, _ name: String) -> [TSField]? {
    guard let head = source.range(of: "export interface \(name) {") else { return nil }
    let chars = Array(source[source.index(before: head.upperBound)...])
    guard let end = close(chars, 0) else { return nil }
    return fields(String(chars[1..<end]))
  }

  /// The members of a union of named interfaces (`export type X = A | B`), by the literal of their `tag` field (left
  /// out of the fields)
  static func interfaceUnion(_ source: String, _ name: String, tag: String = "type") -> [String: [TSField]]? {
    guard let union = alias(source, name) else { return nil }
    var out: [String: [TSField]] = [:]
    for member in union.split(separator: "|") {
      guard let body = interface(source, member.trimmingCharacters(in: .whitespacesAndNewlines)),
            let t = body.first(where: { $0.name == tag }), let literal = t.type.firstMatch(of: /^'([^']*)'$/) else { return nil }
      out[String(literal.1)] = body.filter { $0.name != tag }
    }
    return out
  }

  /// The string literals of a union of them (`'a' | 'b'`)
  static func literals(_ union: String) -> Set<String> {
    Set(union.matches(of: /'([^']*)'/).map { String($0.1) })
  }

  /// The fields of an inline object type (`{ a: string; b?: number }`)
  static func inline(_ type: String) throws -> [TSField] {
    let t = type.trimmingCharacters(in: .whitespacesAndNewlines)
    try #require(t.first == "{" && t.last == "}", "not an object type: \(t)")
    return fields(String(t.dropFirst().dropLast()))
  }

  /// The object members of a union, by the literal of their `tag` field (left out of the fields)
  static func members(_ union: String, tag: String = "type") -> [String: [TSField]] {
    let chars = Array(union)
    var out: [String: [TSField]] = [:]
    var i = 0
    while i < chars.count {
      if chars[i] == "{", let end = close(chars, i) {
        let body = fields(String(chars[(i + 1)..<end]))
        if let t = body.first(where: { $0.name == tag }), let literal = t.type.firstMatch(of: /^'([^']*)'$/) {
          out[String(literal.1)] = body.filter { $0.name != tag }
          i = end + 1
          continue
        }
      }
      i += 1
    }
    return out
  }

  /// `a: string; b?: { c: number }` split at the top level
  static func fields(_ body: String) -> [TSField] {
    var parts: [String] = [], current = "", depth = 0
    for ch in body {
      if "{([".contains(ch) { depth += 1 }
      if "})]".contains(ch) { depth -= 1 }
      if ch == ";" && depth == 0 {
        parts.append(current)
        current = ""
      } else {
        current.append(ch)
      }
    }
    parts.append(current)
    return parts.compactMap { part in
      let p = part.trimmingCharacters(in: .whitespacesAndNewlines)
      guard let m = p.firstMatch(of: /^(\w+)(\??):\s*([\s\S]*)$/) else { return nil }
      return TSField(name: String(m.1), optional: m.2 == "?", type: String(m.3).trimmingCharacters(in: .whitespacesAndNewlines))
    }
  }

  /// The index of the bracket closing the `{` at `open`
  static func close(_ chars: [Character], _ open: Int) -> Int? {
    var depth = 0
    for i in open..<chars.count {
      if chars[i] == "{" { depth += 1 }
      if chars[i] == "}" {
        depth -= 1
        if depth == 0 { return i }
      }
    }
    return nil
  }
}

/// One sample per mirrored member with every field set
enum ContractSamples {
  static let view = #"{"id":"s1","agent":"claude","title":"t","cwd":"/w","status":"ready","turns":[],"running":false,"rev":2,"controls":{"modes":[],"options":[]},"commands":[],"createdAt":"2026-10-27T19:33:34.971Z","updatedAt":"2026-10-27T19:33:34.971Z"}"#
  static let settings = #"{"language":"auto","locale":"en","defaultAgent":"claude","agentOrder":["claude"],"disabledAgents":["pi"],"autoCompact":true,"compactAtTokens":300000,"hiddenOptions":{"claude":{"model":["haiku"]}},"accountSwitch":"earliestReset","theme":"dark","steerQueued":true,"notifySound":false,"notifySystem":true,"planAutoApprove":["claude"],"subagents":[{"id":"review","name":"Review","agent":"codex","model":"gpt-6","effort":"high","mode":"consult","when":"before a merge","brief":"look for bugs","enabled":true,"duty":"reviewer","changes":true}],"subagentGroups":[{"id":"panel","name":"Panel","members":["review"],"when":"a second opinion","enabled":true}]}"#

  static let shell: [ShellMsg] = [
    .hello(Hello(requestId: "h1",
                 client: ClientInfo(name: "acpira-ios", version: "0.1.0", capabilities: ["toast", "notify"], sshConnection: "10.0.0.2 52502 10.0.0.1 22", role: "peer"),
                 env: ShellEnv(cwd: "/w", hostLanguage: "zh-CN", blobBase: "http://127.0.0.1:7357/blobs", projects: true),
                 settings: ["defaultAgent": "claude", "appearance.motion": "none"])),
    .attachView(viewId: "phone", host: .sidebar, initial: .empty, blobBase: "http://127.0.0.1:7357/blobs"),
    .detachView(viewId: "phone"),
    .webviewMessage(viewId: "phone", message: .ready(patches: true)),
    .platformResponse(requestId: "p1", result: ["ok": true], error: "not supported"),
    .shutdown,
  ]

  static let sidecar = [
    #"{"type":"helloOk","requestId":"h1","protocolVersion":1,"sidecar":{"version":"1.11.0","pid":42},"sessionsDir":"/Users/me/.acpira/sessions","features":["preview","projects"],"safeMode":true}"#,
    #"{"type":"helloReject","requestId":"h1","protocolVersion":1,"reason":"protocol version 2 is not 1"}"#,
    #"{"type":"hostMessage","viewId":"phone","message":{"type":"noSession"}}"#,
    #"{"type":"platformRequest","requestId":"p1","request":{"method":"toast","level":"info","text":"hi"}}"#,
    #"{"type":"shutdownOk"}"#,
  ]

  static let platformRequests: [PlatformRequest] = [
    .toast(level: "info", text: "Exported", open: .init(label: "Open", path: "/w/a.md")),
    .notify(sessionId: "s1", title: "Claude Code", body: "Waiting for approval", open: "Open", sound: true, system: false),
  ]

  static let drafts: [Draft] = [
    .image(mimeType: "image/png", data: "iVBORw0KGgo=", name: "shot.png"),
    .text(name: "notes.txt", text: "hi", context: TextContext(sourceId: "browser", sourceName: "Browser", uri: "http://localhost:3000", comment: "this")),
    .file(uri: "file:///w/a.pdf", name: "a.pdf", data: "JVBERi0="),
    .selection(uri: "file:///w/a.ts", name: "a.ts", startLine: 1, endLine: 3, text: "let a"),
    .quote(text: "the agent said", comment: "why?"),
  ]

  static let actions: [WebviewMsg] = [
    .ready(patches: true),
    .resync,
    .send(sessionId: "s1", text: "hi", attachments: [drafts[0]], steer: true, clientId: "c1"),
    .stop(sessionId: "s1", clientId: "c2"),
    .permission(sessionId: "s1", blockId: "b1", optionId: "allow", clientId: "c3"),
    .answer(sessionId: "s1", blockId: "q1", answers: ["a": .one("notes"), "b": .many(["docs"])], skip: false, clientId: "c4"),
    .buildPlan(sessionId: "s1", planId: "p1", optionId: "build", model: ModelPick(configId: "model", value: "opus")),
    .setMode(sessionId: "s1", id: "plan"),
    .setConfig(sessionId: "s1", configId: "effort", value: "high"),
    .selectSession(id: "s1"),
    .newSession(agent: "claude", category: "c1", cwd: "/w", chat: true),
    .renameSession(id: "s1", title: "Renamed"),
    .deleteSession(id: "s1"),
    .restoreSession(id: "s1"),
    .pinSession(id: "s1", pinned: true),
    .markAllRead,
    .searchSessions(query: "zebracorn", seq: 3),
    .forkSession(sessionId: "s1", turnIndex: 1, agent: "codex"),
    .selectAccount(sessionId: "s1", id: "acc-1"),
    .refreshQuota(agent: "claude"),
    .compact(sessionId: "s1"),
    .retry(sessionId: "s1"),
    .takeOverSession(sessionId: "s1"),
    .dequeue(sessionId: "s1", id: "q1"),
    .sendQueued(sessionId: "s1", id: "q1"),
    .steerQueued(sessionId: "s1", id: "q1", clientId: "c5"),
    .editQueued(sessionId: "s1", id: "q1", text: "edited", retainedAttachments: [0], attachments: [drafts[2]]),
    .categoryOp(op: .reorderProject(cwd: "/w/a", before: "/w/b"), file: "s1"),
    .moveSession(id: "s1", cwd: "/w2", category: "c1"),
    .sessionMarkdown(id: "s1", seq: 4),
    .addAccount(agent: "codex", via: .login),
    .removeAccount(id: "acc-1"),
    .unlockCredentials(agent: "claude"),
    .login(sessionId: "s1", methodId: "chatgpt"),
    .reconnect(sessionId: "s1"),
    .retryTurn(sessionId: "s1"),
    .editTurn(requestId: "e1", edit: edit),
    .observeSubagent(sessionId: "s1", subagentId: "n1"),
    .unobserveSubagent(sessionId: "s1", subagentId: "n1"),
    .cancelSubagent(sessionId: "s1", subagentId: "n1"),
    .stopAsyncTask(sessionId: "s1", taskId: "t1"),
    .goal(sessionId: "s1", action: .set, objective: "ship it"),
    .searchFiles(query: "app", seq: 5),
    .setSetting(key: "theme", value: "dark"),
    .controls(agent: "claude", fresh: true),
    .projectDir(sessionId: "s1", dir: "src", seq: 6),
    .projectStatus(sessionId: "s1", seq: 7),
    .projectFile(sessionId: "s1", path: "src/a.ts", base: .head, seq: 8),
    .sessionChanges(sessionId: "s1", seq: 9),
    .diffSource(sessionId: "s1", toolCallId: "call-1", nth: 0, subagentId: "n1", seq: 10),
    .imageBlob(sessionId: "s1", path: "/w/shot.png", seq: 11),
    .openPlan(sessionId: "s1", planId: "p1"),
    .terminalOpen(id: "T", cwd: "/w", cols: 100, rows: 30),
    .terminalInput(id: "T", data: "ls\r", binary: false),
    .terminalResize(id: "T", cols: 120, rows: 40),
    .terminalAck(id: "T", chars: 4096),
    .terminalClose(id: "T"),
    .terminalKill(id: "T"),
  ]

  static let edit = EditTurnRequest(
    sessionId: "s1", turnIndex: 0, turnCount: 2, originalText: "fail", turnId: "u1", text: "hello again", retainedAttachments: [0],
    attachments: [drafts[4]], settings: TurnSettings(modeId: "agent", config: ["model": "opus"]), intent: .continue)

  static let categoryOps: [CategoryOp] = [
    .addProject(cwd: "/w/a"),
    .removeProject(cwd: "/w/a"),
    .reorderProject(cwd: "/w/a", before: "/w/b"),
    .collapseProject(cwd: "/w/a", collapsed: true),
  ]

  /// In AgentBlock.types order (sessionViewParts reads the tool call)
  static let blocks = [
    #"{"type":"thought","text":"hmm","startedAt":1000,"durationSec":2,"streaming":true}"#,
    #"{"type":"plan","entries":[{"title":"read","status":"in_progress","priority":"high"}],"changed":true}"#,
    #"{"type":"tool_call","observation":"unknown","id":"call-1","kind":"edit","verb":"Edit","verbKey":"verb.edit","mcpServer":"Blockbench","target":"a.ts","targetMono":true,"locations":[{"path":"/w/a.ts","line":3}],"readRange":{"path":"/w/a.ts","start":1,"end":9},"status":"in_progress","background":true,"asyncTask":{"id":"t1","state":"running","canStop":true,"taskType":"shell","name":"build","description":"cargo build","summary":"compiling","lastToolName":"Bash","outputFilePath":"/tmp/t1.log","usage":{"totalTokens":1200,"toolUses":3,"durationMs":4500},"stopRequested":true},"startedAt":1000,"endedAt":2000,"meta":"+1 -1","diffStat":{"add":1,"del":1},"todoEntries":[{"title":"read","status":"completed"}],"content":{"type":"diff","lines":[{"kind":"del","text":"-a","oldLine":1},{"kind":"add","text":"+b","newLine":1}],"source":{"path":"/w/a.ts","omitted":true}},"contents":[{"type":"text","text":"done"},{"type":"image","blob":"3f2a.png","mimeType":"image/png"}],"subagentId":"n1"}"#,
    #"{"type":"text","id":"m1","phase":"final","markdown":"done","streaming":true,"images":[{"blob":"3f2a.png","mimeType":"image/png","uri":"/w/shot.png"}]}"#,
    #"{"type":"permission","id":"perm-1","title":"Run ls","command":"ls","description":"list","planId":"p1","defaultToNo":true,"options":[{"id":"allow","label":"Allow","kind":"allow_once","detail":"once"}]}"#,
    #"{"type":"compaction","id":"c1","status":"failed","error":"too long"}"#,
    ##"{"type":"plan_document","id":"p1","title":"Plan","markdown":"# Plan","path":"/w/plan.md","toolCallId":"call-2","approvalToolCallId":"call-3","status":"ready"}"##,
    #"{"type":"question","id":"q1","toolCallId":"call-4","message":"Pick","questions":[{"id":"a","title":"Kind","text":"Which?","kind":"single","options":[{"id":"x","label":"X","description":"the x"}],"other":true,"numeric":false,"required":true}],"outcome":"answered","answers":{"a":"x"}}"#,
    #"{"type":"image","id":"img-1","blob":"3f2a.png","mimeType":"image/png","uri":"/w/shot.png"}"#,
    #"{"type":"notice","id":"n-1","revision":2,"category":"limit","severity":"warning","info":true,"title":"Slow down","details":"429","actions":["retry"]}"#,
    #"{"type":"steer","id":"st-1","text":"also this","attachments":[{"kind":"quote","text":"x","comment":"y"}]}"#,
    #"{"type":"goal","event":"set","objective":"ship it","iterations":2,"tokensUsed":1200,"timeUsedSeconds":0}"#,
  ]

  static let richView = #"{"id":"s1","external":{"source":"chatgpt","sourceKey":"k1","connectionPrompt":"paste this","state":"receiving","activeTurnId":"t1","lastEventAt":"2026-10-27T19:33:34.971Z"},"agent":"claude","accountId":"acc-1","title":"t","cwd":"/w","status":"auth_required","error":"sign in","canTakeOver":true,"authMethods":[{"id":"chatgpt","name":"ChatGPT","description":"browser login","terminal":{"args":["login"],"env":{"NO_COLOR":"1"}}}],"turns":[\#(userTurn),\#(agentTurn)],"running":true,"rev":2,"controls":{"modes":[],"options":[]},"modelShapes":{"opus":[{"id":"effort","name":"Effort","options":[{"id":"high","name":"High"}],"value":"high"}]},"usage":{"used":1000,"size":200000,"cost":0.5,"compactedFrom":150000},"commands":[],"queued":[{"id":"q1","text":"next","attachments":[],"sending":true}],"canSteer":true,"goal":{"objective":"ship it","status":"active","iterations":2,"tokenBudget":100000,"tokensUsed":1200,"timeUsedSeconds":40,"updatedAt":1761593614971},"goalActions":["set","pause","resume","clear"],"subagents":[\#(subagent)],"forkedFrom":{"sessionId":"s0","turnIndex":3,"agent":"codex"},"createdAt":"2026-10-27T19:33:34.971Z","updatedAt":"2026-10-27T19:33:34.971Z"}"#
  static let userTurn = #"{"role":"user","id":"u1","text":"go","command":"goal","attachments":[{"kind":"text","blob":"b.txt","name":"notes.txt","context":{"sourceId":"browser","sourceName":"Browser"}},{"kind":"selection","blob":"s.txt","uri":"file:///w/a.ts","name":"a.ts","startLine":1,"endLine":3}],"settings":{"modeId":"agent","config":{"model":"opus"}},"edited":true,"planId":"p1","auto":true,"autoReason":"goal","goalContinue":true}"#
  static let agentTurn = #"{"role":"agent","observation":"unknown","blocks":[\#(blocks[11])],"usage":{"input":10,"output":20,"cachedRead":5,"cachedWrite":1,"reasoning":3,"total":39,"modelCalls":2,"model":"opus","requestId":"r1","context":{"used":1000,"size":200000}},"startedAt":1000,"respondedAt":1100,"endedAt":2000,"activity":{"kind":"read","label":"Reading"},"stop":"error","error":{"message":"boom","code":-32603,"kind":"overloaded","retryable":true,"failureId":"f1","actions":["retry"]},"command":{"name":"goal","mode":"agent","options":[{"name":"Model","value":"opus"}]},"retry":{"attempt":1,"max":3,"detail":"retrying"}}"#
  static let subagent = #"{"id":"n1","parentId":"n0","turnIndex":1,"visibility":"session","title":"Scout","task":"find it","role":"scout","state":"running","stateSource":"agent","controls":{"cancel":true},"cancelRequested":false,"background":true,"announcedAt":1000,"endedAt":2000,"model":"opus","usage":{"used":10,"size":100},"peer":{"sessionId":"ps1","agentId":"a1","toolCallId":"call-1","sidechainId":"sc1"},"activity":"Reading","toolCount":3,"result":"done","hasProcess":true,"permissions":[\#(blocks[4])],"question":\#(blocks[7]),"harness":{"agent":"codex","persona":"scout","mode":"consult","thread":"n1","round":1,"sessionId":"cs1","effort":"high","group":"g1"}}"#

  /// In HostMsg.types order (the struct checks above index into it)
  static let pushes = [
    #"{"type":"init","state":{"host":"sidebar","appearance":{"density":"airy","motion":"on"},"agents":[{"id":"claude","name":"Claude Code","accounts":true,"available":true}],"accounts":[{"id":"acc-1","agent":"claude","label":"me@example.com","addedAt":"2026-10-27T19:33:28.658Z","quotaIssue":"rate_limited"}],"accountActions":[{"agent":"claude","via":"import","status":"error","error":"no login"}],"hidden":{"claude":{"model":["haiku"]}},"sessions":[{"id":"s1","title":"t","agent":"claude","cwd":"/w","updatedAt":"2026-10-27T19:33:34.971Z","createdAt":"2026-10-27T19:33:34.971Z","model":{"id":"opus","name":"Opus"}}],"categories":{"categories":[{"id":"c1","name":"Work","cwd":"/w","collapsed":true}],"collapsedProjects":["/old"],"projects":["/w"],"removedProjects":["/gone"],"sections":[{"id":"x1","name":"Clients","projects":["/w"],"collapsed":false}]},"active":\#(view),"settings":\#(settings),"locale":"en","home":"/Users/me","cwd":"/w","blobBase":"http://127.0.0.1:7357/blobs"}}"#,
    #"{"type":"agents","agents":[{"id":"claude","name":"Claude Code"}]}"#,
    #"{"type":"sessions","sessions":[{"id":"s1","title":"t","agent":"claude","cwd":"/w","updatedAt":"2026-10-27T19:33:34.971Z"}]}"#,
    #"{"type":"categories","categories":{"categories":[],"collapsedProjects":[]}}"#,
    #"{"type":"noSession"}"#,
    #"{"type":"session","session":\#(view)}"#,
    #"{"type":"sessionPatch","patch":{"id":"s1","base":1,"view":\#(view),"keep":3,"turns":[{"role":"agent","blocks":[]}],"keepBlocks":1,"blocks":[{"type":"text","markdown":"yz"}]}}"#,
    #"{"type":"accounts","accounts":[{"id":"acc-1","agent":"claude","label":"me@example.com","addedAt":"2026-10-27T19:33:28.658Z"}]}"#,
    #"{"type":"accountActions","actions":[{"agent":"claude","via":"login","status":"pending"}]}"#,
    #"{"type":"hidden","hidden":{"claude":{"effort":["low"]}}}"#,
    #"{"type":"settings","settings":\#(settings),"locale":"zh-CN"}"#,
    #"{"type":"controls","agent":"claude","controls":[{"id":"model","name":"Model","category":"model","options":[{"id":"opus","name":"Opus"}],"value":"opus"}],"shapes":{"opus":[{"id":"effort","name":"Effort","options":[{"id":"high","name":"High"}],"value":"high"}]}}"#,
    #"{"type":"sessionHits","seq":3,"hits":[{"id":"s1","snippet":"… the zebracorn …"}]}"#,
    #"{"type":"subagent","sessionId":"s1","subagentId":"n1","rev":3,"running":true,"turns":[{"role":"user","text":"look"},{"role":"agent","blocks":[{"type":"text","markdown":"found it"}]}]}"#,
    #"{"type":"files","seq":5,"files":[{"uri":"file:///w/src/app.ts","path":"src/app.ts"}]}"#,
    #"{"type":"diffSource","seq":10,"source":{"path":"/w/a.ts","oldText":"a\n","newText":"b\n"}}"#,
    #"{"type":"imageBlob","seq":11,"blob":"3f2a.png"}"#,
    ##"{"type":"sessionMarkdown","seq":4,"markdown":"# t\n","error":"unreadable"}"##,
    #"{"type":"projectDir","seq":6,"entries":[{"name":"src","dir":true},{"name":"a.ts"}],"error":"missing"}"#,
    #"{"type":"projectStatus","seq":7,"root":"/w","marks":{"src/a.ts":"M","new/":"U"}}"#,
    #"{"type":"projectFile","seq":8,"text":"b\n","hunks":[{"at":1,"add":1,"del":["a"]}],"error":"EACCES: permission denied"}"#,
    #"{"type":"sessionChanges","seq":9,"files":[{"path":"a.ts","add":1,"del":1,"gone":false}]}"#,
    #"{"type":"terminalOutput","id":"T","data":"term-42\r\n"}"#,
    #"{"type":"terminalState","id":"T","title":"sh","cols":100,"rows":30,"replay":"","exit":{"code":5,"signal":"SIGHUP"},"error":"no shell"}"#,
    #"{"type":"editTurnResult","requestId":"e1","error":"stale"}"#,
    #"{"type":"appearance","appearance":{"density":"airy","motion":"none"}}"#,
  ]
}
