package cc.hotic.acpira.model

import java.io.File
import kotlinx.serialization.KSerializer
import kotlinx.serialization.builtins.ListSerializer
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertIs
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

/**
 * Pins the Kotlin mirror to the TypeScript sources by reading the repository, as apps/ios ContractTests does: every
 * literal this build sends or reads must exist in packages/shared, every action sample encodes to exactly its union
 * member's fields, and the mirrored structs name only TypeScript fields. The samples are the iOS ContractSamples, so both
 * apps decode the same pushes
 */
class ContractTest {
  private val root = File(System.getProperty("acpira.root") ?: "../..")
  private fun source(path: String) = File(root, path).readText()
  private fun shared(file: String) = TS.stripComments(source("packages/shared/$file"))

  @Test fun protocolVersionMatchesTheSidecarContract() {
    val v = Regex("""SIDECAR_PROTOCOL_VERSION\s*=\s*(\d+)""").find(source("packages/shared/sidecar.ts"))?.groupValues?.get(1)
    assertEquals(SidecarProtocol.VERSION.toString(), v)
  }

  @Test fun everyMessageTypeExistsInProtocolTs() {
    val protocol = shared("protocol.ts")
    val actions = TS.members(TS.alias(protocol, "WebviewMsg")!!)
    val pushes = TS.members(TS.alias(protocol, "HostMsg")!!)
    for (type in WebviewMsg.types) assertTrue(type in actions, "WebviewMsg has no type '$type'")
    for (type in HostMsg.types) assertTrue(type in pushes, "HostMsg has no type '$type'")
  }

  @Test fun everyEnvelopeExistsInSidecarTs() {
    val sidecar = shared("sidecar.ts")
    val shell = TS.members(TS.alias(sidecar, "ShellMsg")!!)
    val engine = TS.members(TS.alias(sidecar, "SidecarMsg")!!)
    val platform = TS.members(TS.alias(sidecar, "PlatformRequest")!!, tag = "method")
    for (type in ShellMsg.types) assertTrue(type in shell, "ShellMsg has no type '$type'")
    for (type in SidecarMsg.types) assertTrue(type in engine, "SidecarMsg has no type '$type'")
    for (method in PlatformRequest.methods) assertTrue(method in platform, "PlatformRequest has no method '$method'")
    // The helloOk features the pages wait for (named in sidecar.ts's comments)
    val raw = source("packages/shared/sidecar.ts")
    for (feature in listOf("localDir", "providers", "projects")) assertTrue(raw.contains("'$feature'"), "sidecar.ts names no feature '$feature'")
  }

  @Test fun everyProviderKindExistsInProvidersTs() {
    val providers = source("packages/shared/providers.ts")
    for (kind in ProviderAction.kinds + ProviderProbe.kinds + ProbeOutcome.kinds) {
      assertTrue(providers.contains("kind: '$kind'"), "providers.ts has no kind '$kind'")
    }
  }

  @Test fun everyOpAndDraftKindExistsInTranscriptTs() {
    val transcript = shared("transcript.ts")
    val ops = TS.members(TS.alias(transcript, "CategoryOp")!!, tag = "op")
    for (op in CategoryOp.ops) assertTrue(op in ops, "CategoryOp has no op '$op'")
    val drafts = TS.members(TS.alias(transcript, "Draft")!!, tag = "kind")
    for (kind in Draft.kinds) assertTrue(kind in drafts, "Draft has no kind '$kind'")
  }

  @Test fun everyBlockTypeExistsInTranscriptTs() {
    val transcript = source("packages/shared/transcript.ts")
    for (type in AgentBlock.S.types) assertTrue(transcript.contains("type: '$type'"), "transcript.ts has no block '$type'")
  }

  /** Every action sample encodes to exactly the fields of its member of the WebviewMsg union; the samples cover every type */
  @Test fun actionsEncodeTheirMembersFields() {
    val protocol = shared("protocol.ts")
    val members = TS.members(TS.alias(protocol, "WebviewMsg")!!)
    val ops = TS.members(TS.alias(shared("transcript.ts"), "CategoryOp")!!, tag = "op")
    val covered = mutableSetOf<String>()
    for (action in Samples.actions) {
      val json = action.toJson()
      val type = json["type"]!!.jsonPrimitive.content
      assertEquals(action.type, type)
      covered += type
      var fields = members[type] ?: error("WebviewMsg has no '$type'")
      if (type == "categoryOp") fields = fields + "op" + ops[json["op"]!!.jsonPrimitive.content]!!
      assertEquals(fields.toSet(), json.keys - "type", "WebviewMsg '$type' encodes other fields")
    }
    assertEquals(WebviewMsg.types.toSet(), covered, "the samples cover ${covered.sorted()}")
    // The nested payloads name their TypeScript fields
    val edit = (Samples.actions.first { it is WebviewMsg.EditTurn } as WebviewMsg.EditTurn).edit.toJson()
    assertEquals(TS.interfaceFields(protocol, "EditTurnRequest")!!.toSet(), edit.keys)
    for (draft in Samples.drafts) {
      val json = draft.toJson()
      val kind = json["kind"]!!.jsonPrimitive.content
      assertEquals(TS.members(TS.alias(shared("transcript.ts"), "Draft")!!, tag = "kind")[kind]!!.toSet(), json.keys - "kind", "Draft '$kind'")
    }
  }

