package cc.hotic.acpira.store

import android.util.Base64
import cc.hotic.acpira.i18n.t
import cc.hotic.acpira.logic.TurnEdit
import cc.hotic.acpira.logic.attachmentName
import cc.hotic.acpira.logic.canEditPrompts
import cc.hotic.acpira.logic.editTurnRequest
import cc.hotic.acpira.logic.openTurnControls
import cc.hotic.acpira.logic.pendingPlanReview
import cc.hotic.acpira.logic.reviseOption
import cc.hotic.acpira.model.AgentTurn
import cc.hotic.acpira.model.AsyncTaskInfo
import cc.hotic.acpira.model.Attachment
import cc.hotic.acpira.model.CompactionBlock
import cc.hotic.acpira.model.CompactionStatus
import cc.hotic.acpira.model.Draft
import cc.hotic.acpira.model.FailureAction
import cc.hotic.acpira.model.GoalAction
import cc.hotic.acpira.model.GoalBlock
import cc.hotic.acpira.model.GoalEvent
import cc.hotic.acpira.model.GoalStatus
import cc.hotic.acpira.model.ModelPick
import cc.hotic.acpira.model.PermissionBlock
import cc.hotic.acpira.model.PlanDocumentBlock
import cc.hotic.acpira.model.QueuedPrompt
import cc.hotic.acpira.model.QuestionBlock
import cc.hotic.acpira.model.QuestionOutcome
import cc.hotic.acpira.model.SessionGoal
import cc.hotic.acpira.model.SessionStatus
import cc.hotic.acpira.model.SessionView
import cc.hotic.acpira.model.TextBlock
import cc.hotic.acpira.model.ToolCallBlock
import cc.hotic.acpira.model.ToolKind
import cc.hotic.acpira.model.ToolStatus
import cc.hotic.acpira.model.TurnStop
import cc.hotic.acpira.model.UserTurn
import cc.hotic.acpira.model.WebviewMsg
import cc.hotic.acpira.ui.Staged
import cc.hotic.acpira.logic.PastedText
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonElement
import java.util.UUID

// What the chat's interactive surfaces do (apps/ios Store/AppModel.swift "Cards in the transcript", "Queue", "Short
// stops", "Plans", "Session state", Store/AppModel+Turns.swift): with an engine each posts its message; the demo plays
// the engine's answer locally.

private fun AppModel.edit(id: String, change: (SessionView) -> SessionView) {
  views[id]?.let { views[id] = change(it) }
}

/** The newest agent turn of [id] rewritten by [change] */
private fun AppModel.editLastAgent(id: String, change: (AgentTurn) -> AgentTurn) = edit(id) { v ->
  val at = v.turns.indexOfLast { it is AgentTurn }
  if (at < 0) v else v.copy(turns = v.turns.toMutableList().also { it[at] = change(it[at] as AgentTurn) })
}

/**
 * Settings `steerQueued` on an agent that can steer, while a turn runs: a send joins the turn, and a queued row's send
 * steers it in (Shell.tsx `steering`)
 */
fun AppModel.steering(id: String): Boolean = settings.steerQueued && views[id]?.canSteer == true && views[id]?.running == true

// Questions

/** A question answered or skipped: it closes, and its card leaves the dock */
fun AppModel.answer(id: String, question: String, answers: Map<String, JsonElement>, skipped: Boolean) {
  if (post(WebviewMsg.Answer(id, question, if (skipped) emptyMap() else answers, skip = if (skipped) true else null))) return
  edit(id) { v ->
    v.copy(turns = v.turns.map { turn ->
      if (turn !is AgentTurn) turn else turn.copy(blocks = turn.blocks.map { b ->
        if (b is QuestionBlock && b.id == question) {
          b.copy(outcome = if (skipped) QuestionOutcome.skipped else QuestionOutcome.answered, answers = if (skipped) null else answers)
        } else b
      })
    })
  }
}

// Plans

/**
 * Build a saved plan (the plan card's Build): [option] answers its approval (the allow-once one when null), [model] is
 * the executor picked on the card. The demo goes on with a short build after the card
 */
