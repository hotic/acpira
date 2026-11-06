package cc.hotic.acpira.model

import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonObject
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertIs
import kotlin.test.assertNull
import kotlin.test.assertTrue

/** The localDir and model-source messages as the engine writes and reads them */
class ProvidersTest {
  private fun host(line: String): HostMsg = HostMsg.decode(WireJson, WireJson.parseToJsonElement(line).jsonObject)

  @Test fun aLocalDirListingDecodes() {
    val m = assertIs<HostMsg.LocalDir>(host("""
      {"type":"localDir","seq":3,"path":"/Users/dev","parent":"/Users","entries":[{"name":"acpira","dir":true,"git":true},{"name":"notes","dir":true}]}
    """))
    assertEquals(3, m.seq)
    assertEquals("/Users", m.parent)
    assertEquals(listOf(DirEntry("acpira", true, true), DirEntry("notes", true)), m.entries)
    assertNull(m.error)
    // A root has no parent, a failed listing only its error
    val root = assertIs<HostMsg.LocalDir>(host("""{"type":"localDir","seq":4,"path":"C:\\","entries":[],"error":"denied"}"""))
    assertNull(root.parent)
    assertEquals("denied", root.error)
  }

  @Test fun aLocalDirRequestLeavesAnAbsentPathOut() {
    assertEquals("""{"type":"localDir","seq":1}""", WebviewMsg.LocalDir(null, 1).toJson().toString())
    assertEquals("""{"type":"localDir","path":"~/src","seq":2}""", WebviewMsg.LocalDir("~/src", 2).toJson().toString())
    assertEquals("""{"type":"categoryOp","op":"addProject","cwd":"/srv/app"}""", WebviewMsg.AddProject("/srv/app").toJson().toString())
  }

  @Test fun aChildPathFollowsTheListedSeparator() {
    assertEquals("/Users/dev/acpira", childPath("/Users/dev", "acpira"))
    assertEquals("/srv", childPath("/", "srv"))
    assertEquals("C:\\Users\\dev", childPath("C:\\Users", "dev"))
    assertEquals("C:\\Users", childPath("C:\\", "Users"))
    assertEquals("dev", folderName("C:\\Users\\dev"))
    assertEquals("/", folderName("/"))
  }

  @Test fun anUnknownProviderFieldSurvivesAnEdit() {
    val m = assertIs<HostMsg.Providers>(host("""
      {"type":"providers","view":{"families":["generic"],"providers":[
        {"id":"ds","name":"DeepSeek","preset":"custom","format":"openai-chat","baseUrl":"https://api.deepseek.com","fullUrl":false,
         "enabled":true,"hasKey":true,"headers":{"x-a":"1"},"future":{"n":2},
         "models":[{"id":"chat","enabled":true,"input":["text"],"thinking":"auto","efforts":[],"sampling":{"topK":5},"estimated":[],"maxSteps":9}]}
      ]}}
    """))
    val p = m.view.providers.single()
    assertTrue(p.hasKey)
    val edited = p.with("name", "DS").withModels(p.models.map { it.enabled(false) })
    val sent = WebviewMsg.ProviderAction(ProviderAction.Save(edited)).toJson()
    val provider = sent["action"]!!.jsonObject["provider"]!!.jsonObject
    assertEquals("""{"n":2}""", provider["future"].toString())
    assertEquals("""{"x-a":"1"}""", provider["headers"].toString())
    assertFalse("hasKey" in provider)
    assertEquals("DS", provider.string("name"))
    val model = (provider["models"] as kotlinx.serialization.json.JsonArray).single().jsonObject
    assertEquals("false", model["enabled"].toString())
    assertEquals("9", model["maxSteps"].toString())
    assertEquals("""{"topK":5}""", model["sampling"].toString())
    // Absent key keeps the stored one
    assertFalse("key" in sent["action"]!!.jsonObject)
  }

  @Test fun aNewSourceAndATypedModelCarryTheFileDefaults() {
    val p = Provider.new("Gateway", "anthropic", "https://gw.test/v1", fullUrl = false).withModels(listOf(ProviderModel.new("m1")))
    val sent = WebviewMsg.ProviderAction(ProviderAction.Save(p, key = "k")).toJson().toString()
    assertEquals(
      """{"type":"providerAction","action":{"kind":"save","provider":{"id":"","name":"Gateway","preset":"custom","format":"anthropic",""" +
        """"baseUrl":"https://gw.test/v1","fullUrl":false,"enabled":true,"models":[{"id":"m1","enabled":true,"input":["text"],""" +
        """"thinking":"auto","efforts":[],"sampling":{},"estimated":[]}]},"key":"k"}}""",
      sent,
    )
    assertEquals("""{"type":"providerAction","action":{"kind":"delete","id":"ds"}}""", WebviewMsg.ProviderAction(ProviderAction.Delete("ds")).toJson().toString())
  }

  @Test fun probesAndTheirOutcomes() {
    assertEquals("""{"type":"providerProbe","id":"p1","probe":{"kind":"local"}}""", WebviewMsg.ProviderProbe("p1", ProviderProbe.Local).toJson().toString())
    val local = assertIs<ProbeOutcome.Local>(assertIs<HostMsg.ProviderProbed>(host("""
      {"type":"providerProbed","id":"p1","outcome":{"kind":"local","servers":[{"preset":"ollama","name":"Ollama","baseUrl":"http://127.0.0.1:11434/v1","models":[{"id":"qwen3"}]}]}}
    """)).outcome)
    val source = Provider.fromLocal(local.servers.single())
    assertEquals("ollama", source.preset)
    assertTrue(source.local)
    assertEquals(listOf("qwen3"), source.models.map { it.id })
    assertEquals(ProbeOutcome.Failed("401"), ProbeOutcome.decode(WireJson.parseToJsonElement("""{"kind":"failed","error":"401"}""")))
    assertEquals(ProbeOutcome.Other("test"), ProbeOutcome.decode(WireJson.parseToJsonElement("""{"kind":"test","ms":3,"text":"hi"}""")))
    assertTrue(sameUrl("http://127.0.0.1:11434/v1/", "HTTP://127.0.0.1:11434/v1"))
  }

  @Test fun theEndpointASourceCalls() {
    assertEquals("https://a.test/v1/chat/completions", endpointOf("https://a.test/v1/", "openai-chat", false))
    assertEquals("https://a.test/v1/messages", endpointOf("https://a.test/v1", "anthropic", false))
    assertEquals("https://a.test/v1/x", endpointOf(" https://a.test/v1/x ", "anthropic", true))
    assertTrue(looksFull("https://a.test/v1/Responses/"))
    assertFalse(looksFull("https://a.test/v1"))
    assertEquals(JsonObject(emptyMap()), Provider(JsonObject(emptyMap())).forWire())
  }
}