  /** Every push sample decodes to its own case (not Unknown), names only its member's fields, and the samples cover every type */
  @Test fun pushesDecode() {
    val members = TS.members(TS.alias(shared("protocol.ts"), "HostMsg")!!)
    val covered = mutableSetOf<String>()
    for (sample in Samples.pushes) {
      val obj = WireJson.parseToJsonElement(sample).jsonObject
      val type = obj["type"]!!.jsonPrimitive.content
      covered += type
      assertEquals(members[type]!!.toSet(), obj.keys - "type", "the '$type' sample does not set every field")
      val msg = HostMsg.decode(WireJson, obj)
      assertTrue(msg !is HostMsg.Unknown, "'$type' decodes to Unknown")
    }
    assertEquals(HostMsg.types.toSet(), covered, "the samples cover ${covered.sorted()}")

    fun push(type: String) = HostMsg.decode(WireJson, WireJson.parseToJsonElement(Samples.pushes.first { it.contains("\"type\":\"$type\"") }).jsonObject)
    val init = assertIs<HostMsg.Init>(push("init")).state
    assertEquals(AccountQuotaIssue.rate_limited, init.accounts.single().quotaIssue)
    assertEquals(AccountActionVia.`import`, init.accountActions!!.single().via)
    assertEquals(RelayMode.consult, init.settings.subagents.single().mode)
    assertEquals("before a merge", init.settings.subagents.single().summonWhen)
    assertEquals(listOf("haiku"), init.hidden["claude"]!!["model"])
    assertEquals(ProjectError.TooLarge, assertIs<HostMsg.StagedFile>(push("stagedFile")).error)
    assertEquals(15874091L, assertIs<HostMsg.StagedFile>(push("stagedFile")).size)
    assertEquals(ProjectError.Message("EACCES: permission denied"), assertIs<HostMsg.ProjectFile>(push("projectFile")).error)
    val status = assertIs<HostMsg.ProjectStatus>(push("projectStatus"))
    assertEquals(mapOf("src/a.ts" to GitMark.modified, "new/" to GitMark.untracked), status.marks)
    assertEquals(TerminalExit(5, "SIGHUP"), assertIs<HostMsg.TerminalState>(push("terminalState")).exit)
    assertEquals(ConnectionState.error, assertIs<HostMsg.Connections>(push("connections")).connections.single().state)
    val controls = assertIs<HostMsg.Controls>(push("controls"))
    assertEquals("high", controls.shapes!!["opus"]!!.single().value)
    val subagent = assertIs<HostMsg.Subagent>(push("subagent"))
    assertEquals(2, subagent.turns.size)
  }

  /** A sessionsPatch applies to the list it was made against and refuses any other */
  @Test fun aListPatchAppliesAsRuns() {
    val patch = assertIs<HostMsg.ListPatch>(HostMsg.decode(WireJson, WireJson.parseToJsonElement(Samples.pushes.first { it.contains("sessionsPatch") }).jsonObject)).patch
    val held = listOf(SessionSummary("s1"), SessionSummary("s2"))
    assertEquals(listOf("s3", "s1", "s2"), patch.apply(held, 3)!!.map { it.id })
    assertNull(patch.apply(held, 2))
    assertNull(patch.apply(held.take(1), 3))
    // Round trip: a kept run stays a pair
    val again = WireJson.encodeToJsonElement(SessionsPatch.serializer(), patch).jsonObject
    assertIs<JsonArray>(again["runs"]!!.jsonArray[1])
  }

  /** The mirrored structs name only TypeScript fields, and all of them but those left out on purpose */
  @Test fun structsMirrorTheirInterfaces() {
    val transcript = shared("transcript.ts")
    val protocol = shared("protocol.ts")
    val subagents = shared("subagents.ts")
    fun check(s: KSerializer<*>, fields: List<String>?, leftOut: Set<String> = emptySet(), subset: Boolean = false) {
      val name = s.descriptor.serialName
      assertNotNull(fields, "no interface for $name")
      val kotlin = (0 until s.descriptor.elementsCount).map { s.descriptor.getElementName(it) }.toSet()
      assertTrue(fields.toSet().containsAll(kotlin), "$name fields missing from TypeScript: ${(kotlin - fields.toSet()).sorted()}")
      assertTrue(fields.toSet().containsAll(leftOut), "$name leaves out fields TypeScript does not have: ${(leftOut - fields.toSet()).sorted()}")
      if (!subset) assertEquals(fields.toSet() - leftOut, kotlin, "$name does not mirror ${(fields.toSet() - leftOut - kotlin).sorted()}")
    }
    check(SessionView.serializer(), TS.interfaceFields(transcript, "SessionView"))
    // completedAt / question feed the page's plugin style hooks; the phone has none
    check(SessionSummary.serializer(), TS.interfaceFields(transcript, "SessionSummary"), setOf("completedAt", "question"))
    check(AgentInfo.serializer(), TS.interfaceFields(transcript, "AgentInfo"))
    check(AccountInfo.serializer(), TS.interfaceFields(transcript, "AccountInfo"), subset = true)
    check(AuthMethodInfo.serializer(), TS.interfaceFields(transcript, "AuthMethodInfo"))
    check(Usage.serializer(), TS.interfaceFields(transcript, "Usage"))
    check(SessionGoal.serializer(), TS.interfaceFields(transcript, "SessionGoal"))
    check(ForkOrigin.serializer(), TS.interfaceFields(transcript, "ForkOrigin"))
    check(TextContext.serializer(), TS.interfaceFields(transcript, "TextContext"))
    check(FullDiffSource.serializer(), TS.interfaceFields(transcript, "FullDiffSource"), setOf("omitted"))
    check(AsyncTaskInfo.serializer(), TS.interfaceFields(transcript, "AsyncTaskInfo"))
    check(TurnUsage.serializer(), TS.interfaceFields(transcript, "TurnUsage"))
    check(TurnError.serializer(), TS.interfaceFields(transcript, "TurnError"))
    check(UserTurn.serializer(), TS.interfaceFields(transcript, "UserTurn"), setOf("role"))
    // Plugin confirmation cards are desktop-only, as on iOS
    check(AgentTurn.serializer(), TS.interfaceFields(transcript, "AgentTurn"), setOf("role", "pluginCards"))
    check(SessionCategories.serializer(), TS.interfaceFields(transcript, "SessionCategories"), setOf("projectMeta"))
    for ((s, name) in listOf(
      ThoughtBlock.serializer() to "ThoughtBlock", PlanBlock.serializer() to "PlanBlock", ToolCallBlock.serializer() to "ToolCallBlock",
      TextBlock.serializer() to "TextBlock", PermissionBlock.serializer() to "PermissionBlock", CompactionBlock.serializer() to "CompactionBlock",
      PlanDocumentBlock.serializer() to "PlanDocumentBlock", QuestionBlock.serializer() to "QuestionBlock", ImageBlock.serializer() to "ImageBlock",
      NoticeBlock.serializer() to "NoticeBlock", SteerBlock.serializer() to "SteerBlock", GoalBlock.serializer() to "GoalBlock",
    )) check(s, TS.interfaceFields(transcript, name), setOf("type"))
    check(SubagentSummary.serializer(), TS.interfaceFields(subagents, "SubagentSummary"))
    check(SubagentHarness.serializer(), TS.interfaceFields(subagents, "SubagentHarness"))
    check(SubagentPersona.serializer(), TS.interfaceFields(subagents, "SubagentPersona"))
    check(SubagentGroup.serializer(), TS.interfaceFields(subagents, "SubagentGroup"))
    check(InitState.serializer(), TS.interfaceFields(protocol, "InitState"), setOf("agentInstalls"))
    check(SettingsView.serializer(), TS.interfaceFields(shared("settings.ts"), "SettingsView"), setOf(
      "sessionScope", "sessionListPosition", "uiFontSize", "codeFontSize", "diffMarkers", "fontSmoothing", "shareEditorSelection",
      "agentCpuCap", "proxy", "openFilesWith", "desktopLayout", "panelScope", "panelFiles", "panelTerminal", "panelBrowser", "sshLocalAgents",
    ))
    for ((s, name) in listOf(
      FileHit.serializer() to "FileHit", ProjectEntry.serializer() to "ProjectEntry", ProjectHunk.serializer() to "ProjectHunk",
      ProjectChange.serializer() to "ProjectChange", TerminalExit.serializer() to "TerminalExit", SessionHit.serializer() to "SessionHit",
      AccountAction.serializer() to "AccountAction",
    )) check(s, TS.interfaceFields(protocol, name))
    check(ConnectionView.serializer(), TS.interfaceFields(shared("remote.ts"), "ConnectionView"), subset = true)
    check(PlatformRequest.ToastOpen.serializer(), TS.interfaceFields(protocol, "ToastOpen"), setOf("reveal"))
    val helloOk = TS.members(TS.alias(shared("sidecar.ts"), "SidecarMsg")!!)["helloOk"]!!
    check(HelloOk.serializer(), helloOk)
  }

