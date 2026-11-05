import AcpiraModels
import Foundation

// Editing a sent prompt (HistoryMessage.tsx HistoryEditor, shared/turnSettings.ts, shared/modelShapes.ts) and the retry
// a failed reply offers (Alert.tsx). The phone edits in the chat's own composer: the prompt and its attachments move
// into it, its pickers act on a local snapshot of the selections the prompt went out with, and send rewrites the turn.

/// modelShapes.ts isModelControl
func isModelControl(_ c: ConfigControl) -> Bool { c.category == "model" || (c.category == nil && c.id == "model") }

/// shapeFor: a model's remembered dependent controls; an id never seen borrows a sibling's from its family
func shapeFor(_ shapes: ModelShapes?, model: ConfigControl, value: String) -> [ConfigControl]? {
  guard let shapes else { return nil }
  if let shape = shapes[value] { return shape }
  let family = ModelCatalog.group(model.options).first { $0.variants.contains { $0.id == value } }
  return family?.variants.lazy.compactMap { shapes[$0.id] }.first
}

/// captureTurnSettings: wire ids, never display names
func captureTurnSettings(_ controls: SessionControls) -> TurnSettings {
  TurnSettings(modeId: controls.modeId,
               config: Dictionary(controls.options.compactMap { c in c.value.map { (c.id, $0) } }, uniquingKeysWith: { a, _ in a }))
}

/// controlsForTurn: a selection the controls no longer offer (a CLI update) yields to the current one, never to a guess
func controlsForTurn(_ controls: SessionControls, _ settings: TurnSettings?) -> SessionControls {
  var out = controls
  if let mode = settings?.modeId, controls.modes.contains(where: { $0.id == mode }) { out.modeId = mode }
  out.options = controls.options.map { c in
    var c = c
    if let value = settings?.config[c.id], c.options.contains(where: { $0.id == value }) { c.value = value }
    return c
  }
  return out
}

/// reshape: the editor never calls set_config_option, so a model's dependent controls come from that model itself. Its
/// learned shape first (the live view can pair a model with another's parameters while a pick is in flight), then the
/// live controls for the live model, then a family sibling's shape; an unknown model drops unverified model parameters
private func reshape(_ fallback: SessionControls, _ settings: TurnSettings, live: SessionControls, shapes: ModelShapes?) -> SessionControls {
  guard let model = live.options.first(where: isModelControl), let value = settings.config[model.id],
        model.options.contains(where: { $0.id == value }) else { return fallback }
  let shape = shapes?[value] ?? (value == model.value ? nil : shapeFor(shapes, model: model, value: value))
  if let shape {
    var shaped = live
    shaped.options = live.options.filter(isModelControl) + shape
    return controlsForTurn(shaped, settings)
  }
  if value == model.value { return controlsForTurn(live, settings) }
  // A draft has no agent to correct an old scale. Until this model's shape is known, do not offer another model's
  // effort or Fast as though it belonged to the selected model.
  var next = fallback
  next.options.removeAll { ComposerControls.isReasoning($0) || $0.category == "model_config" }
  return next
}

/// openTurnControls: the editor's selections when it opens on a sent prompt
func openTurnControls(_ live: SessionControls, _ settings: TurnSettings?, shapes: ModelShapes?) -> SessionControls {
  let fallback = controlsForTurn(live, settings)
  guard let settings else { return fallback }
  return reshape(fallback, settings, live: live, shapes: shapes)
}

/// editTurnConfig: one pick in the editor; a model pick brings that model's own parameters along
func editTurnConfig(_ editor: SessionControls, live: SessionControls, id: String, value: String, shapes: ModelShapes?) -> SessionControls {
  var next = editor
  next.options = editor.options.map { c in
    var c = c
    if c.id == id { c.value = value }
    return c
  }
  guard let control = live.options.first(where: { $0.id == id }) else { return next }
  if control.category == ComposerControls.sidekickCategory { return seatSidekickEffort(next, sidekick: value, shapes: shapes) }
  guard isModelControl(control) else { return next }
  return reshape(next, captureTurnSettings(next), live: live, shapes: shapes)
}