fun AppModel.buildPlan(id: String, plan: String, option: String?, model: ModelPick?) {
  if (post(WebviewMsg.BuildPlan(id, plan, option, model))) return
  if (views[id]?.turns.isNullOrEmpty()) return
  model?.let { setConfig(id, it.configId, it.value) }
  settlePlan(id, plan, "executing")
  edit(id) { it.copy(running = true) }
  chat.scope.launch {
    delay(DEMO_STEP_MS)
    editLastAgent(id) { turn ->
      turn.copy(blocks = turn.blocks + ToolCallBlock(id = "demo-build-$plan", kind = ToolKind.edit, verb = t("verb.edit"), target = "README.md", targetMono = true, status = ToolStatus.completed))
    }
    delay(DEMO_STEP_MS)
    editLastAgent(id) { turn ->
      turn.copy(
        blocks = turn.blocks.map { if (it is PlanDocumentBlock && it.id == plan) it.copy(status = "approved") else it } + TextBlock(markdown = t("mobile.demo.build")),
        stop = TurnStop.end_turn,
        endedAt = now().toEpochMilli().toDouble(),
      )
    }
    edit(id) { it.copy(running = false) }
  }
}

/**
 * A send while a plan waits on its card: Revise answers the card and the words go out as the next prompt, so they reach
 * the agent after the rejection instead of steering a turn that is blocked on the card (Shell.tsx). True when it took
 * the send
 */
fun AppModel.revisePlan(id: String, text: String): Boolean {
  if (text.isEmpty()) return false
  val review = views[id]?.let { pendingPlanReview(it.turns) } ?: return false
  val revise = reviseOption(review) ?: return false
  if (post(WebviewMsg.Permission(id, review.id, revise.id))) {
    post(WebviewMsg.Send(id, text))
    return true
  }
  settlePlan(id, review.planId.orEmpty(), "rejected")
  return false
}

/**
 * Demo: the plan's approval answered. Its permission leaves, the plan takes [status], and a rejection ends the turn the
 * way claude-agent-acp does (cancelled, nothing said)
 */
private fun AppModel.settlePlan(id: String, plan: String, status: String) {
  editLastAgent(id) { turn ->
    val blocks = turn.blocks.mapNotNull { b ->
      when {
        b is PermissionBlock && b.planId == plan -> null
        b is PlanDocumentBlock && b.id == plan -> b.copy(status = status)
        else -> b
      }
    }
    if (status == "rejected") turn.copy(blocks = blocks, stop = TurnStop.cancelled, endedAt = now().toEpochMilli().toDouble())
    else turn.copy(blocks = blocks)
  }
  if (status == "rejected") edit(id) { it.copy(running = false) }
}

// Queue

/** Send now: the running turn stops and the queued prompt goes at once; with steering on it joins the turn instead */
fun AppModel.sendQueued(id: String, item: String) {
  if (steering(id) && post(WebviewMsg.SteerQueued(id, item))) return
  if (post(WebviewMsg.SendQueued(id, item))) return
  val prompt = takeQueued(id, item) ?: return
  if (views[id]?.running == true) stop(id)
  send(id, prompt.text)
}

/**
 * Edit: takes a prompt out of the queue and back into the composer, ahead of whatever is being typed there rather than
 * over it. The engine's queue row names the blobs it stored, which a send cannot hand back, so they are read back first
 * (AppModel.swift `editQueued`); while one cannot be read the prompt stays queued and a toast says so
 */
fun AppModel.editQueued(id: String, item: String) {
  val prompt = views[id]?.queued?.firstOrNull { it.id == item } ?: return
  chat.scope.launch {
    val staged = try {
      localCopies(id, prompt.attachments.orEmpty())
    } catch (e: CancellationException) {
      throw e
    } catch (_: Exception) {
      val stored = prompt.attachments.orEmpty().firstOrNull { storedBlob(it) != null }
      showToast(t("attach.loadFailed", "name" to (stored?.let(::attachmentName)?.ifEmpty { null } ?: t("common.image"))))
      return@launch
    }
    // Nothing to edit when the prompt went out during the read
    takeQueued(id, item) ?: return@launch
    val draft = chat.draft(id)
    draft.text = if (draft.text.isBlank()) prompt.text else prompt.text + "\n\n" + draft.text
    draft.staged = staged + draft.staged
  }
}

/**
 * A queued prompt's attachments as the composer stages them: every stored photo or text read back into its bytes,
 * base64 like a fresh pick or paste; files stay the links they are, quotes as they are. A selection has no draft on
 * this side and is left out, as a send leaves it out (AppModel.swift `drafts`)
 */
