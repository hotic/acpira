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
    try checkUnion("WebviewMsg", in: Self.source("protocol.ts"), mirrored: WebviewMsg.types, samples: ContractSamples.actions) {
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
      "subagents", "subagentGroups", "agentCpuCap", "proxy", "openFilesWith", "desktopLayout", "panelScope", "panelFiles",
      "panelTerminal", "panelBrowser",
    ])
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

  /// The transcript structs the messages carry name only fields transcript.ts has (they mirror a subset)
  @Test(.enabled(if: readable))
  func transcriptFieldsExist() throws {
    let transcript = try Self.source("transcript.ts")
    guard case .initState(let state) = try decode(ContractSamples.pushes[0]) as HostMsg else { Issue.record("not init"); return }
    try checkStruct(try need(state.active), try need(TS.interface(transcript, "SessionView")), subset: true)
    try checkStruct(try need(state.sessions.first), try need(TS.interface(transcript, "SessionSummary")), subset: true)
    try checkStruct(try need(state.accounts.first), try need(TS.interface(transcript, "AccountInfo")), subset: true)
    try checkStruct(try need(state.agents.first), try need(TS.interface(transcript, "AgentInfo")), subset: true)
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
  /// equal, still decodes (to a known case) without any one optional field, and fails without any one required field
  func checkUnion<T: Codable & Equatable>(
    _ name: String, tag: String = "type", in source: String, mirrored: [String], samples: [T], known: (T) -> Bool
  ) throws {
    let members = TS.members(try #require(TS.alias(source, name), "\(name) not found"), tag: tag)
    for literal in mirrored {
      #expect(members[literal] != nil, "\(name) has no member '\(literal)'")
    }
    var covered: Set<String> = []
    for sample in samples {
      let object = try JSONObject.of(sample)
      let literal = try #require(object[tag] as? String)
      covered.insert(literal)
      guard let fields = members[literal] else { continue }
      #expect(Set(object.keys).subtracting([tag]) == Set(fields.map(\.name)), "\(name) '\(literal)' encodes other fields")
      #expect(try JSONDecoder().decode(T.self, from: JSONObject.data(object)) == sample, "\(name) '\(literal)' does not round-trip")
      for field in fields {
        var without = object
        without[field.name] = nil
        let decoded = try? JSONDecoder().decode(T.self, from: JSONObject.data(without))
        if field.optional {
          #expect(decoded.map(known) == true, "\(name) '\(literal)' needs optional '\(field.name)'")
        } else {
          #expect(decoded == nil, "\(name) '\(literal)' takes no '\(field.name)' though TypeScript requires it")
        }
      }
    }
    #expect(covered == Set(mirrored), "\(name) samples cover \(covered.sorted()), not \(mirrored.sorted())")
  }

  /// The struct's stored properties are the TypeScript fields (minus `leftOut`) with the same optionality; with
  /// `subset`, only that every property names a TypeScript field
  func checkStruct(_ value: Any, _ fields: [TSField], leftOut: Set<String> = [], subset: Bool = false) throws {
    let swift = Dictionary(uniqueKeysWithValues: Mirror(reflecting: value).children.compactMap { child in
      child.label.map { ($0, Mirror(reflecting: child.value).displayStyle == .optional) }
    })
    let ts = Dictionary(uniqueKeysWithValues: fields.map { ($0.name, $0.optional) })
    let type = String(describing: Swift.type(of: value))
    #expect(Set(swift.keys).isSubset(of: ts.keys), "\(type) fields missing from TypeScript: \(Set(swift.keys).subtracting(ts.keys).sorted())")
    #expect(leftOut.isSubset(of: ts.keys), "\(type) leaves out fields TypeScript does not have: \(leftOut.subtracting(ts.keys).sorted())")
    if subset { return }
    #expect(Set(swift.keys) == Set(ts.keys).subtracting(leftOut), "\(type) does not mirror \(Set(ts.keys).subtracting(leftOut).subtracting(swift.keys).sorted())")
    for (name, optional) in swift where ts[name] != nil {
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
  static let settings = #"{"language":"auto","locale":"en","defaultAgent":"claude","agentOrder":["claude"],"disabledAgents":["pi"],"autoCompact":true,"compactAtTokens":300000,"hiddenOptions":{"claude":{"model":["haiku"]}},"accountSwitch":"earliestReset","theme":"dark","steerQueued":true,"notifySound":false,"notifySystem":true,"planAutoApprove":["claude"]}"#

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
    .send(sessionId: "s1", text: "hi", attachments: [drafts[0]], steer: true),
    .stop(sessionId: "s1"),
    .permission(sessionId: "s1", blockId: "b1", optionId: "allow"),
    .answer(sessionId: "s1", blockId: "q1", answers: ["a": .one("notes"), "b": .many(["docs"])], skip: false),
    .buildPlan(sessionId: "s1", planId: "p1", optionId: "build", model: ModelPick(configId: "model", value: "opus")),
    .setMode(sessionId: "s1", id: "plan"),
    .setConfig(sessionId: "s1", configId: "effort", value: "high"),
    .selectSession(id: "s1"),
    .newSession(agent: "claude", category: "c1", cwd: "/w"),
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
    .steerQueued(sessionId: "s1", id: "q1"),
    .editQueued(sessionId: "s1", id: "q1", text: "edited", retainedAttachments: [0], attachments: [drafts[2]]),
  ]

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
  ]
}
