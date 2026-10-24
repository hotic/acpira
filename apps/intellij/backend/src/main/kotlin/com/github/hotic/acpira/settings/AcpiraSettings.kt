package com.github.hotic.acpira.settings

import com.google.gson.JsonElement
import com.google.gson.JsonNull
import com.google.gson.JsonObject
import com.google.gson.JsonParser
import com.intellij.openapi.components.BaseState
import com.intellij.openapi.components.RoamingType
import com.intellij.openapi.components.Service
import com.intellij.openapi.components.SimplePersistentStateComponent
import com.intellij.openapi.components.State
import com.intellij.openapi.components.Storage
import com.intellij.openapi.components.service
import java.util.concurrent.CopyOnWriteArrayList

// The shell owns the settings store the sidecar reads a snapshot of: flat acpira.* keys (`defaultAgent`, `appearance.motion`, …) with
// JSON values, exactly what VS Code keeps under `acpira.*`. Application-wide (every project's sidecar sees the same values) and
// persisted by the IDE in options/acpira.xml as one JSON document, so arbitrary value shapes need no XML mapping. Not roamed: the
// values reference machine-local things (agent binaries, custom agent commands)
@Service(Service.Level.APP)
@State(name = "Acpira", storages = [Storage("acpira.xml", roamingType = RoamingType.DISABLED)])
class AcpiraSettings : SimplePersistentStateComponent<AcpiraSettings.State>(State()) {
    class State : BaseState() {
        var json by string("{}")
    }

    private val listeners = CopyOnWriteArrayList<(keys: List<String>, snapshot: JsonObject) -> Unit>()

    @Synchronized
    fun snapshot(): JsonObject = parse(state.json)

    fun write(key: String, value: JsonElement?) {
        val snap = synchronized(this) {
            val values = parse(state.json)
            if (value == null || value is JsonNull) values.remove(key) else values.add(key, value)
            state.json = values.toString()
            values
        }
        listeners.forEach { it(listOf(key), snap.deepCopy()) }
    }

    fun subscribe(fn: (keys: List<String>, snapshot: JsonObject) -> Unit): () -> Unit {
        listeners.add(fn)
        return { listeners.remove(fn) }
    }

    companion object {
        fun getInstance(): AcpiraSettings = service()

        // A hand-edited or damaged options file must not take the whole plugin down; it reads as empty and is rewritten on the next change
        fun parse(json: String?): JsonObject =
            runCatching { JsonParser.parseString(json ?: "{}") }.getOrNull()?.takeIf { it.isJsonObject }?.asJsonObject ?: JsonObject()
    }
}