private suspend fun AppModel.localCopies(id: String, attachments: List<Attachment>): List<Staged> = attachments.mapNotNull { a ->
  when (a) {
    is Attachment.Image -> {
      val blob = a.blob ?: return@mapNotNull null
      val bytes = if (storedBlob(a) != null) media.loader.bytes(id, blob) else Base64.decode(blob, Base64.DEFAULT)
      val thumb = withContext(Dispatchers.Default) { MediaLoader.decodeNow(bytes)?.image }
      val name = a.name ?: t("common.image")
      Staged(Draft.Image(a.mimeType, if (storedBlob(a) != null) Base64.encodeToString(bytes, Base64.NO_WRAP) else blob, a.name), name, thumb)
    }
    is Attachment.Text -> {
      val text = PastedText.inline(a.blob) ?: a.blob?.takeIf { storedBlob(a) != null }?.let { media.loader.bytes(id, it).decodeToString() }
        ?: return@mapNotNull null
      Staged(Draft.Text(a.name, text, a.context), a.name)
    }
    is Attachment.File -> Staged(Draft.File(a.uri, a.name), a.name)
    is Attachment.Quote -> Staged(Draft.Quote(a.text, a.comment), "")
    is Attachment.Selection, is Attachment.Unknown -> null
  }
}

/** A blob the engine stored is named `<hash>.<ext>`; base64 never holds a dot */
private fun storedBlob(a: Attachment): String? = when (a) {
  is Attachment.Image -> a.blob
  is Attachment.Text -> a.blob
  else -> null
}?.takeIf { it.contains('.') }

/** Takes a prompt out of the queue (edit puts its text back in the composer, remove drops it) */
fun AppModel.takeQueued(id: String, item: String): QueuedPrompt? {
  val prompt = views[id]?.queued?.firstOrNull { it.id == item }
  if (post(WebviewMsg.Dequeue(id, item))) return prompt
  edit(id) { v -> v.copy(queued = v.queued?.filter { it.id != item }?.takeIf { it.isNotEmpty() }) }
  return prompt
}

// Short stops

/** Continue after a limit: a follow-up asking the agent to carry on */
fun AppModel.carryOn(id: String) = send(id, t("alert.continueText"))

/** /compact, offered when the context no longer fits; the demo plays the compaction as its own automatic turn */
fun AppModel.compact(id: String) {
  if (post(WebviewMsg.Compact(id))) return
  val view = views[id] ?: return
  if (view.running) return
  val started = now().toEpochMilli().toDouble()
  val prompt = UserTurn(text = "/compact", auto = true, autoReason = "compact")
  val block = CompactionBlock("demo-compact-${started.toLong()}", status = CompactionStatus.in_progress)
  views[id] = view.copy(turns = view.turns + prompt + AgentTurn(blocks = listOf(block), startedAt = started), running = true)
  chat.scope.launch {
    delay(DEMO_STEP_MS * 3)
    editLastAgent(id) { it.copy(blocks = listOf(block.copy(status = CompactionStatus.completed)), endedAt = now().toEpochMilli().toDouble(), stop = TurnStop.end_turn) }
    edit(id) { it.copy(running = false) }
  }
}

// Session state

/** The session's agent started again (the session card's retry): after an error, or a sign-in finished on the computer */
fun AppModel.restart(id: String) {
  if (!post(WebviewMsg.Retry(id))) settleStatus(id)
}

/** Sign in with a method the engine runs itself, without a terminal on the computer */
fun AppModel.login(id: String, method: String) {
  if (!post(WebviewMsg.Login(id, method))) settleStatus(id)
}

/** Take the session over from the Acpira instance that holds it */
fun AppModel.takeOver(id: String) {
  if (!post(WebviewMsg.TakeOverSession(id))) settleStatus(id)
}

/** "Continue in a new session": the new-session screen preset to this agent and project, as the bar's new button */
fun AppModel.continueNew(id: String) {
  val view = views[id] ?: return
  push(Screen.New(agent = view.agent, cwd = view.cwd.takeUnless { isChat(it) }))
}

/**
 * An AIR failure notice's remedy: a real turn retries its prompt, a session-scoped failure (no turn started) restarts
 * the session; sign-in belongs to the session card
 */
fun AppModel.failureAction(id: String, action: FailureAction) {
  when (action) {
    FailureAction.retry -> if ((views[id]?.turns?.lastOrNull() as? AgentTurn)?.startedAt != null) retryTurn(id) else restart(id)
    FailureAction.new_session -> continueNew(id)
    FailureAction.login, FailureAction.unknown -> {}
  }
}

// Background tasks

/**
 * Stop on a background task's row. The engine marks the stop asked at once and the adapter's state update settles the
 * row; a refusal comes back as the engine's error toast with the row live again. The demo stops it a beat later
 */
fun AppModel.stopTask(id: String, task: String) {
  if (post(WebviewMsg.StopAsyncTask(id, task))) return
  updateTask(id, task) { it.copy(stopRequested = true) }
  chat.scope.launch {
    // Long enough to read "Stopping…"
    delay(DEMO_STOP_MS)
    updateTask(id, task) { it.copy(stopRequested = null, state = "stopped") }
  }
}