  /** The string enums name exactly their TypeScript literals (`unknown` is the app's own fallback) */
  @Test fun stringUnionsMatch() {
    val transcript = shared("transcript.ts")
    val protocol = shared("protocol.ts")
    val subagents = shared("subagents.ts")
    fun <E : Enum<E>> check(entries: List<E>, union: String?, name: String, wire: (E) -> String = { it.name }) {
      assertNotNull(union, "no union $name")
      val kotlin = entries.map(wire).filter { it != "unknown" && it.isNotEmpty() }.toSet()
      assertEquals(TS.literals(union), kotlin, name)
    }
    fun field(source: String, iface: String, name: String) = TS.interfaceType(source, iface, name)
    check(GoalStatus.entries, TS.alias(transcript, "GoalStatus"), "GoalStatus")
    check(GoalAction.entries, TS.alias(transcript, "GoalAction"), "GoalAction")
    check(GoalEvent.entries, TS.alias(transcript, "GoalEvent"), "GoalEvent")
    check(AddAccountVia.entries, TS.alias(protocol, "AddAccountVia"), "AddAccountVia")
    check(AccountActionVia.entries, TS.alias(protocol, "AddAccountVia") + " | " + TS.alias(protocol, "AccountActionVia"), "AccountActionVia")
    check(AccountActionStatus.entries, field(protocol, "AccountAction", "status"), "AccountAction.status")
    check(ExportFormat.entries, TS.alias(protocol, "ExportFormat"), "ExportFormat")
    check(ProjectBase.entries, TS.alias(protocol, "ProjectBase"), "ProjectBase")
    check(GitMark.entries, TS.alias(protocol, "GitMark"), "GitMark") { it.wire }
    check(EditTurnRequest.Intent.entries, field(protocol, "EditTurnRequest", "intent"), "EditTurnRequest.intent")
    check(SubagentState.entries, TS.alias(subagents, "SubagentState"), "SubagentState")
    check(SubagentVisibility.entries, TS.alias(subagents, "SubagentVisibility"), "SubagentVisibility")
    check(SubagentSummary.StateSource.entries, field(subagents, "SubagentSummary", "stateSource"), "SubagentSummary.stateSource")
    check(RelayMode.entries, TS.alias(subagents, "RelayMode"), "RelayMode")
    check(AccountQuotaIssue.entries, TS.alias(transcript, "AccountQuotaIssue") ?: field(transcript, "AccountInfo", "quotaIssue"), "AccountQuotaIssue")
    check(ConnectionState.entries, TS.alias(shared("remote.ts"), "ConnectionState"), "ConnectionState")
    assertEquals(TS.literals(TS.alias(protocol, "ProjectError")!!), ProjectError.reasons.map { it.raw }.toSet(), "ProjectError")
  }

  @Test fun unknownBlocksAndEnumsDecodeToPlaceholders() {
    val turn = WireJson.decodeFromString(Turn.serializer(), """
      {"role":"agent","blocks":[{"type":"hologram","x":1},{"type":"tool_call","id":"t","kind":"teleport","verb":"Go","status":"warping"}]}
    """)
    val blocks = (turn as AgentTurn).blocks
    assertEquals(UnknownBlock("hologram"), blocks[0])
    val tool = blocks[1] as ToolCallBlock
    assertEquals(ToolKind.other, tool.kind)
    assertEquals(ToolStatus.completed, tool.status)
    // An unknown push, an unknown project error and a sidekick step without an id keep everything else
    assertEquals(HostMsg.Unknown("hologram"), HostMsg.decode(WireJson, JsonObject(mapOf("type" to JsonPrimitive("hologram")))))
    val node = WireJson.decodeFromString(SubagentSummary.serializer(), """
      {"id":"n1","turnIndex":0,"visibility":"teleport","state":"running","steps":[{"kind":"read"},{"id":"c1","kind":"read","verb":"Read","status":"completed"}]}
    """)
    assertEquals(SubagentVisibility.unknown, node.visibility)
    assertEquals(listOf("c1"), node.steps!!.map { it.id })
  }

