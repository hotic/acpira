package com.github.hotic.acpira.sidecar

import com.google.gson.JsonObject

// Handshake, attachView, and ordinary sends share one queue so a page's one-shot `ready` cannot overtake attachView once helloOk
// flips the connection live. Drain is a single method: prefix (attach envelopes) then whatever arrived while hello was in flight
internal class SidecarOutbox {
    private val queued = ArrayList<JsonObject>()
    var ready = false
        private set

    fun offer(envelope: JsonObject): JsonObject? {
        if (!ready) {
            queued.add(envelope)
            return null
        }
        return envelope
    }

    fun flush(prefix: List<JsonObject>): List<JsonObject> {
        val ordered = ArrayList<JsonObject>(prefix.size + queued.size)
        ordered.addAll(prefix)
        ordered.addAll(queued)
        queued.clear()
        ready = true
        return ordered
    }

    fun reset() { ready = false }

    fun dropView(viewId: String) {
        queued.removeIf { it.get("viewId")?.asString == viewId }
    }
}
