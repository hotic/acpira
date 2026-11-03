package cc.hotic.acpira.sync

import cc.hotic.acpira.model.ClientInfo
import cc.hotic.acpira.model.Hello
import cc.hotic.acpira.model.HostMsg
import cc.hotic.acpira.model.ShellEnv
import cc.hotic.acpira.model.WebviewMsg
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.consumeAsFlow
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.take
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.yield
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertIs
import kotlin.test.assertNotNull
import kotlin.test.assertTrue

/** The envelope order the engine relies on, against a transport the test feeds by hand */
class EngineClientTest {
  private class FakeTransport : EngineTransport {
    val incoming = Channel<String>(Channel.UNLIMITED)
    val sent = mutableListOf<String>()
    override fun frames(): Flow<String> = incoming.consumeAsFlow()
    override fun send(text: String): Boolean { sent += text; return true }
    override fun close() { incoming.close() }
    fun types() = sent.map { Json.parseToJsonElement(it).jsonObject["type"]!!.jsonPrimitive.content }
  }

  private val hello = Hello(requestId = "h1", client = ClientInfo("test", "0"), env = ShellEnv(hostLanguage = "en"))

  @Test
  fun helloFirstThenAttachThenQueuedActions() = runTest {
    val wire = FakeTransport()
    val client = EngineClient(wire, hello)
    client.start(backgroundScope)
    // Posted before the engine answered: held until the view is attached
    client.post(WebviewMsg.Ready())
    client.post(WebviewMsg.Send("s1", "hi"))
    yield()
    assertEquals(listOf("hello"), wire.types())

    wire.incoming.send("""{"type":"helloOk","requestId":"h1","protocolVersion":1,"sessionsDir":"/tmp"}""")
    assertIs<EngineEvent.Connected>(client.events.first())
    yield()
    assertEquals(listOf("hello", "attachView", "webviewMessage", "webviewMessage"), wire.types())
    val send = Json.parseToJsonElement(wire.sent[3]).jsonObject["message"]!!.jsonObject
    assertEquals("send", send["type"]!!.jsonPrimitive.content)
    // A session action carries a clientId, so a resend after a lost link runs once
    assertNotNull(send["clientId"])
  }

  @Test
  fun hostMessagesOfThisViewOnlyAndUnknownRequestsAnswered() = runTest {
    val wire = FakeTransport()
    val client = EngineClient(wire, hello)
    client.start(backgroundScope)
    // Two envelopes in one frame, split on "\n"
    wire.incoming.send(
      """{"type":"helloOk","requestId":"h1","protocolVersion":1}""" + "\n" +
        """{"type":"hostMessage","viewId":"other","message":{"type":"noSession"}}""",
    )
    wire.incoming.send("""{"type":"hostMessage","viewId":"phone","message":{"type":"noSession"}}""")
    wire.incoming.send("""{"type":"platformRequest","requestId":"r1","request":{"method":"openFile","path":"/x"}}""")
    wire.incoming.send("""{"type":"platformRequest","request":{"method":"toast","level":"info","text":"hey"}}""")
    val events = client.events.take(3).toList()
    assertIs<EngineEvent.Connected>(events[0])
    assertEquals(HostMsg.NoSession, (events[1] as EngineEvent.Host).message)
    assertIs<EngineEvent.Platform>(events[2])
    val answer = wire.sent.map { Json.parseToJsonElement(it).jsonObject }.single { it["type"]!!.jsonPrimitive.content == "platformResponse" }
    assertEquals("r1", answer["requestId"]!!.jsonPrimitive.content)
    assertTrue(answer.containsKey("error"))
  }

  @Test
  fun aClosedWireEndsTheEvents() = runTest {
    val wire = FakeTransport()
    val client = EngineClient(wire, hello)
    client.start(backgroundScope)
    wire.incoming.close()
    assertEquals(listOf<EngineEvent>(EngineEvent.Closed(null)), client.events.toList())
  }

  @Test
  fun anUnansweredHelloGivesTheWireUp() = runTest {
    val wire = FakeTransport()
    val client = EngineClient(wire, hello, helloTimeoutMs = 1_000)
    client.start(backgroundScope)
    val closed = client.events.first()
    assertIs<EngineEvent.Closed>(closed)
    assertTrue(closed.reason.orEmpty().contains("did not answer"), "${closed.reason}")
    assertEquals(listOf("hello"), wire.types())
  }

  @Test
  fun aLostWireHandsOverWhatItMayNotHaveDelivered() = runTest {
    val wire = FakeTransport()
    val client = EngineClient(wire, hello)
    client.start(backgroundScope)
    wire.incoming.send("""{"type":"helloOk","requestId":"h1","protocolVersion":1}""")
    client.events.first()
    // Written just before the wire died: the engine may never have read it
    client.post(WebviewMsg.Send("s1", "written"))
    client.post(WebviewMsg.SelectSession("s1"))
    yield()
    wire.incoming.close()
    client.events.toList()
    // Posted after the end: never written
    client.post(WebviewMsg.Permission("s1", "b1", "allow"))
    val unsent = client.unsent()
    // Session actions only, oldest first, each keeping the clientId it was given so the engine runs it once
    assertEquals(listOf("send", "permission"), unsent.map { it.type })
    val written = Json.parseToJsonElement(wire.sent.last { it.contains("written") }).jsonObject["message"]!!.jsonObject
    assertEquals(written["clientId"]!!.jsonPrimitive.content, unsent[0].clientId)
  }
}