  /** The iOS samples' blocks and rich view decode with every part the phone draws */
  @Test fun theRichViewDecodes() {
    val view = WireJson.decodeFromString(SessionView.serializer(), Samples.richView)
    assertEquals(listOf(GoalAction.set, GoalAction.pause, GoalAction.resume, GoalAction.clear), view.goalActions)
    assertEquals(listOf("login"), view.authMethods!!.single().terminal!!.args)
    val node = view.subagents!!.single()
    assertEquals(SubagentVisibility.session, node.visibility)
    assertEquals(RelayMode.consult, node.harness!!.mode)
    assertEquals("sc1", node.peer.sidechainId)
    assertEquals("perm-1", node.permissions!!.single().id)
    assertEquals("q1", node.question!!.id)
    val user = view.turns.first() as UserTurn
    assertEquals("browser", (user.attachments!!.first() as Attachment.Text).context!!.sourceId)
    assertEquals("high", view.modelShapes!!["opus"]!!.single().value)
    for (sample in Samples.blocks) {
      val block = WireJson.decodeFromString(AgentBlock.serializer(), sample)
      assertTrue(block !is UnknownBlock, "block does not decode: $sample")
    }
    val tool = WireJson.decodeFromString(AgentBlock.serializer(), Samples.blocks[2]) as ToolCallBlock
    assertEquals(1200, tool.asyncTask!!.usage!!.totalTokens)
    val question = WireJson.decodeFromString(AgentBlock.serializer(), Samples.blocks[7]) as QuestionBlock
    assertEquals("3f2a.png", question.answerImages!!["a"]!!.single().blob)
  }

  @Test fun everyFixtureDecodes() {
    val dir = File(root, "apps/ios/Packages/AcpiraKit/Sources/AcpiraFixtures/synthetic")
    val names = dir.listFiles { f -> f.extension == "json" }!!.map { it.nameWithoutExtension }
    val views = names.filter { it !in setOf("accounts", "agents", "clock", "sessions") && !it.endsWith(".children") }
    assertTrue(views.size > 10)
    for (name in views) {
      val view = WireJson.decodeFromString(SessionView.serializer(), File(dir, "$name.json").readText())
      assertEquals(true, view.id.isNotEmpty(), name)
    }
    val sessions = WireJson.decodeFromString(ListSerializer(SessionSummary.serializer()), File(dir, "sessions.json").readText())
    assertTrue(sessions.isNotEmpty())
  }

  /** `ssh://` folders split as remote.ts and acpira_shared::remote split them (the fixture every side runs) */
  @Test fun remoteFolders() {
    val cases = WireJson.parseToJsonElement(source("test/fixtures/remote-folders.json")).jsonArray
    assertTrue(cases.isNotEmpty())
    for (item in cases.map { it.jsonObject }) {
      val input = item.string("input")!!
      val parsed = RemoteFolder.parse(input)
      assertEquals(item.string("alias"), parsed?.alias, "alias of $input")
      assertEquals(item.string("path"), parsed?.path, "path of $input")
    }
  }

  @Test fun aSendCarriesItsFieldsAsTheEngineReadsThem() {
    val json = WebviewMsg.Send("s1", "hi", clientId = "c").toJson().toString()
    assertEquals("""{"type":"send","sessionId":"s1","text":"hi","clientId":"c"}""", json)
  }

  @Test fun aSteerOfAQueuedPromptIsStampedAndReplayed() {
    val steer = WebviewMsg.SteerQueued("s1", "q1").stamped { "c9" }
    assertEquals("c9", steer.clientId)
    assertTrue(steer.replayable)
    assertTrue(!WebviewMsg.SearchFiles("a", 1).replayable)
  }

  // The engine's Hello has no defaults for these; WireJson leaves default values out, so each must be forced on
  @Test fun aHelloCarriesEveryFieldTheEngineRequires() {
    val hello = ShellMsg.HelloMsg(Hello(requestId = "h", client = ClientInfo("a", "1"), env = ShellEnv(hostLanguage = "en"))).toJson()
    for (key in listOf("type", "protocolVersion", "requestId", "client", "env")) assertTrue(key in hello, "hello misses $key: $hello")
    assertEquals(SidecarProtocol.VERSION.toString(), hello["protocolVersion"].toString())
  }

  @Test fun envelopesDecode() {
    val ok = assertIs<SidecarMsg.HelloOkMsg>(SidecarMsg.decode(Samples.helloOk)).ok
    assertEquals(42, ok.sidecar!!.pid)
    assertEquals(true, ok.safeMode)
    assertTrue(ok.has("projects"))
    assertEquals(SidecarMsg.ShutdownOk, SidecarMsg.decode("""{"type":"shutdownOk"}"""))
    val notify = assertIs<SidecarMsg.Platform>(SidecarMsg.decode(
      """{"type":"platformRequest","request":{"method":"notify","sessionId":"s1","title":"t","body":"b","open":"Open","sound":true,"system":false}}""",
    )).request
    assertEquals(PlatformRequest.Notify("s1", "t", "b", "Open", sound = true, system = false), notify)
    val toast = assertIs<SidecarMsg.Platform>(SidecarMsg.decode(
      """{"type":"platformRequest","request":{"method":"toast","level":"info","text":"x","open":{"label":"Open","path":"/w/a.md"}}}""",
    )).request
    assertEquals(PlatformRequest.ToastOpen("Open", "/w/a.md"), (toast as PlatformRequest.Toast).open)
  }
}

