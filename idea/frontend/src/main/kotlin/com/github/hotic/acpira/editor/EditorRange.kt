package com.github.hotic.acpira.editor

import com.google.gson.JsonObject

// The webview's `EditorSelection` (src/shared/protocol.ts), built the same way as src/host/editorSelection.ts: 0-based lines in,
// 1-based inclusive lines out; a selection that ends at the start of a later line does not include that line; blank or oversized
// text (over MAX_TEXT_BYTES, which the engine would refuse) is not offered
object EditorRange {
    const val MAX_TEXT_BYTES = 1024 * 1024

    fun of(uri: String, startLine: Int, endLine: Int, endsAtLineStart: Boolean, text: String): JsonObject? {
        if (text.isBlank() || text.toByteArray(Charsets.UTF_8).size > MAX_TEXT_BYTES) return null
        val last = if (endsAtLineStart && endLine > startLine) endLine else endLine + 1
        return JsonObject().apply {
            addProperty("uri", uri)
            addProperty("startLine", startLine + 1)
            addProperty("endLine", last)
            addProperty("text", text)
        }
    }
}
