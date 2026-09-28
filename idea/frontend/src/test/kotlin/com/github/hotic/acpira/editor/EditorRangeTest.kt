package com.github.hotic.acpira.editor

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

// Mirrors test/editorSelection.test.ts: both shells must label the same range the same way
class EditorRangeTest {
    @Test
    fun linesBecomeOneBasedAndInclusive() {
        val r = EditorRange.of("file:///w/a.ts", 11, 18, false, "x")!!
        assertEquals(12, r.get("startLine").asInt)
        assertEquals(19, r.get("endLine").asInt)
    }

    @Test
    fun aSelectionEndingAtALineStartLeavesThatLineOut() {
        assertEquals(19, EditorRange.of("file:///w/a.ts", 11, 19, true, "x")!!.get("endLine").asInt)
        // Unless it is the only line
        assertEquals(12, EditorRange.of("file:///w/a.ts", 11, 11, true, "x")!!.get("endLine").asInt)
    }

    @Test
    fun blankAndOversizedTextIsNotOffered() {
        assertNull(EditorRange.of("file:///w/a.ts", 0, 0, false, "  \n"))
        assertNull(EditorRange.of("file:///w/a.ts", 0, 0, false, "x".repeat(EditorRange.MAX_TEXT_BYTES + 1)))
    }
}