/** The iOS ContractSamples (apps/ios … Tests/AcpiraKitTests/ContractTests.swift), in Kotlin */
private object Samples {
  const val helloOk = """{"type":"helloOk","requestId":"h1","protocolVersion":1,"sidecar":{"version":"1.11.0","pid":42},"sessionsDir":"/Users/me/.acpira/sessions","features":["preview","projects"],"safeMode":true}"""

  val drafts = listOf(
    Draft.Image("image/png", "iVBORw0KGgo=", "shot.png"),
    Draft.Text("notes.txt", "hi", TextContext("browser", "Browser", "http://localhost:3000", "this")),
    Draft.File("file:///w/a.pdf", "a.pdf", "JVBERi0="),
    Draft.Quote("the reply's words", "why this"),
  )

  private val provider = Provider(WireJson.parseToJsonElement(
    """{"id":"gw","name":"Gateway","preset":"custom","format":"openai-chat","baseUrl":"https://gw.example/v1","fullUrl":false,"enabled":true,"models":[],"headers":{"X-Team":"a"}}""",
  ).jsonObject)

  private val edit = EditTurnRequest(
    sessionId = "s1", turnIndex = 0, turnCount = 2, originalText = "fail", turnId = "u1", text = "hello again", retainedAttachments = listOf(0),
    attachments = listOf(drafts[1]), settings = TurnSettings("agent", mapOf("model" to "opus")), intent = EditTurnRequest.Intent.`continue`,
  )

  val actions: List<WebviewMsg> = listOf(
    WebviewMsg.Ready(patches = true, listPatches = true),
    WebviewMsg.Resync,
    WebviewMsg.ResyncSessions,
    WebviewMsg.Send("s1", "hi", listOf(drafts[0]), steer = true, clientId = "c1"),
    WebviewMsg.Stop("s1", "c2"),
    WebviewMsg.Permission("s1", "b1", "allow", "c3"),
    WebviewMsg.Answer("s1", "q1", mapOf("a" to JsonPrimitive("notes"), "b" to JsonArray(listOf(JsonPrimitive("docs")))), skip = false, clientId = "c4", images = mapOf("a" to listOf(drafts[0]))),
    WebviewMsg.BuildPlan("s1", "p1", "build", ModelPick("model", "opus")),
    WebviewMsg.SetMode("s1", "plan"),
    WebviewMsg.SetConfig("s1", "effort", "high"),
    WebviewMsg.SelectSession("s1"),
    WebviewMsg.DeselectSession("s1"),
    WebviewMsg.NewSession("claude", "/w", chat = true, category = "c1"),
    WebviewMsg.RenameSession("s1", "Renamed"),
    WebviewMsg.DeleteSession("s1", "d1"),
    WebviewMsg.RestoreSession("s1"),
    WebviewMsg.PinSession("s1", true),
    WebviewMsg.ArchiveSession("s1", true),
    WebviewMsg.SetSessionUnread("s1", true),
    WebviewMsg.MarkAllRead,
    WebviewMsg.SearchSessions("zebracorn", 3),
    WebviewMsg.ForkSession("s1", 1, "codex"),
    WebviewMsg.SelectAccount("s1", "acc-1"),
    WebviewMsg.RefreshQuota("claude", "s1"),
    WebviewMsg.Compact("s1"),
    WebviewMsg.Retry("s1"),
    WebviewMsg.TakeOverSession("s1"),
    WebviewMsg.Dequeue("s1", "q1"),
    WebviewMsg.SendQueued("s1", "q1"),
    WebviewMsg.SteerQueued("s1", "q1", "c5"),
    WebviewMsg.EditQueued("s1", "q1", "edited", listOf(0), listOf(drafts[2])),
    WebviewMsg.CategoryOp(CategoryOp.ReorderProject("/w/a", "/w/b"), file = "s1"),
    WebviewMsg.MoveSession("s1", "/w2", "c1"),
    WebviewMsg.SessionMarkdown("s1", 4, ExportFormat.text),
    WebviewMsg.AddAccount("codex", AddAccountVia.login, "s1"),
    WebviewMsg.RemoveAccount("acc-1", "s1"),
    WebviewMsg.UnlockCredentials("claude", "s1"),
    WebviewMsg.Login("s1", "chatgpt"),
    WebviewMsg.Reconnect("s1"),
    WebviewMsg.RetryTurn("s1"),
    WebviewMsg.EditTurn("e1", edit),
    WebviewMsg.ObserveSubagent("s1", "n1"),
    WebviewMsg.UnobserveSubagent("s1", "n1"),
    WebviewMsg.CancelSubagent("s1", "n1"),
    WebviewMsg.StopAsyncTask("s1", "t1"),
    WebviewMsg.Goal("s1", GoalAction.set, "ship it"),
    WebviewMsg.SearchFiles("app", 5),
    WebviewMsg.SetSetting("theme", JsonPrimitive("dark")),
    WebviewMsg.Controls("claude", fresh = true, cwd = "ssh://gen1/w"),
    WebviewMsg.ProjectDir("s1", "src", 6),
    WebviewMsg.ProjectStatus("s1", 7),
    WebviewMsg.ProjectFile("s1", "src/a.ts", 8, ProjectBase.head),
    WebviewMsg.SessionChanges("s1", 9),
    WebviewMsg.DiffSource("s1", "call-1", 0, 10, "n1"),
    WebviewMsg.ImageBlob("s1", "/w/shot.png", 11, shown = true),
    WebviewMsg.StageFile("s1", "promo/film.mp4", 12),
    WebviewMsg.OpenPlan("s1", "p1"),
    WebviewMsg.TerminalOpen("T", "/w", 100, 30),
    WebviewMsg.TerminalInput("T", "ls\r", false),
    WebviewMsg.TerminalResize("T", 120, 40),
    WebviewMsg.TerminalAck("T", 4096),
    WebviewMsg.TerminalClose("T"),
    WebviewMsg.TerminalKill("T"),
    WebviewMsg.Connections,
    WebviewMsg.LocalDir("~/code", 31),
    WebviewMsg.Providers,
    WebviewMsg.ProviderAction(ProviderAction.Save(provider, "sk-1")),
    WebviewMsg.ProviderProbe("p1", ProviderProbe.Models(provider, "sk-1")),
  )