/// The built-in agent offers the sidekick's level select only once a sidekick is set, so a pick nothing answers (a new
/// session's draft, the editor) takes it from the picked model's learned shape: its levels, the level held while still
/// offered, else the one the model last ran at. None, or a model never seen, drops it. Its Fast goes either way: no
/// learned shape says whether the new model has a fast tier, and the engine refuses a switch it does not offer
private func seatSidekickEffort(_ controls: SessionControls, sidekick: String, shapes: ModelShapes?) -> SessionControls {
  var next = controls
  next.options.removeAll { $0.category == ComposerControls.sidekickFastCategory }
  let held = controls.options.first { $0.category == ComposerControls.sidekickEffortCategory }
  next.options.removeAll { $0.category == ComposerControls.sidekickEffortCategory }
  guard sidekick != ComposerControls.noSidekick, let model = controls.options.first(where: isModelControl),
        let scale = shapeFor(shapes, model: model, value: sidekick)?.first(where: ComposerControls.isReasoning),
        scale.options.count > 1 else { return next }
  var effort = held ?? ConfigControl(id: "sidekick_effort", name: "Sidekick effort", category: ComposerControls.sidekickEffortCategory)
  effort.options = scale.options
  effort.value = held?.value.flatMap { v in scale.options.contains { $0.id == v } ? v : nil } ?? scale.value
  let at = next.options.firstIndex { $0.category == ComposerControls.sidekickCategory }.map { $0 + 1 } ?? next.options.endIndex
  next.options.insert(effort, at: at)
  return next
}

/// A sent prompt being rewritten in the composer. `turnCount` is the transcript's length when the edit began, so the
/// engine refuses the edit as stale once the conversation moved on; `keptText` / `keptStaged` are what the composer held
/// before, back once the edit is sent or dropped
struct TurnEdit: Equatable {
  var turnIndex: Int
  var turnCount: Int
  var original: UserTurn
  var controls: SessionControls
  var keptText = ""
  var keptStaged: [Attachment] = []
}

/// Prompts can be rewritten while the session is idle and usable: not running, not starting a held-back prompt
/// (heldPrompt.ts: a queued head while nothing runs reads as sent), and not a ChatGPT mirror
func canEditPrompts(_ session: SessionView) -> Bool {
  session.external == nil && !session.running && (session.status == .ready || session.status == .starting)
    && (session.queued ?? []).isEmpty
}

/// The prompt's attachments still staged, by their index in the prompt: each staged copy keeps one
func retainedAttachments(_ original: [Attachment], staged: [Attachment]) -> [Int] {
  var left = staged
  return original.indices.filter { i in
    guard let at = left.firstIndex(of: original[i]) else { return false }
    left.remove(at: at)
    return true
  }
}

/// The editTurn request for a send from the editing composer: the prompt's attachments still staged go by index, the
/// ones staged during the edit as drafts, as a plain send hands them over (`AppModel.drafts`)
func editTurnRequest(sessionId: String, edit: TurnEdit, text: String, staged: [Attachment]) -> EditTurnRequest {
  let original = edit.original.attachments ?? []
  let retained = retainedAttachments(original, staged: staged)
  var added = staged
  for i in retained { if let at = added.firstIndex(of: original[i]) { added.remove(at: at) } }
  return EditTurnRequest(sessionId: sessionId, turnIndex: edit.turnIndex, turnCount: edit.turnCount, originalText: edit.original.text,
                         turnId: edit.original.id, text: text.trimmingCharacters(in: .whitespacesAndNewlines),
                         retainedAttachments: retained, attachments: AppModel.drafts(added),
                         settings: captureTurnSettings(edit.controls))
}

/// The turn a reply's actions offer Retry on: the newest one, when it ended on an error Retry can answer (Alert.tsx):
/// not a context that no longer fits (compaction is the way on), and not a failure whose declared remedies leave retry out
func retryableTurn(_ session: SessionView) -> Int? {
  guard let turn = shortStop(session), turn.stop == .error, !isContextLengthError(turn.error),
        turn.error?.actions.map({ $0.contains(.retry) }) ?? true else { return nil }
  return session.turns.count - 1
}
