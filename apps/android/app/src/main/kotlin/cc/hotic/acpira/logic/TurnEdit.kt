package cc.hotic.acpira.logic

import cc.hotic.acpira.model.Attachment
import cc.hotic.acpira.model.ConfigControl
import cc.hotic.acpira.model.Draft
import cc.hotic.acpira.model.EditTurnRequest
import cc.hotic.acpira.model.ModelShapes
import cc.hotic.acpira.model.SessionControls
import cc.hotic.acpira.model.SessionStatus
import cc.hotic.acpira.model.SessionView
import cc.hotic.acpira.model.TurnSettings
import cc.hotic.acpira.model.UserTurn

// Editing a sent prompt (apps/ios Logic/TurnEdit.swift; packages/shared/turnSettings.ts and modelShapes.ts). The phone
// edits in the chat's own composer: the prompt and its attachments move into it, its pickers act on a local snapshot of
// the selections the prompt went out with, and send rewrites the turn. Turn edits never call setConfig.

/** modelShapes.ts isModelControl */
fun isModelControl(c: ConfigControl): Boolean = c.category == "model" || (c.category == null && c.id == "model")

/** shapeFor: a model's remembered dependent controls; an id never seen borrows a sibling's from its family */
fun shapeFor(shapes: ModelShapes?, model: ConfigControl, value: String): List<ConfigControl>? {
  if (shapes == null) return null
  shapes[value]?.let { return it }
  val family = ModelCatalog.group(model.options) { null }.firstOrNull { f -> f.variants.any { it.id == value } }
  return family?.variants?.firstNotNullOfOrNull { shapes[it.id] }
}

/** captureTurnSettings: wire ids, never display names */
fun captureTurnSettings(controls: SessionControls): TurnSettings =
  TurnSettings(modeId = controls.modeId, config = controls.options.mapNotNull { c -> c.value?.let { c.id to it } }.distinctBy { it.first }.toMap())

/** controlsForTurn: a selection the controls no longer offer (a CLI update) yields to the current one, never to a guess */
fun controlsForTurn(controls: SessionControls, settings: TurnSettings?): SessionControls {
  val mode = settings?.modeId?.takeIf { m -> controls.modes.any { it.id == m } } ?: controls.modeId
  return controls.copy(
    modeId = mode,
    options = controls.options.map { c ->
      val value = settings?.config?.get(c.id)
      if (value != null && c.options.any { it.id == value }) c.copy(value = value) else c
    },
  )
}

/**
 * reshape: the editor never calls set_config_option, so a model's dependent controls come from that model itself. Its
 * learned shape first (the live view can pair a model with another's parameters while a pick is in flight), then the
 * live controls for the live model, then a family sibling's shape; an unknown model drops unverified model parameters
 */
private fun reshape(fallback: SessionControls, settings: TurnSettings, live: SessionControls, shapes: ModelShapes?): SessionControls {
  val model = live.options.firstOrNull(::isModelControl) ?: return fallback
  val value = settings.config[model.id] ?: return fallback
  if (model.options.none { it.id == value }) return fallback
  val shape = shapes?.get(value) ?: if (value == model.value) null else shapeFor(shapes, model, value)
  if (shape != null) return controlsForTurn(live.copy(options = live.options.filter(::isModelControl) + shape), settings)
  if (value == model.value) return controlsForTurn(live, settings)
  // Until this model's shape is known, do not offer another model's effort or Fast as though it belonged to it
  return fallback.copy(options = fallback.options.filterNot { ComposerControls.isReasoning(it) || it.category == "model_config" })
}

/** openTurnControls: the editor's selections when it opens on a sent prompt */
fun openTurnControls(live: SessionControls, settings: TurnSettings?, shapes: ModelShapes?): SessionControls {
  val fallback = controlsForTurn(live, settings)
  return if (settings == null) fallback else reshape(fallback, settings, live, shapes)
}

/** editTurnConfig: one pick in the editor; a model pick brings that model's own parameters along */
fun editTurnConfig(editor: SessionControls, live: SessionControls, id: String, value: String, shapes: ModelShapes?): SessionControls {
  val next = editor.copy(options = editor.options.map { if (it.id == id) it.copy(value = value) else it })
  val control = live.options.firstOrNull { it.id == id } ?: return next
  if (control.category == ComposerControls.sidekickCategory) return seatSidekickEffort(next, value, shapes)
  return if (isModelControl(control)) reshape(next, captureTurnSettings(next), live, shapes) else next
}

