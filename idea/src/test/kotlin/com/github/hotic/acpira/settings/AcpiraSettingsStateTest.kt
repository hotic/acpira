package com.github.hotic.acpira.settings

import com.intellij.openapi.util.JDOMUtil
import com.intellij.util.xmlb.XmlSerializer
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

// The state document round-trips through the platform's XML serializer, and a hand-written options file in that shape loads; a damaged
// document reads as empty instead of failing
class AcpiraSettingsStateTest {
    @Test fun `state round-trips through the XML serializer and tracks modifications`() {
        val json = """{"agents":{"echo":{"command":"x"}},"uiFontSize":15}"""
        val state = AcpiraSettings.State()
        val before = state.modificationCount
        state.json = json
        assertTrue(state.modificationCount > before)
        val element = XmlSerializer.serialize(state)
        assertEquals("""<State><option name="json" value="$json" /></State>""", JDOMUtil.write(element).replace("&quot;", "\"").replace(Regex("\\s*\\n\\s*"), ""))
        assertEquals(json, XmlSerializer.deserialize(element, AcpiraSettings.State::class.java).json)
    }

    @Test fun `an options file in the documented shape deserializes`() {
        val xml = """<component name="Acpira"><option name="json" value="{&quot;a&quot;:1}" /></component>"""
        assertEquals("""{"a":1}""", XmlSerializer.deserialize(JDOMUtil.load(xml), AcpiraSettings.State::class.java).json)
    }

    @Test fun `a damaged document reads as empty`() {
        assertEquals(0, AcpiraSettings.parse("{ nope").size())
        assertEquals(0, AcpiraSettings.parse("[1,2]").size())
        assertEquals(0, AcpiraSettings.parse(null).size())
        assertEquals(1, AcpiraSettings.parse("""{"a":1}""").get("a").asInt)
    }
}
