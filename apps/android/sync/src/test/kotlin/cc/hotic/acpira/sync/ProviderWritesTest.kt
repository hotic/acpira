package cc.hotic.acpira.sync

import cc.hotic.acpira.model.Provider
import cc.hotic.acpira.model.ProviderAction
import cc.hotic.acpira.model.ProviderModel
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertSame

/** packages/ui/settings/providerWrites.test.ts, the cases the phone relies on */
class ProviderWritesTest {
  private fun save(id: String, enabled: Boolean, key: String? = null) = ProviderAction.Save(
    Provider.new(id, "openai-chat", "https://example.test/v1", fullUrl = false).with("id", id)
      .withModels(listOf(ProviderModel.new("model").enabled(enabled))),
    key,
  )

  @Test fun coalescesConsecutiveTogglesButNeverDropsACredentialSave() {
    val sent = mutableListOf<ProviderAction>()
    val writes = ProviderWrites({ sent += it }, {})
    val first = save("a", false); val stale = save("a", true); val latest = save("a", false)
    val credentials = save("a", true, "test-key")
    writes.push(first, true)
    writes.push(stale, true)
    writes.push(latest, true)
    writes.push(credentials)
    writes.push(stale, true)
    while (writes.pending) writes.receive()
    assertEquals(listOf<ProviderAction>(first, latest, credentials, stale), sent)
  }

  @Test fun keepsTheLatestDraftAcrossOlderReplies() {
    val sent = mutableListOf<ProviderAction>()
    val writes = ProviderWrites({ sent += it }, {})
    val first = save("a", false); val second = save("b", false); val latest = save("a", true)
    writes.push(first, true)
    writes.push(second, true)
    writes.push(latest, true)
    assertEquals(true, writes.drafts["a"]?.models?.single()?.enabled)
    assertSame(first, writes.receive())
    assertSame(second, writes.receive())
    assertNull(writes.drafts["b"])
    assertSame(latest, writes.receive())
    assertEquals(emptyMap(), writes.drafts)
    assertFalse(writes.pending)
  }

  @Test fun holdsAWriteBehindAPendingRead() {
    val sent = mutableListOf<ProviderAction>()
    var reads = 0
    val writes = ProviderWrites({ sent += it }, { reads++ })
    writes.refresh()
    writes.refresh()
    val action = save("a", false)
    writes.push(action, true)
    assertEquals(1, reads)
    assertEquals(emptyList(), sent)
    assertNull(writes.receive())
    assertEquals(listOf<ProviderAction>(action), sent)
    writes.refresh()
    assertEquals(1, reads)
    assertSame(action, writes.receive())
    writes.refresh()
    assertEquals(2, reads)
  }
}
