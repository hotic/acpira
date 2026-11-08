package cc.hotic.acpira.store

import android.content.Context
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import cc.hotic.acpira.logic.TurnEdit
import cc.hotic.acpira.model.AccountInfo
import cc.hotic.acpira.model.WireJson
import cc.hotic.acpira.ui.Staged
import kotlinx.coroutines.MainScope
import kotlinx.serialization.builtins.ListSerializer

/**
 * The chat's own state beside the engine's views (apps/ios AppModel's `drafts`, `accounts`): each session's unsent
 * draft and the prompt being rewritten in it, the stored logins with their quotas, the account sheet on screen. Held by
 * the app model ([AppModel.chat]) and carried to the model of a new wire to the same computer
 */
class ChatState(context: Context, demo: Boolean) {
  private val drafts = HashMap<String, ChatDraft>()

  /** The unsent prompt and attachments of one session: leaving the chat and coming back keeps them */
  fun draft(id: String): ChatDraft = drafts.getOrPut(id) { ChatDraft() }

  /** Drafts waiting on the engine's answer to an edit, by the edit's request id */
  internal fun draftAwaiting(request: String): Pair<String, ChatDraft>? =
    drafts.entries.firstOrNull { it.value.pending?.request == request }?.let { it.key to it.value }

  /** Every stored login the engine lists (HostMsg.Accounts); the demo's fixture accounts without one */
  var accounts by mutableStateOf(if (demo) demoAccounts(context) else emptyList())

  /** The session whose account sheet is up (NativeAccountSheet), null while none is */
  var accountSheet by mutableStateOf<String?>(null)

  /** The demo's own timers (a build going on after its plan card) */
  internal val scope = MainScope()

  /** A new wire to the same computer keeps the drafts and the logins the screens showed */
  fun adopt(from: ChatState) {
    drafts.putAll(from.drafts)
    accounts = from.accounts
  }

  private fun demoAccounts(context: Context): List<AccountInfo> = runCatching {
    val text = context.assets.open("fixtures/accounts.json").bufferedReader().use { it.readText() }
    WireJson.decodeFromString(ListSerializer(AccountInfo.serializer()), text)
  }.getOrDefault(emptyList())
}

/** One session's composer contents (apps/ios ComposerDraft) */
@Stable
class ChatDraft {
  var text by mutableStateOf("")
  var staged by mutableStateOf<List<Staged>>(emptyList())
  /** A sent prompt being rewritten in the composer (PromptEdit.swift); null while the composer writes a new one */
  var edit by mutableStateOf<TurnEdit<Staged>?>(null)
  /** An edit sent and not answered yet: a refusal puts it back */
  internal var pending: PendingEdit? = null
  /** The `draft` launch extra went in once */
  internal var seeded = false
}

/** An edit on its way: the request, the edit and what the composer held when it was sent */
internal data class PendingEdit(val request: String, val edit: TurnEdit<Staged>, val text: String, val staged: List<Staged>)