  /** In AgentBlock.S.types order */
  val blocks = listOf(
    """{"type":"thought","text":"hmm","startedAt":1000,"durationSec":2,"streaming":true}""",
    """{"type":"plan","entries":[{"title":"read","status":"in_progress","priority":"high"}],"changed":true}""",
    """{"type":"tool_call","observation":"unknown","id":"call-1","kind":"edit","verb":"Edit","verbKey":"verb.edit","mcpServer":"Blockbench","target":"a.ts","targetMono":true,"locations":[{"path":"/w/a.ts","line":3}],"readRange":{"path":"/w/a.ts","start":1,"end":9},"status":"in_progress","background":true,"asyncTask":{"id":"t1","state":"running","canStop":true,"taskType":"shell","name":"build","description":"cargo build","summary":"compiling","lastToolName":"Bash","outputFilePath":"/tmp/t1.log","usage":{"totalTokens":1200,"toolUses":3,"durationMs":4500},"stopRequested":true},"startedAt":1000,"endedAt":2000,"meta":"+1 -1","diffStat":{"add":1,"del":1},"todoEntries":[{"title":"read","status":"completed"}],"content":{"type":"diff","lines":[{"kind":"del","text":"-a","oldLine":1},{"kind":"add","text":"+b","newLine":1}],"source":{"path":"/w/a.ts","omitted":true}},"contents":[{"type":"text","text":"done"},{"type":"image","blob":"3f2a.png","mimeType":"image/png"}],"subagentId":"n1"}""",
    """{"type":"text","id":"m1","phase":"final","markdown":"done","streaming":true,"images":[{"blob":"3f2a.png","mimeType":"image/png","uri":"/w/shot.png"}]}""",
    """{"type":"permission","id":"perm-1","title":"Run ls","command":"ls","description":"list","planId":"p1","defaultToNo":true,"options":[{"id":"allow","label":"Allow","kind":"allow_once","detail":"once"}]}""",
    """{"type":"compaction","id":"c1","status":"failed","error":"too long"}""",
    """{"type":"plan_document","id":"p1","title":"Plan","markdown":"# Plan","path":"/w/plan.md","toolCallId":"call-2","approvalToolCallId":"call-3","status":"ready"}""",
    """{"type":"question","id":"q1","toolCallId":"call-4","message":"Pick","questions":[{"id":"a","title":"Kind","text":"Which?","kind":"single","options":[{"id":"x","label":"X","description":"the x"}],"other":true,"numeric":false,"required":true}],"outcome":"answered","answers":{"a":"x"},"images":true,"answerImages":{"a":[{"blob":"3f2a.png","mimeType":"image/png"}]}}""",
    """{"type":"image","id":"img-1","blob":"3f2a.png","mimeType":"image/png","uri":"/w/shot.png"}""",
    """{"type":"notice","id":"n-1","revision":2,"category":"limit","severity":"warning","info":true,"title":"Slow down","details":"429","actions":["retry"],"retry":{"status":"in_progress","max":10,"nextAt":1,"reconnect":true,"attempts":[{"at":1,"attempt":1,"text":"529"}]}}""",
    """{"type":"steer","id":"st-1","text":"also this","attachments":[{"kind":"quote","text":"x","comment":"y"}]}""",
    """{"type":"goal","event":"set","objective":"ship it","iterations":2,"tokensUsed":1200,"timeUsedSeconds":0}""",
  )

