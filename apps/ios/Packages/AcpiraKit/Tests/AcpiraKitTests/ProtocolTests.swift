import AcpiraFixtures
import AcpiraModels
import Foundation
import Testing

// Round trips of the envelope and protocol mirrors against JSON taken from real places: the Rust and TypeScript tests
// that pin the engine's side, and frames `acpira --ws` sent a URLSessionWebSocketTask client (paths, ids and the
// account anonymized, the agent list and slash commands cut down). The shape checks against the TypeScript sources are
// in ContractTests.
struct ProtocolTests {
  func decode<T: Decodable>(_ type: T.Type, _ json: String) throws -> T {
    try JSONDecoder().decode(type, from: Data(json.utf8))
  }

  /// `value` encodes to the same JSON as `json`, and `json` decodes to `value`
  func roundTrip<T: Codable & Equatable>(_ value: T, _ json: String, sourceLocation: SourceLocation = #_sourceLocation) throws {
    #expect(JSONObject.same(try JSONObject.of(value), try JSONObject.parse(json)), "\(value) encodes otherwise than \(json)",
            sourceLocation: sourceLocation)
    #expect(try decode(T.self, json) == value, sourceLocation: sourceLocation)
  }

  // MARK: Actions

  /// rust/crates/acpira-shared/src/protocol.rs `parses_webview_messages`
  @Test func actionsTheRustSideParses() throws {
    try roundTrip(WebviewMsg.send(sessionId: nil, text: "hi"), #"{"type":"send","text":"hi"}"#)
    try roundTrip(WebviewMsg.send(sessionId: nil, text: "hi", steer: true), #"{"type":"send","text":"hi","steer":true}"#)
    try roundTrip(WebviewMsg.ready(), #"{"type":"ready"}"#)
    try roundTrip(WebviewMsg.ready(patches: true), #"{"type":"ready","patches":true}"#)
  }

  /// test/sidecarContract.test.ts: the actions it sends the real sidecar, with its ids filled in
  @Test func actionsTheSidecarContractSends() throws {
    let id = "148877a3-9bc8-4af0-841b-99bd4ae61fd3"
    try roundTrip(WebviewMsg.send(sessionId: id, text: "where is the zebracorn"),
                  #"{"type":"send","sessionId":"\#(id)","text":"where is the zebracorn"}"#)
    try roundTrip(WebviewMsg.searchSessions(query: "zebracorn", seq: 1), #"{"type":"searchSessions","query":"zebracorn","seq":1}"#)
    try roundTrip(WebviewMsg.permission(sessionId: id, blockId: "perm-1", optionId: "allow"),
                  #"{"type":"permission","sessionId":"\#(id)","blockId":"perm-1","optionId":"allow"}"#)
    try roundTrip(WebviewMsg.stop(sessionId: id), #"{"type":"stop","sessionId":"\#(id)"}"#)
    try roundTrip(WebviewMsg.answer(sessionId: id, blockId: "q-1", answers: ["kind": .one("notes"), "where": .many(["docs"])]),
                  #"{"type":"answer","sessionId":"\#(id)","blockId":"q-1","answers":{"kind":"notes","where":["docs"]}}"#)
    try roundTrip(WebviewMsg.answer(sessionId: id, blockId: "q-2", answers: [:], skip: true),
                  #"{"type":"answer","sessionId":"\#(id)","blockId":"q-2","answers":{},"skip":true}"#)
    try roundTrip(WebviewMsg.compact(sessionId: id), #"{"type":"compact","sessionId":"\#(id)"}"#)
    try roundTrip(WebviewMsg.renameSession(id: id, title: "  Renamed  "), #"{"type":"renameSession","id":"\#(id)","title":"  Renamed  "}"#)
    try roundTrip(WebviewMsg.pinSession(id: id, pinned: true), #"{"type":"pinSession","id":"\#(id)","pinned":true}"#)
    try roundTrip(WebviewMsg.forkSession(sessionId: id, turnIndex: 1), #"{"type":"forkSession","sessionId":"\#(id)","turnIndex":1}"#)
    try roundTrip(WebviewMsg.deleteSession(id: id), #"{"type":"deleteSession","id":"\#(id)"}"#)
    try roundTrip(WebviewMsg.restoreSession(id: id), #"{"type":"restoreSession","id":"\#(id)"}"#)
  }

  /// A prompt with a photo and a file only the phone holds: Rust `Draft` (transcript.rs) takes both inline
  @Test func attachmentsTravelInline() throws {
    let msg = WebviewMsg.send(sessionId: "s1", text: "look", attachments: [
      .image(mimeType: "image/jpeg", data: "/9j/4AAQ", name: "IMG_0001.jpg"),
      .file(uri: "file:///private/var/mobile/a.pdf", name: "a.pdf", data: "JVBERi0="),
    ])
    try roundTrip(msg, #"{"type":"send","sessionId":"s1","text":"look","attachments":[{"kind":"image","mimeType":"image/jpeg","data":"/9j/4AAQ","name":"IMG_0001.jpg"},{"kind":"file","uri":"file:///private/var/mobile/a.pdf","name":"a.pdf","data":"JVBERi0="}]}"#)
  }

  // MARK: Envelopes

  /// rust/crates/acpira-host/tests/engine/harness.rs `hello`, and test/host-preview/shim.js's (a browser's values)
  @Test func helloAsTheEngineTestsAndTheHarnessPageSendIt() throws {
    let rust = #"{"type":"hello","protocolVersion":1,"requestId":"h","client":{"name":"t","version":"0","capabilities":[]},"env":{"hostLanguage":"en"},"settings":{}}"#
    try roundTrip(ShellMsg.hello(Hello(requestId: "h", client: ClientInfo(name: "t", version: "0"), env: ShellEnv(hostLanguage: "en"))), rust)
    let page = #"{"type":"hello","protocolVersion":1,"requestId":"hello-1","client":{"name":"host-preview","version":"0","capabilities":["toast","notify","openExternal","writeSetting","openResolvedFile","openPlanDocument","revealInOS","runInTerminal","openInEditor"]},"env":{"hostLanguage":"zh-CN","blobBase":"http://127.0.0.1:7357/blobs"},"settings":{"defaultAgent":"grok","appearance.motion":"none"}}"#
    guard case .hello(let hello) = try decode(ShellMsg.self, page) else { Issue.record("not hello"); return }
    #expect(hello.client.capabilities.count == 9)
    #expect(hello.settings["defaultAgent"] == "grok")
    #expect(hello.env.blobBase == "http://127.0.0.1:7357/blobs")
    try roundTrip(ShellMsg.hello(hello), page)
  }

  /// rust/crates/acpira-shared/src/sidecar.rs `envelopes`
  @Test func envelopesTheRustSideWrites() throws {
    try roundTrip(ShellMsg.attachView(viewId: "V", host: .sidebar, initial: .mostRecent),
                  #"{"type":"attachView","viewId":"V","host":"sidebar","initial":{"mostRecent":true}}"#)
    try roundTrip(ShellMsg.attachView(viewId: "phone", host: .sidebar, initial: .empty),
                  #"{"type":"attachView","viewId":"phone","host":"sidebar","initial":{"empty":true}}"#)
    try roundTrip(ShellMsg.attachView(viewId: "V", host: .editor, initial: .session("s1")),
                  #"{"type":"attachView","viewId":"V","host":"editor","initial":"s1"}"#)
    // A method the phone does not carry out keeps its name, so the client can refuse it by requestId
    let reveal = try decode(SidecarMsg.self, #"{"type":"platformRequest","request":{"method":"revealInOS","path":"/x"}}"#)
    #expect(reveal == .platformRequest(requestId: nil, request: .unknown(method: "revealInOS")))
    try roundTrip(SidecarMsg.shutdownOk, #"{"type":"shutdownOk"}"#)
    try roundTrip(ShellMsg.webviewMessage(viewId: "phone", message: .ready(patches: true)),
                  #"{"type":"webviewMessage","viewId":"phone","message":{"type":"ready","patches":true}}"#)
    try roundTrip(ShellMsg.platformResponse(requestId: "p1", error: "openBrowser is not supported"),
                  #"{"type":"platformResponse","requestId":"p1","error":"openBrowser is not supported"}"#)
  }

  @Test func helloOkFromTheHarness() throws {
    let ok = try decode(SidecarMsg.self, HarnessFrames.helloOk)
    guard case .helloOk(let payload) = ok else { Issue.record("not helloOk"); return }
    #expect(payload.requestId == "h1")
    #expect(payload.protocolVersion == SidecarProtocol.version)
    #expect(payload.features == ["preview", "projects", "remote", "plugins"])
    #expect(payload.safeMode == nil)
    #expect(JSONObject.same(try JSONObject.of(ok), try JSONObject.parse(HarnessFrames.helloOk)))
  }

  // MARK: Pushes

  /// The init a fresh engine sent a view attached with `initial: {empty: true}`: no session, every SettingsView key,
  /// agentInstalls (not mirrored) ignored
  @Test func initFromTheHarness() throws {
    let frame = try decode(SidecarMsg.self, HarnessFrames.initial)
    guard case .hostMessage("phone", .initState(let state)) = frame else { Issue.record("not init: \(frame)"); return }
    #expect(state.host == .sidebar)
    #expect(state.appearance["motion"] == "on")
    #expect(state.agents.map(\.id) == ["acpira", "grok", "codex", "antigravity", "chatgpt"])
    #expect(state.agents[1].localAccount?.status == "login_required")
    #expect(state.agents.last?.external == true)
    #expect(state.accounts.first?.quotaIssue == .expired)
    #expect(state.accountActions == [])
    #expect(state.sessions.isEmpty)
    #expect(state.categories?.categories.isEmpty == true)
    #expect(state.active == nil)
    #expect(state.settings.defaultAgent == "acpira")
    #expect(state.settings.compactAtTokens == 300_000)
    #expect(state.blobBase == "http://127.0.0.1:7357/blobs")
    // What the app re-encodes decodes the same
    #expect(try JSONDecoder().decode(SidecarMsg.self, from: JSONEncoder().encode(frame)) == frame)
  }

  /// newSession on that view: the whole view at rev 0, the list with the new summary, then a patch to rev 3
  @Test func sessionThenPatchFromTheHarness() throws {
    guard case .hostMessage(_, .session(let view)) = try decode(SidecarMsg.self, HarnessFrames.session) else {
      Issue.record("not session"); return
    }
    #expect(view.status == .starting)
    #expect(view.rev == 0)
    guard case .hostMessage(_, .sessions(let list)) = try decode(SidecarMsg.self, HarnessFrames.sessions) else {
      Issue.record("not sessions"); return
    }
    #expect(list.first?.id == view.id)
    #expect(list.first?.createdAt == "2026-10-27T19:33:34.971Z")
    guard case .hostMessage(_, .sessionPatch(let patch)) = try decode(SidecarMsg.self, HarnessFrames.patch) else {
      Issue.record("not sessionPatch"); return
    }
    #expect(patch.id == view.id)
    #expect(patch.base == view.rev)
    #expect(patch.keep == 0 && patch.turns.isEmpty && patch.keepBlocks == nil)
    #expect(patch.view.rev == 3)
    #expect(patch.view.status == .ready)
    #expect(patch.view.controls.modes.map(\.id) == ["agent", "plan", "raw"])
    #expect(patch.view.canSteer == true)
    #expect(patch.view.commands.last?.input?.hint == "<objective> | pause | resume | clear")
  }

  /// test/sessionPatch.test.ts: a streamed chunk keeping three turns and the first block of the fourth
  @Test func patchFromTheTypeScriptTest() throws {
    let json = #"{"id":"s","base":1,"view":{"id":"s","agent":"fake","title":"t","cwd":"/w","status":"ready","running":false,"rev":2,"createdAt":"a","updatedAt":"b","commands":[],"controls":{"modes":[],"options":[]},"turns":[]},"keep":3,"turns":[{"role":"agent","blocks":[]}],"keepBlocks":1,"blocks":[{"type":"text","markdown":"yz"}]}"#
    let patch = try decode(SessionPatch.self, json)
    #expect(patch.base == 1 && patch.view.rev == 2 && patch.keep == 3 && patch.keepBlocks == 1)
    #expect(patch.turns == [.agent(AgentTurn(blocks: []))])
    #expect(patch.blocks == [.text(TextBlock(markdown: "yz"))])
    #expect(try decode(SessionPatch.self, String(decoding: JSONEncoder().encode(patch), as: UTF8.self)) == patch)
  }

  /// The synthetic lists the app renders, as the pushes that carry them
  @Test func fixtureListsAsPushes() throws {
    let lists = [("agents", "agents"), ("sessions", "sessions"), ("accounts", "accounts")]
    for (type, name) in lists {
      let body = String(decoding: try #require(Fixtures.data(name)), as: UTF8.self)
      let msg = try decode(HostMsg.self, #"{"type":"\#(type)","\#(type)":\#(body)}"#)
      switch msg {
      case .agents(let agents): #expect(agents == Fixtures.agents)
      case .sessions(let sessions): #expect(sessions == Fixtures.sessions)
      case .accounts(let accounts): #expect(accounts == Fixtures.accounts)
      default: Issue.record("\(type) decoded as \(msg)")
      }
    }
  }

  // MARK: Tolerance

  @Test func unknownTypesDecodeToPlaceholders() throws {
    #expect(try decode(HostMsg.self, #"{"type":"terminalOutput","id":"T","data":"x"}"#) == .unknown("terminalOutput"))
    #expect(try decode(WebviewMsg.self, #"{"type":"hologram","x":1}"#) == .unknown("hologram"))
    #expect(try decode(Draft.self, #"{"kind":"hologram"}"#) == .unknown("hologram"))
    #expect(try decode(ShellMsg.self, #"{"type":"previewClose","panelId":"p"}"#) == .unknown("previewClose"))
    #expect(try decode(SidecarMsg.self, #"{"type":"previewOpened","requestId":"r","error":"no"}"#) == .unknown("previewOpened"))
    // An unmirrored push inside a known envelope keeps the envelope
    let wrapped = try decode(SidecarMsg.self, #"{"type":"hostMessage","viewId":"phone","message":{"type":"pluginUi","slots":[]}}"#)
    #expect(wrapped == .hostMessage(viewId: "phone", message: .unknown("pluginUi")))
  }

  @Test func unknownFieldsAndValuesAreIgnored() throws {
    let hits = try decode(HostMsg.self, #"{"type":"sessionHits","seq":2,"hits":[{"id":"s1","snippet":"x","score":0.9}],"extra":true}"#)
    #expect(hits == .sessionHits(seq: 2, hits: [try decode(SessionHit.self, #"{"id":"s1","snippet":"x"}"#)]))
    let actions = try decode(HostMsg.self, #"{"type":"accountActions","actions":[{"agent":"claude","via":"passkey","status":"queued"}]}"#)
    guard case .accountActions(let list) = actions else { Issue.record("not accountActions"); return }
    #expect(list.first?.via == .unknown && list.first?.status == .unknown)
    // A settings push from an engine that dropped or renamed keys reads the defaults
    guard case .settings(let settings, "en") = try decode(HostMsg.self, #"{"type":"settings","settings":{"theme":"dark"},"locale":"en"}"#) else {
      Issue.record("not settings"); return
    }
    #expect(settings.theme == "dark" && settings.defaultAgent == "acpira" && settings.autoCompact)
  }

  @Test func jsonValueKeepsKinds() throws {
    let value = try decode(JSONValue.self, #"{"a":1,"b":true,"c":"x","d":[null,2.5],"e":{}}"#)
    #expect(value == ["a": 1, "b": true, "c": "x", "d": [.null, 2.5], "e": [:]])
    #expect(JSONObject.same(try JSONObject.of(value), try JSONObject.parse(#"{"a":1,"b":true,"c":"x","d":[null,2.5],"e":{}}"#)))
  }
}

/// Frames `acpira --ws` (1.11.0, debug build) sent a URLSessionWebSocketTask client on a fresh home: hello with
/// capabilities [], attachView `{empty: true}`, ready with patches, then newSession. Anonymized and cut down
enum HarnessFrames {
  static let helloOk = #"{"type":"helloOk","requestId":"h1","protocolVersion":1,"sidecar":{"version":"1.11.0","pid":67564},"sessionsDir":"/Users/me/.acpira/harness/sessions","features":["preview","projects","remote","plugins"]}"#

  static let initial = #"{"type":"hostMessage","viewId":"phone","message":{"type":"init","state":{"host":"sidebar","appearance":{"density":"airy","radius":"12","surface":"tonal","font":"inter","userMessage":"block","toolLine":"icon","thought":"orb","sessions":"dropdown","composer":"island","beam":"full","beamColor":"mono","send":"metal","accent":"agent","motion":"on","fold":"codex","autoExpand":"off"},"agents":[{"id":"acpira","name":"Acpira","available":true},{"id":"grok","name":"Grok Build","localAccount":{"label":"Grok Build","status":"login_required"},"available":true,"install":{"command":"curl -fsSL https://x.ai/cli/install.sh | bash","docs":"https://docs.x.ai/build/overview"}},{"id":"codex","name":"Codex","accounts":true,"available":true,"install":{"command":"npm install -g --include=optional @agentclientprotocol/codex-acp@2.1.1","docs":"https://github.com/agentclientprotocol/codex-acp"}},{"id":"antigravity","name":"Antigravity","available":true,"install":{"command":"'/Applications/Acpira.app/Contents/MacOS/acpira' install-agent antigravity","docs":"https://antigravity.google/docs/ide/extensions"}},{"id":"chatgpt","name":"ChatGPT","external":true,"available":true}],"accounts":[{"id":"acc-1","agent":"codex","label":"me@example.com","detail":"ChatGPT Pro","addedAt":"2026-10-27T19:33:28.658Z","quotaIssue":"expired"}],"accountActions":[],"agentInstalls":[],"hidden":{},"sessions":[],"categories":{"categories":[],"collapsedProjects":[]},"settings":{"language":"auto","locale":"en","defaultAgent":"acpira","agentOrder":[],"disabledAgents":[],"sessionScope":"workspace","sessionListPosition":"hidden","autoCompact":true,"compactAtTokens":300000,"hiddenOptions":{},"accountSwitch":"off","theme":"auto","uiFontSize":13,"codeFontSize":12,"diffMarkers":"color","fontSmoothing":false,"shareEditorSelection":true,"steerQueued":false,"notifySound":false,"notifySystem":false,"planAutoApprove":[],"subagents":[],"subagentGroups":[],"agentCpuCap":80,"proxy":"auto","openFilesWith":"system","desktopLayout":"chat","panelScope":"project","panelFiles":true,"panelTerminal":true,"panelBrowser":true},"locale":"en","home":"/Users/me","cwd":"/Users/me","blobBase":"http://127.0.0.1:7357/blobs"}}}"#

  static let session = #"{"type":"hostMessage","viewId":"phone","message":{"type":"session","session":{"id":"148877a3-9bc8-4af0-841b-99bd4ae61fd3","agent":"acpira","title":"New session","cwd":"/Users/me/project","status":"starting","turns":[],"running":false,"rev":0,"controls":{"modes":[],"options":[]},"commands":[],"createdAt":"2026-10-27T19:33:34.971Z","updatedAt":"2026-10-27T19:33:34.971Z"}}}"#

  static let sessions = #"{"type":"hostMessage","viewId":"phone","message":{"type":"sessions","sessions":[{"id":"148877a3-9bc8-4af0-841b-99bd4ae61fd3","title":"New session","agent":"acpira","acpSessionId":"12fb621d-b0eb-4315-b0d6-679eef11045e","cwd":"/Users/me/project","updatedAt":"2026-10-27T19:33:34.971Z","createdAt":"2026-10-27T19:33:34.971Z"}]}}"#

  static let patch = #"{"type":"hostMessage","viewId":"phone","message":{"type":"sessionPatch","patch":{"id":"148877a3-9bc8-4af0-841b-99bd4ae61fd3","base":0,"view":{"id":"148877a3-9bc8-4af0-841b-99bd4ae61fd3","agent":"acpira","title":"New session","cwd":"/Users/me/project","status":"ready","authMethods":[],"turns":[],"running":false,"rev":3,"controls":{"modes":[{"id":"agent","name":"Agent","description":"Reads, edits and runs commands; what asks first is the approval level in settings"},{"id":"plan","name":"Plan","description":"Investigates and writes a plan for approval; changes nothing else"},{"id":"raw","name":"Raw","description":"Talks to the model directly: no system prompt, tools or project instructions"}],"modeId":"agent","options":[]},"commands":[{"name":"compact","description":"Summarize the conversation so far to free context"},{"name":"goal","description":"Set a goal the agent keeps working toward across turns until it is achieved","input":{"hint":"<objective> | pause | resume | clear"}}],"canSteer":true,"goalActions":["set","pause","resume","clear"],"createdAt":"2026-10-27T19:33:34.971Z","updatedAt":"2026-10-27T19:33:34.971Z"},"keep":0,"turns":[]}}}"#
}