/**
 * The built-in agent offers the sidekick's level select only once a sidekick is set, so a pick nothing answers (a new
 * session's draft, the editor) takes it from the picked model's learned shape: its levels, the level held while still
 * offered, else the one the model last ran at. None, or a model never seen, drops it. Its Fast goes either way: no
 * learned shape says whether the new model has a fast tier, and the engine refuses a switch it does not offer
 */
private fun seatSidekickEffort(controls: SessionControls, sidekick: String, shapes: ModelShapes?): SessionControls {
  val held = controls.options.firstOrNull { it.category == ComposerControls.sidekickEffortCategory }
  val rest = controls.options.filter { it.category != ComposerControls.sidekickFastCategory && it.category != ComposerControls.sidekickEffortCategory }
  val next = controls.copy(options = rest)
  val model = controls.options.firstOrNull(::isModelControl)
  if (sidekick == ComposerControls.noSidekick || model == null) return next
  val scale = shapeFor(shapes, model, sidekick)?.firstOrNull { ComposerControls.isReasoning(it) }
  if (scale == null || scale.options.size <= 1) return next
  val effort = (held ?: ConfigControl(id = "sidekick_effort", name = "Sidekick effort", category = ComposerControls.sidekickEffortCategory)).copy(
    options = scale.options,
    value = held?.value?.takeIf { v -> scale.options.any { it.id == v } } ?: scale.value,
  )
  val at = rest.indexOfFirst { it.category == ComposerControls.sidekickCategory }.let { if (it < 0) rest.size else it + 1 }
  return next.copy(options = rest.toMutableList().apply { add(at, effort) })
}

/**
 * A sent prompt being rewritten in the composer. [turnCount] is the transcript's length when the edit began, so the
 * engine refuses the edit as stale once the conversation moved on; [keptText] / [keptStaged] are what the composer held
 * before, back once the edit is sent or dropped. [originals] are the prompt's own attachments as the composer holds
 * them: staged entries the edit made for them, by their index in the prompt. [K] is the composer's staged item
 */
data class TurnEdit<K>(
  val turnIndex: Int,
  val turnCount: Int,
  val original: UserTurn,
  val controls: SessionControls,
  val keptText: String = "",
  val keptStaged: List<K> = emptyList(),
  val originals: List<Pair<K, Int>> = emptyList(),
)

/**
 * Prompts can be rewritten while the session is idle and usable: not running, not starting a held-back prompt
 * (heldPrompt.ts: a queued head while nothing runs reads as sent)
 */
fun canEditPrompts(session: SessionView): Boolean =
  !session.running && (session.status == SessionStatus.ready || session.status == SessionStatus.starting) && session.queued.isNullOrEmpty()

/**
 * The editTurn request for a send from the editing composer: the prompt's attachments still staged go by index, the
 * ones staged during the edit as drafts, as a plain send hands them over
 */
fun <K> editTurnRequest(sessionId: String, edit: TurnEdit<K>, text: String, staged: List<K>, draftOf: (K) -> Draft): EditTurnRequest {
  val retained = edit.originals.filter { (item, _) -> staged.any { it === item } }.map { it.second }.sorted()
  val added = staged.filter { item -> edit.originals.none { it.first === item } }
  return EditTurnRequest(
    sessionId = sessionId,
    turnIndex = edit.turnIndex,
    turnCount = edit.turnCount,
    originalText = edit.original.text,
    turnId = edit.original.id,
    text = text.trim(),
    retainedAttachments = retained,
    attachments = added.map(draftOf),
    settings = captureTurnSettings(edit.controls),
  )
}

/** The session as the editing composer reads it: the edit's own selections, nothing running to stop, no context ring */
fun editingView(session: SessionView, controls: SessionControls): SessionView = session.copy(controls = controls, running = false, usage = null)

/** A prompt attachment's name as the composer's staged chip shows it */
fun attachmentName(a: Attachment): String = when (a) {
  is Attachment.Image -> a.name ?: ""
  is Attachment.Text -> a.name
  is Attachment.File -> a.name
  is Attachment.Selection -> "${a.name}:${a.startLine}"
  is Attachment.Quote -> a.text
  is Attachment.Unknown -> a.kind
}