  private const val view = """{"id":"s1","agent":"claude","title":"t","cwd":"/w","status":"ready","turns":[],"running":false,"rev":2,"controls":{"modes":[],"options":[]},"commands":[],"createdAt":"2026-10-27T19:33:34.971Z","updatedAt":"2026-10-27T19:33:34.971Z"}"""
  private const val settings = """{"language":"auto","locale":"en","defaultAgent":"claude","agentOrder":["claude"],"disabledAgents":["pi"],"autoCompact":true,"compactAtTokens":300000,"hiddenOptions":{"claude":{"model":["haiku"]}},"accountSwitch":"earliestReset","theme":"dark","steerQueued":true,"notifySound":false,"notifySystem":true,"planAutoApprove":["claude"],"subagents":[{"id":"review","name":"Review","agent":"codex","model":"gpt-6","effort":"high","mode":"consult","when":"before a merge","brief":"look for bugs","enabled":true,"duty":"reviewer","changes":true}],"subagentGroups":[{"id":"panel","name":"Panel","members":["review"],"when":"a second opinion","enabled":true}]}"""
  private const val userTurn = """{"role":"user","id":"u1","text":"go","command":"goal","attachments":[{"kind":"text","blob":"b.txt","name":"notes.txt","context":{"sourceId":"browser","sourceName":"Browser"}},{"kind":"selection","blob":"s.txt","uri":"file:///w/a.ts","name":"a.ts","startLine":1,"endLine":3}],"settings":{"modeId":"agent","config":{"model":"opus"}},"edited":true,"planId":"p1","auto":true,"autoReason":"goal","goalContinue":true}"""
  private val agentTurn = """{"role":"agent","observation":"unknown","blocks":[${blocks[11]}],"usage":{"input":10,"output":20,"cachedRead":5,"cachedWrite":1,"reasoning":3,"total":39,"modelCalls":2,"decodeMs":800,"model":"opus","requestId":"r1","context":{"used":1000,"size":200000}},"startedAt":1000,"respondedAt":1100,"endedAt":2000,"idleMs":300,"activity":{"kind":"read","label":"Reading"},"stop":"error","error":{"message":"boom","code":-32603,"kind":"overloaded","retryable":true,"failureId":"f1","actions":["retry"]},"command":{"name":"goal","mode":"agent","options":[{"name":"Model","value":"opus"}]},"retry":{"attempt":1,"max":3,"detail":"retrying"}}"""
  private val subagent = """{"id":"n1","parentId":"n0","turnIndex":1,"visibility":"session","title":"Scout","task":"find it","role":"scout","state":"running","stateSource":"agent","controls":{"cancel":true},"cancelRequested":false,"background":true,"announcedAt":1000,"endedAt":2000,"model":"opus","usage":{"used":10,"size":100},"peer":{"sessionId":"ps1","agentId":"a1","toolCallId":"call-1","sidechainId":"sc1"},"activity":"Reading","toolCount":3,"result":"done","hasProcess":true,"permissions":[${blocks[4]}],"question":${blocks[7]},"harness":{"agent":"codex","persona":"scout","mode":"consult","thread":"n1","round":1,"sessionId":"cs1","effort":"high","group":"g1"}}"""
  val richView = """{"id":"s1","agent":"claude","accountId":"acc-1","title":"t","cwd":"/w","status":"auth_required","error":"sign in","canTakeOver":true,"authMethods":[{"id":"chatgpt","name":"ChatGPT","description":"browser login","terminal":{"args":["login"],"env":{"NO_COLOR":"1"}}}],"turns":[$userTurn,$agentTurn],"running":true,"rev":2,"controls":{"modes":[],"options":[]},"modelShapes":{"opus":[{"id":"effort","name":"Effort","options":[{"id":"high","name":"High"}],"value":"high"}]},"usage":{"used":1000,"size":200000,"cost":0.5,"compactedFrom":150000},"commands":[],"queued":[{"id":"q1","text":"next","attachments":[],"sending":true}],"canSteer":true,"goal":{"objective":"ship it","status":"active","iterations":2,"tokenBudget":100000,"tokensUsed":1200,"timeUsedSeconds":40,"updatedAt":1761593614971},"goalActions":["set","pause","resume","clear"],"subagents":[$subagent],"forkedFrom":{"sessionId":"s0","turnIndex":3,"agent":"codex"},"createdAt":"2026-10-27T19:33:34.971Z","updatedAt":"2026-10-27T19:33:34.971Z"}"""

  /** In HostMsg.types order */
  val pushes = listOf(
    """{"type":"init","state":{"host":"sidebar","appearance":{"density":"airy","motion":"on"},"agents":[{"id":"claude","name":"Claude Code","accounts":true,"available":true}],"accounts":[{"id":"acc-1","agent":"claude","label":"me@example.com","addedAt":"2026-10-27T19:33:28.658Z","quotaIssue":"rate_limited"}],"accountActions":[{"agent":"claude","via":"import","status":"error","error":"no login"}],"hidden":{"claude":{"model":["haiku"]}},"sessions":[{"id":"s1","title":"t","agent":"claude","cwd":"/w","updatedAt":"2026-10-27T19:33:34.971Z","createdAt":"2026-10-27T19:33:34.971Z","model":{"id":"opus","name":"Opus"}}],"categories":{"categories":[{"id":"c1","name":"Work","cwd":"/w","collapsed":true}],"collapsedProjects":["/old"],"projects":["/w"],"removedProjects":["/gone"],"sections":[{"id":"x1","name":"Clients","projects":["/w"],"collapsed":false}]},"active":$view,"settings":$settings,"locale":"en","home":"/Users/me","cwd":"/w","blobBase":"http://127.0.0.1:7357/blobs"}}""",
    """{"type":"agents","agents":[{"id":"claude","name":"Claude Code"}]}""",
    """{"type":"sessions","sessions":[{"id":"s1","title":"t","agent":"claude","cwd":"/w","updatedAt":"2026-10-27T19:33:34.971Z"}]}""",
    """{"type":"categories","categories":{"categories":[],"collapsedProjects":[]}}""",
    """{"type":"noSession"}""",
    """{"type":"session","session":$view}""",
    """{"type":"sessionPatch","patch":{"id":"s1","base":1,"view":$view,"keep":3,"turns":[{"role":"agent","blocks":[]}],"keepBlocks":1,"blocks":[{"type":"text","markdown":"yz"}]}}""",
    """{"type":"sessionsPatch","patch":{"base":3,"rev":4,"size":2,"runs":[{"id":"s3","title":"t","agent":"claude","cwd":"/w","updatedAt":"2026-10-27T19:33:34.971Z"},[0,2]]}}""",
    """{"type":"accounts","accounts":[{"id":"acc-1","agent":"claude","label":"me@example.com","addedAt":"2026-10-27T19:33:28.658Z"}]}""",
    """{"type":"accountActions","actions":[{"agent":"claude","via":"login","status":"pending"}]}""",
    """{"type":"hidden","hidden":{"claude":{"effort":["low"]}}}""",
    """{"type":"settings","settings":$settings,"locale":"zh-CN"}""",
    """{"type":"controls","agent":"claude","controls":[{"id":"model","name":"Model","category":"model","options":[{"id":"opus","name":"Opus"}],"value":"opus"}],"shapes":{"opus":[{"id":"effort","name":"Effort","options":[{"id":"high","name":"High"}],"value":"high"}]},"commands":[{"name":"review","description":"Review the diff","input":{"hint":"path"}}],"cwd":"ssh://gen1/w"}""",
    """{"type":"sessionHits","seq":3,"hits":[{"id":"s1","snippet":"… the zebracorn …"}]}""",
    """{"type":"subagent","sessionId":"s1","subagentId":"n1","rev":3,"running":true,"turns":[{"role":"user","text":"look"},{"role":"agent","blocks":[{"type":"text","markdown":"found it"}]}]}""",
    """{"type":"files","seq":5,"files":[{"uri":"file:///w/src/app.ts","path":"src/app.ts"}]}""",
    """{"type":"diffSource","seq":10,"source":{"path":"/w/a.ts","oldText":"a\n","newText":"b\n"}}""",
    """{"type":"imageBlob","seq":11,"blob":"3f2a.png"}""",
    """{"type":"stagedFile","seq":12,"blob":"3f2a0b1c4d5e6f70.mp4","size":15874091,"error":"tooLarge"}""",
    """{"type":"sessionMarkdown","seq":4,"markdown":"# t\n","error":"unreadable"}""",
    """{"type":"projectDir","seq":6,"entries":[{"name":"src","dir":true},{"name":"a.ts"}],"error":"missing"}""",
    """{"type":"projectStatus","seq":7,"root":"/w","marks":{"src/a.ts":"M","new/":"U"}}""",
    """{"type":"projectFile","seq":8,"text":"b\n","hunks":[{"at":1,"add":1,"del":["a"]}],"error":"EACCES: permission denied"}""",
    """{"type":"sessionChanges","seq":9,"files":[{"path":"a.ts","add":1,"del":1,"gone":false}]}""",
    """{"type":"terminalOutput","id":"T","data":"term-42\r\n"}""",
    """{"type":"terminalState","id":"T","title":"sh","cols":100,"rows":30,"replay":"","exit":{"code":5,"signal":"SIGHUP"},"error":"no shell"}""",
    """{"type":"editTurnResult","requestId":"e1","error":"stale"}""",
    """{"type":"deleteSessionResult","requestId":"d1","undo":false,"error":"busy"}""",
    """{"type":"appearance","appearance":{"density":"airy","motion":"none"}}""",
    """{"type":"connections","connections":[{"alias":"gen1","disabled":false,"state":"error","error":{"message":"denied"}}]}""",
    """{"type":"localDir","seq":31,"path":"/Users/me/code","parent":"/Users/me","entries":[{"name":"acpira","dir":true,"git":true}],"error":"slow"}""",
    """{"type":"providers","view":{"providers":[{"id":"gw","name":"Gateway","preset":"custom","format":"openai-chat","baseUrl":"https://gw.example/v1","fullUrl":false,"enabled":true,"hasKey":true,"models":[],"headers":{"X-Team":"a"}}],"families":["generic"]},"error":"locked"}""",
    """{"type":"providerProbed","id":"p1","outcome":{"kind":"failed","error":"401"}}""",
  )
}

