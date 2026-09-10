package com.github.hotic.acpira.sidecar

import com.google.gson.JsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test

class SidecarOutboxTest {
    private fun env(type: String, viewId: String? = null) = JsonObject().apply {
        addProperty("type", type)
        if (viewId != null) addProperty("viewId", viewId)
    }

    @Test fun `helloOk sends attachView before a ready that arrived during handshake`() {
        val box = SidecarOutbox()
        assertNull(box.offer(env("webviewMessage", "sidebar")))
        val attach = env("attachView", "sidebar")
        val ordered = box.flush(listOf(attach))
        assertEquals(listOf("attachView", "webviewMessage"), ordered.map { it.get("type").asString })
        assertTrue(box.ready)
        val later = env("webviewMessage", "sidebar")
        assertSame(later, box.offer(later))
    }

    @Test fun `a ready after reset queues again until the next helloOk`() {
        val box = SidecarOutbox()
        box.flush(emptyList())
        box.reset()
        assertFalse(box.ready)
        assertNull(box.offer(env("webviewMessage", "a")))
        assertEquals(listOf("attachView", "webviewMessage"), box.flush(listOf(env("attachView", "a"))).map { it.get("type").asString })
    }

    @Test fun `detach drops that view's queued envelopes only`() {
        val box = SidecarOutbox()
        box.offer(env("webviewMessage", "a"))
        box.offer(env("webviewMessage", "b"))
        box.dropView("a")
        assertEquals(listOf("b"), box.flush(emptyList()).map { it.get("viewId").asString })
    }
}