/**
 * Stop on a running command's row: the engine ends that command's process and the agent's own update settles the row
 * as failed, the turn going on; nothing found comes back as the engine's error toast. The demo fails it at once
 */
fun AppModel.stopCommand(id: String, toolCallId: String) {
  if (post(WebviewMsg.StopCommand(id, toolCallId))) return
  val end = now().toEpochMilli().toDouble()
  edit(id) { v ->
    v.copy(turns = v.turns.map { turn ->
      if (turn !is AgentTurn) turn
      else turn.copy(blocks = turn.blocks.map { b ->
        if (b is ToolCallBlock && b.id == toolCallId && b.status == ToolStatus.in_progress) b.copy(status = ToolStatus.failed, endedAt = end) else b
      })
    })
  }
}

/** Rewrites the task on every row that carries it */
private fun AppModel.updateTask(id: String, task: String, change: (AsyncTaskInfo) -> AsyncTaskInfo) = edit(id) { v ->
  v.copy(turns = v.turns.map { turn ->
    if (turn !is AgentTurn) turn
    else turn.copy(blocks = turn.blocks.map { b ->
      val info = (b as? ToolCallBlock)?.asyncTask
      if (info != null && info.id == task) b.copy(asyncTask = change(info)) else b
    })
  })
}

private const val DEMO_STOP_MS = 1_000L

/** Demo: whatever held the session back is resolved */
private fun AppModel.settleStatus(id: String) = edit(id) { it.copy(status = SessionStatus.ready, error = null, authMethods = null, canTakeOver = null) }

// Goal

/**
 * A goal control. The engine sends the turn-starting ones as a `/goal …` prompt; the demo plays the agent's new snapshot
 * and its milestone row on the last agent turn (normalize.rs apply_goal)
 */
fun AppModel.goal(id: String, action: GoalAction, objective: String? = null) {
  if (post(WebviewMsg.Goal(id, action, objective))) return
  val view = views[id] ?: return
  var goal = view.goal
  val event = when (action) {
    GoalAction.set -> {
      val text = objective?.trim()?.takeIf { it.isNotEmpty() } ?: return
      goal = SessionGoal(objective = text, status = GoalStatus.active, timeUsedSeconds = 0.0)
      GoalEvent.set
    }
    GoalAction.pause -> { goal = goal?.copy(status = GoalStatus.paused); GoalEvent.paused }
    GoalAction.resume -> { goal = goal?.copy(status = GoalStatus.active); GoalEvent.resumed }
    GoalAction.clear -> GoalEvent.cleared
    GoalAction.unknown -> return
  }
  val snapshot = goal ?: view.goal
  val row = GoalBlock(event, snapshot?.objective, snapshot?.iterations, snapshot?.tokensUsed, snapshot?.timeUsedSeconds)
  val last = view.turns.lastOrNull() as? AgentTurn
  val turns = if (last != null) view.turns.dropLast(1) + last.copy(blocks = last.blocks + row)
  else view.turns + AgentTurn(blocks = listOf(row), stop = TurnStop.end_turn)
  views[id] = view.copy(turns = turns, goal = if (action == GoalAction.clear) null else goal)
}

// Accounts

/** The account sheet of [id] comes up (the header menu's account entry, `popup=account`) */
fun AppModel.openAccount(id: String) {
  chat.accountSheet = id
}

fun AppModel.selectAccount(id: String, account: String) {
  if (post(WebviewMsg.SelectAccount(id, account))) return
  edit(id) { it.copy(accountId = account) }
  summaries = summaries.map { if (it.id == id) it.copy(accountId = account) else it }
  chat.accounts.firstOrNull { it.id == account }?.label?.let { showToast(t("mobile.account.switched", "name" to it)) }
}

/** An agent's accounts came into view: the engine refreshes their quotas */
fun AppModel.refreshQuota(agent: String) {
  post(WebviewMsg.RefreshQuota(agent))
}

fun AppModel.removeAccount(account: String) {
  if (!post(WebviewMsg.RemoveAccount(account))) chat.accounts = chat.accounts.filter { it.id != account }
}

// Prompt edits

/**
 * Takes the sent prompt at [index] into the chat's composer: its text and attachments, and the selections it went out
 * with as the composer's local snapshot. What the composer held waits until the edit is sent or dropped
 */
