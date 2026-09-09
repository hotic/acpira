package com.github.hotic.acpira.settings

import com.google.gson.JsonElement
import com.google.gson.JsonNull
import com.google.gson.JsonObject
import com.intellij.openapi.components.Service
import com.intellij.openapi.components.service
import java.util.concurrent.CopyOnWriteArrayList

// The shell owns the settings store the sidecar reads a snapshot of: flat acpira.* keys (`defaultAgent`, `appearance.motion`, …) with
// JSON values, exactly what VS Code keeps under `acpira.*`. Application-wide, so every project's sidecar sees the same values.
// Persistence across IDE restarts is the next milestone; within one IDE session the settings page works end to end
@Service(Service.Level.APP)
class AcpiraSettings {
    private val values = JsonObject()
    private val listeners = CopyOnWriteArrayList<(keys: List<String>, snapshot: JsonObject) -> Unit>()

    @Synchronized
    fun snapshot(): JsonObject = values.deepCopy()

    @Synchronized
    fun write(key: String, value: JsonElement?) {
        if (value == null || value is JsonNull) values.remove(key) else values.add(key, value)
        val snap = values.deepCopy()
        listeners.forEach { it(listOf(key), snap) }
    }

    fun subscribe(fn: (keys: List<String>, snapshot: JsonObject) -> Unit): () -> Unit {
        listeners.add(fn)
        return { listeners.remove(fn) }
    }

    companion object {
        fun getInstance(): AcpiraSettings = service()
    }
}
