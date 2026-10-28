import AcpiraModels
import Foundation

// chat/planReview.ts and chat/planSections.ts: the plan approval the newest turn waits on, its Revise choice, and a
// turn cut at each saved plan, so the plan card stands between the work before it and the continuation after it.

/// reviseOption: the reject-once option that keeps the agent planning. Kimi's reject-and-exit is another decision and
/// stays among the card's extra approvals
func reviseOption(_ permission: PermissionBlock) -> PermissionOption? {
  permission.options.first {
    $0.kind == .reject_once && $0.id.range(of: "exit", options: .caseInsensitive) == nil && !$0.label.contains("退出")
  }
}

/// pendingPlanReview: the plan approval the newest agent turn is blocked on. Its card carries the decision, so while it
/// waits the turn reads as finished and the composer's send revises the plan instead of queueing behind it
func pendingPlanReview(_ turns: [Turn]) -> PermissionBlock? {
  guard case .agent(let turn)? = turns.last else { return nil }
  for case .permission(let p) in turn.blocks where p.planId != nil { return p }
  return nil
}

/// One stretch of a turn: the blocks up to a saved plan, and that plan
struct PlanSection: Identifiable {
  /// "start", then the id of the plan the section follows: stable while chunks stream, so folds keep their state
  var key: String
  var blocks: [AgentBlock]
  var plan: PlanDocumentBlock?
  var id: String { key }
}

/// splitPlanSections: a plan is a chronological boundary outside both neighbouring process folds. The trailing section
/// stays even while empty, since the approval and the continuation arrive there. A question's own tool row is dropped
/// (its card holds the request and the answer) unless it failed, and so are thoughts with no text.
/// A delegation row (a tool call with `subagentId`) is not a boundary and never draws as a tool: when its node is in
/// `placed` it stays, once per node, as the point the subagent rows render at (`processParts`); the others drop
func splitPlanSections(_ blocks: [AgentBlock], placed: Set<String> = []) -> [PlanSection] {
  var sections = [PlanSection(key: "start", blocks: [])]
  var seen = Set<String>()
  let questionTools = Set(blocks.compactMap { block -> String? in if case .question(let q) = block { q.toolCallId } else { nil } })
  for block in blocks {
    switch block {
    case .toolCall(let call) where call.status != .failed && questionTools.contains(call.id):
      continue
    case .toolCall(let call) where call.subagentId != nil:
      if let id = call.subagentId, placed.contains(id), seen.insert(id).inserted {
        sections[sections.count - 1].blocks.append(block)
      }
    case .thought(let thought) where thought.text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty:
      continue
    case .planDocument(let plan):
      sections[sections.count - 1].plan = plan
      sections.append(PlanSection(key: plan.id, blocks: []))
    default:
      sections[sections.count - 1].blocks.append(block)
    }
  }
  return sections
}

/// The plan card's name and body. The engine names a plan after its first `# heading` ("Plan" when it has none); that
/// heading is dropped from the body, the card is the plan itself
func planHeading(_ block: PlanDocumentBlock) -> (title: String, body: String) {
  var markdown = Substring(block.markdown)
  while let first = markdown.first, first.isWhitespace { markdown = markdown.dropFirst() }
  let title = !block.title.isEmpty && block.title != "Plan" ? block.title : t("plan.title")
  if let h1 = markdown.prefixMatch(of: /#[ \t]+(.+?)[ \t]*#*[ \t]*(?:\r?\n|$)/), String(h1.1) == block.title {
    return (title, String(markdown[h1.range.upperBound...]).trimmingCharacters(in: .whitespacesAndNewlines))
  }
  return (title, String(markdown).trimmingCharacters(in: .whitespacesAndNewlines))
}

/// The model a plan is built with (PlanDocument.tsx): picked on the card until Build, so choosing an executor never
/// changes the model still writing the plan. `selected` wins while the control offers it
struct PlanExecutor {
  /// The session's model control (`category: model`), absent when the agent offers none
  var control: ConfigControl?
  /// The option the build runs on
  var value: String?
  var family: ModelFamily?
  var variant: ModelVariant?
  /// The chip's faint parameters: the variant's effort, Fast and 1M, then the session's reasoning level
  var meta: String?

  init(controls: SessionControls, selected: String?) {
    let control = controls.options.first { $0.category == "model" }
    self.control = control
    let value = control.flatMap { c in selected.flatMap { s in c.options.contains { $0.id == s } ? s : nil } ?? c.value }
    self.value = value
    let family = control.flatMap { c in ModelCatalog.group(c.options).first { f in f.variants.contains { $0.id == value } } }
    self.family = family
    let variant = family?.variants.first { $0.id == value }
    self.variant = variant
    let reasoning = ComposerControls.split(controls.options).reasoning
    let tags = ComposerControls.chipTags(family, variant, reasoning: reasoning, modelConfig: [], standard: t("composer.standard"))
    meta = tags.isEmpty ? nil : tags.map(\.label).joined(separator: " ")
  }

  /// What the chip names: the family, else the option's own name
  var label: String? { family?.name ?? control?.options.first { $0.id == value }?.name }

  /// The build's model pick, sent along with Build (the engine switches only when it differs)
  var pick: ModelPick? { control.flatMap { c in value.map { ModelPick(configId: c.id, value: $0) } } }
}

/// A variant's name inside its family's menu: the effort (Standard for the level-less one), Fast and 1M
func variantName(_ v: ModelVariant) -> String {
  if v.lead != nil { return ModelCatalog.fusionLabel(v) }
  let parts = [v.effort.isEmpty ? t("composer.standard") : v.effort, v.fast ? "Fast" : "", v.long ? "1M" : ""]
  return parts.filter { !$0.isEmpty }.joined(separator: " ")
}
