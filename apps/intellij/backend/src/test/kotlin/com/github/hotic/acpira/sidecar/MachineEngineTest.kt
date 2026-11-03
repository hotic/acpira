package com.github.hotic.acpira.sidecar

import com.google.gson.JsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import java.io.IOException
import java.net.StandardProtocolFamily
import java.net.UnixDomainSocketAddress
import java.nio.ByteBuffer
import java.nio.channels.ServerSocketChannel
import java.nio.charset.StandardCharsets
import java.nio.file.Files
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit

class MachineEngineTest {
    @Test fun `macOS and Linux reach the machine engine, Windows and ACPIRA_ENGINE=stdio keep the child process`() {
        val none: (String) -> String? = { null }
        assertTrue(MachineEngine.wanted(none, "Mac OS X"))
        assertTrue(MachineEngine.wanted(none, "Linux"))
        assertFalse(MachineEngine.wanted(none, "Windows 11"))
        val stdio: (String) -> String? = { if (it == "ACPIRA_ENGINE") "stdio" else null }
        assertFalse(MachineEngine.wanted(stdio, "Mac OS X"))
        assertFalse(MachineEngine.wanted(stdio, "Linux"))
    }

    @Test fun `the last stdout line names the socket, anything else is an error`() {
        val reached = MachineEngine.parse("landing\n{\"socket\":\"/u/.acpira/run/engine.sock\",\"version\":\"2.0.7-dev.1\",\"pid\":123}\n\n")
        assertEquals(MachineEngine.Reached("/u/.acpira/run/engine.sock", "2.0.7-dev.1", 123L), reached)
        for (bad in listOf("", "not json", "{\"version\":\"1\"}", "{\"socket\":1}")) {
            try { MachineEngine.parse(bad); fail("parsed $bad") } catch (_: SidecarSetupException) {}
        }
    }

    // SidecarService asks `acpira engine` again on an IOException from connecting: a vanished socket and a stale one with no listener must both be one
    @Test fun `a socket that is gone or no longer listened on fails to connect with an IOException`() {
        val dir = Files.createTempDirectory("acpira-eng")
        try {
            val path = dir.resolve("e.sock")
            for (setup in listOf({}, { ServerSocketChannel.open(StandardProtocolFamily.UNIX).use { it.bind(UnixDomainSocketAddress.of(path)) } })) {
                setup()
                try { EngineConnection(path, {}, {}); fail("connected to $path") } catch (_: IOException) {}
            }
        } finally { dir.toFile().deleteRecursively() }
    }

    @Test fun `envelopes go both ways over the socket and the engine closing it reports an exit once`() {
        val dir = Files.createTempDirectory("acpira-eng")
        val path = dir.resolve("e.sock")
        try {
            ServerSocketChannel.open(StandardProtocolFamily.UNIX).use { server ->
                server.bind(UnixDomainSocketAddress.of(path))
                val got = LinkedBlockingQueue<JsonObject>()
                val exits = LinkedBlockingQueue<String>()
                val conn = EngineConnection(path, { got.add(it) }, { exits.add(it) })
                server.accept().use { peer ->
                    conn.send(JsonObject().apply { addProperty("type", "hello") })
                    val buf = ByteBuffer.allocate(256)
                    while (!String(buf.array(), 0, buf.position(), StandardCharsets.UTF_8).endsWith("\n")) peer.read(buf)
                    assertEquals("{\"type\":\"hello\"}\n", String(buf.array(), 0, buf.position(), StandardCharsets.UTF_8))
                    peer.write(StandardCharsets.UTF_8.encode("\n{\"type\":\"helloOk\"}\n"))
                    assertEquals("helloOk", got.poll(5, TimeUnit.SECONDS)?.get("type")?.asString)
                    assertTrue(conn.alive)
                }
                assertEquals("connection closed", exits.poll(5, TimeUnit.SECONDS))
                assertFalse(conn.alive)
                conn.stop()
                assertEquals(null, exits.poll(200, TimeUnit.MILLISECONDS))
            }
        } finally { dir.toFile().deleteRecursively() }
    }
}
