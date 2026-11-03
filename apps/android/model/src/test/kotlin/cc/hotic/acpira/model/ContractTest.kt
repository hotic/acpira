package cc.hotic.acpira.model

import java.io.File
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

/**
 * Pins the Kotlin mirror to the TypeScript sources by reading the repository, as apps/ios ContractTests does: every
 * literal this build sends or reads must exist in packages/shared, so a rename there fails here first
 */
class ContractTest {
  private val root = File(System.getProperty("acpira.root") ?: "../..")
  private fun source(path: String) = File(root, path).readText()

  @Test fun protocolVersionMatchesTheSidecarContract() {
    val v = Regex("""SIDECAR_PROTOCOL_VERSION\s*=\s*(\d+)""").find(source("packages/shared/sidecar.ts"))?.groupValues?.get(1)
    assertEquals(SidecarProtocol.VERSION.toString(), v)
  }

  @Test fun everyMessageTypeExistsInProtocolTs() {
    val protocol = source("packages/shared/protocol.ts")
    for (type in WebviewMsg.types + HostMsg.types) {
      assertTrue(protocol.contains("type: '$type'"), "protocol.ts has no type '$type'")
    }
  }

  @Test fun everyBlockTypeExistsInTranscriptTs() {
    val transcript = source("packages/shared/transcript.ts")
    for (type in AgentBlock.S.types) assertTrue(transcript.contains("type: '$type'"), "transcript.ts has no block '$type'")
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
    val sessions = WireJson.decodeFromString(kotlinx.serialization.builtins.ListSerializer(SessionSummary.serializer()), File(dir, "sessions.json").readText())
    assertTrue(sessions.isNotEmpty())
  }

  @Test fun aSendCarriesItsFieldsAsTheEngineReadsThem() {
    val json = WebviewMsg.Send("s1", "hi", clientId = "c").toJson().toString()
    assertEquals("""{"type":"send","sessionId":"s1","text":"hi","clientId":"c"}""", json)
  }

  // The engine's Hello has no defaults for these; WireJson leaves default values out, so each must be forced on
  @Test fun aHelloCarriesEveryFieldTheEngineRequires() {
    val hello = ShellMsg.HelloMsg(Hello(requestId = "h", client = ClientInfo("a", "1"), env = ShellEnv(hostLanguage = "en"))).toJson()
    for (key in listOf("type", "protocolVersion", "requestId", "client", "env")) assertTrue(key in hello, "hello misses $key: $hello")
    assertEquals(SidecarProtocol.VERSION.toString(), hello["protocolVersion"].toString())
  }
}