fun AppModel.beginEdit(id: String, index: Int) {
  val view = views[id] ?: return
  if (!canEditPrompts(view)) return
  val prompt = view.turns.getOrNull(index) as? UserTurn ?: return
  if (prompt.auto == true) return
  val draft = chat.draft(id)
  // Taking in another prompt keeps the draft from before the first
  val kept = draft.edit?.let { it.keptText to it.keptStaged } ?: (draft.text to draft.staged)
  // The prompt's own attachments stand in the composer as they were sent (a stored photo's tile, a text that opens, a
  // file that opens); each keeps its index for the retained list (AppModel+Turns.swift `beginEdit`)
  val originals = prompt.attachments.orEmpty().mapIndexed { i, a -> Staged(standIn(a), attachmentName(a), source = a) to i }
  draft.edit = TurnEdit(
    turnIndex = index, turnCount = view.turns.size, original = prompt,
    controls = openTurnControls(view.controls, prompt.settings, view.modelShapes),
    keptText = kept.first, keptStaged = kept.second, originals = originals,
  )
  draft.text = prompt.text
  draft.staged = originals.map { it.first }
}

/**
 * What a sent attachment taken in for editing carries as a draft. An edit sends it back by index, never as this draft,
 * which only tells the strip its kind: a photo by its blob, a text by its inline contents, a file or selection by its uri
 */
private fun standIn(a: Attachment): Draft = when (a) {
  is Attachment.Image -> Draft.Image(a.mimeType, a.blob.orEmpty(), a.name)
  is Attachment.Text -> Draft.Text(a.name, PastedText.inline(a.blob).orEmpty(), a.context)
  is Attachment.File -> Draft.File(a.uri, a.name)
  is Attachment.Selection -> Draft.File(a.uri, a.name)
  is Attachment.Quote -> Draft.Quote(a.text, a.comment)
  is Attachment.Unknown -> Draft.File("", a.kind)
}

/** `popup=edit`: the newest prompt the user typed, taken in for editing (screenshots) */
fun AppModel.beginEditLast(id: String) {
  val turns = views[id]?.turns ?: return
  val at = turns.indexOfLast { it is UserTurn && it.auto != true }
  if (at >= 0) beginEdit(id, at)
}

/** The edit dropped: the composer gets back what it held */
fun AppModel.cancelEdit(id: String) {
  val draft = chat.draft(id)
  val edit = draft.edit ?: return
  draft.edit = null
  draft.text = edit.keptText
  draft.staged = edit.keptStaged
}

/**
 * Send from the editing composer: the turn and everything after it give way to the rewrite. The engine answers with
 * `editTurnResult`; until then the composer stays empty, and a refusal puts the rewrite back. The demo drops the later
 * turns and replays its reply, as a send does
 */
fun AppModel.sendEdit(id: String) {
  val draft = chat.draft(id)
  val edit = draft.edit ?: return
  val text = draft.text
  val staged = draft.staged
  draft.edit = null
  draft.text = ""
  draft.staged = emptyList()
  if (live) {
    val request = UUID.randomUUID().toString()
    draft.pending = PendingEdit(request, edit, text, staged)
    post(WebviewMsg.EditTurn(request, editTurnRequest(id, edit, text, staged) { it.draft }))
    return
  }
  val view = views[id] ?: return
  if (edit.turnIndex > view.turns.size) return
  // The engine applies the edit's selections to the live session before the prompt goes out
  views[id] = view.copy(turns = view.turns.take(edit.turnIndex), controls = edit.controls)
  // The kept originals go again too: their stand-ins name the same blobs, which the demo's bubble reads
  send(id, text, staged.map { it.draft })
  restoreKept(id, edit)
}

/**
 * The engine's answer to an edit: a refusal names why in a toast and puts the rewrite back (drafts survive a rejected
 * edit), an accepted one gives the composer its earlier draft. Either only into an untouched composer
 */
fun AppModel.editAnswered(request: String, error: String?) {
  val (id, draft) = chat.draftAwaiting(request) ?: return
  val pending = draft.pending ?: return
  draft.pending = null
  if (error != null) {
    showToast(error)
    if (draft.text.isNotEmpty() || draft.staged.isNotEmpty() || draft.edit != null) return
    draft.edit = pending.edit
    draft.text = pending.text
    draft.staged = pending.staged
  } else {
    restoreKept(id, pending.edit)
  }
}

private fun AppModel.restoreKept(id: String, edit: TurnEdit<Staged>) {
  val draft = chat.draft(id)
  if (draft.text.isNotEmpty() || draft.staged.isNotEmpty() || draft.edit != null) return
  draft.text = edit.keptText
  draft.staged = edit.keptStaged
}

private const val DEMO_STEP_MS = 700L