/**
 * Just enough of a TypeScript reader for the contract files: type aliases, the object members of a tagged union, and an
 * interface's field names (the shapes packages/shared writes, as apps/ios ContractTests reads them)
 */
private object TS {
  fun stripComments(source: String): String =
    source.replace(Regex("""/\*[\s\S]*?\*/"""), "").replace(Regex("""(?<![:'"\w])//[^\n]*"""), "")

  /** The text of `export type <name> = …;` after the `=`, or null */
  fun alias(source: String, name: String): String? {
    val start = Regex("""export type $name\s*=""").find(source) ?: return null
    return until(source, start.range.last + 1) { depth, c -> depth == 0 && c == ';' }
  }

  /** The members of a tagged union: tag literal → field names (the tag left out) */
  fun members(union: String, tag: String = "type"): Map<String, List<String>> {
    val out = linkedMapOf<String, List<String>>()
    var i = 0
    while (i < union.length) {
      if (union[i] == '{') {
        val body = until(union, i + 1) { depth, c -> depth == 0 && c == '}' }
        val fields = fields(body)
        val literal = Regex("""(?:^|[;\s{])$tag\s*:\s*'([^']+)'""").find(body)?.groupValues?.get(1)
        if (literal != null) out[literal] = fields.map { it.first }.filter { it != tag }
        i += body.length + 2
      } else {
        i++
      }
    }
    return out
  }

  /** The field names of `export interface <name> { … }`, or null */
  fun interfaceFields(source: String, name: String): List<String>? = interfaceBody(source, name)?.let { body -> fields(body).map { it.first } }

  /** The type of one field of an interface, as written */
  fun interfaceType(source: String, name: String, field: String): String? =
    interfaceBody(source, name)?.let { body -> fields(body).firstOrNull { it.first == field }?.second }

  /** The string literals of a union of them */
  fun literals(union: String): Set<String> = Regex("""'([^']*)'""").findAll(union).map { it.groupValues[1] }.toSet()

  private fun interfaceBody(source: String, name: String): String? {
    val start = Regex("""export interface $name\b[^{]*\{""").find(source) ?: return null
    return until(source, start.range.last + 1) { depth, c -> depth == 0 && c == '}' }
  }

  /** The top-level `name?: type` pairs of an object type's body */
  private fun fields(body: String): List<Pair<String, String>> {
    val parts = mutableListOf<String>()
    var depth = 0
    val current = StringBuilder()
    for ((i, c) in body.withIndex()) {
      when {
        c in "{([<" -> depth++
        c in "})]" -> depth--
        c == '>' && body.getOrNull(i - 1) != '=' -> depth--
      }
      if (depth == 0 && (c == ';' || c == '\n' || c == ',')) {
        parts += current.toString(); current.clear()
      } else {
        current.append(c)
      }
    }
    parts += current.toString()
    return parts.mapNotNull { part ->
      val m = Regex("""^\s*(?:readonly\s+)?['"]?([A-Za-z_$][\w$]*)['"]?\s*(\??)\s*:\s*([\s\S]*)$""").find(part) ?: return@mapNotNull null
      m.groupValues[1] to m.groupValues[3].trim()
    }
  }

  /** The text from [from] up to the first character [stop] accepts at its bracket depth */
  private fun until(text: String, from: Int, stop: (Int, Char) -> Boolean): String {
    var depth = 0
    for (i in from until text.length) {
      val c = text[i]
      if (stop(depth, c)) return text.substring(from, i)
      when {
        c in "{([<" -> depth++
        c in "})]" -> depth--
        c == '>' && text.getOrNull(i - 1) != '=' -> depth--
      }
    }
    return text.substring(from)
  }
}
