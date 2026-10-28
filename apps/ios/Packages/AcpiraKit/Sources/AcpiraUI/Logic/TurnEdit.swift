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
/// live controls for the live model, then a family sibling's shape; an unknown model keeps the snapshot
private func reshape(_ fallback: SessionControls, _ settings: TurnSettings, live: SessionControls, shapes: ModelShapes?) -> SessionControls {
  guard let model = live.options.first(where: isModelControl), let value = settings.config[model.id],
        model.options.contains(where: { $0.id == value }) else { return fallback }
  let shape = shapes?[value] ?? (value == model.value ? nil : shapeFor(shapes, model: model, value: value))
  if let shape {
    var shaped = live
    shaped.options = live.options.filter(isModelControl) + shape
    return controlsForTurn(shaped, settings)
  }
  return value == model.value ? controlsForTurn(live, settings) : fallback
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
  guard let control = live.options.first(where: { $0.id == id }), isModelControl(control) else { return next }
  return reshape(next, captureTurnSettings(next), live: live, shapes: shapes)
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

/// The editTurn request for a send from the editing composer. Attachments staged during the edit stay on the phone, as
/// they do for a plain send, until the composer hands drafts to the engine
func editTurnRequest(sessionId: String, edit: TurnEdit, text: String, staged: [Attachment]) -> EditTurnRequest {
  EditTurnRequest(sessionId: sessionId, turnIndex: edit.turnIndex, turnCount: edit.turnCount, originalText: edit.original.text,
                  turnId: edit.original.id, text: text.trimmingCharacters(in: .whitespacesAndNewlines),
                  retainedAttachments: retainedAttachments(edit.original.attachments ?? [], staged: staged), attachments: [],
                  settings: captureTurnSettings(edit.controls))
}

/// The turn a reply's actions offer Retry on: the newest one, when it ended on an error Retry can answer (Alert.tsx):
/// not a context that no longer fits (compaction is the way on), and not a failure whose declared remedies leave retry out
func retryableTurn(_ session: SessionView) -> Int? {
  guard let turn = shortStop(session), turn.stop == .error, !isContextLengthError(turn.error),
        turn.error?.actions.map({ $0.contains(.retry) }) ?? true else { return nil }
  return session.turns.count - 1
}
