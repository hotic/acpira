import AcpiraDesign
import AcpiraModels
import SwiftUI

// The user's prompt card (Turns.tsx UserMessage + Attachments.tsx TurnAttachments), the permission card
// (Permission.tsx + permissionOptions.ts) and the row under a finished reply (TurnActions.tsx).

// MARK: - User message

/// A sent prompt: a right-aligned bubble on the chip fill, no outline; attachments above it, also right-aligned. The
/// bubble's right edge meets the prose's (the agent turn's --pad). A long press opens its menu: Edit (the desktop's
/// click on the card, while the session is idle), Copy, and on the newest typed prompt Set as goal (Goal.tsx
/// PromptFooter) where the agent takes one
struct UserMessageCard: View {
  let turn: UserTurn
  /// The prompt's place in the transcript; nil for a prompt steered into a turn, which is not rewritten on its own
  var index: Int?
  var editable = false
  var goal = false
  @Environment(\.sessionActions) private var actions

  var body: some View {
    if turn.auto != true {
      let shown = turn.text.trimmingTrailingWhitespace()
      let attachments = turn.attachments ?? []
      let shape = RoundedRectangle(cornerRadius: Metrics.rBubble)
      let edit = editable ? index : nil
      VStack(alignment: .trailing, spacing: Metrics.s2) {
        // At its own width, so the row sits on the bubble's right edge
        if !attachments.isEmpty { TurnAttachments(attachments: attachments).fixedSize() }
        if !shown.isEmpty {
          Text(web: shown)
            .textStyle(WebType.text1)
            .foregroundStyle(Palette.fgStrong)
            .lineLimit(Int(Metrics.userMessageMax / WebType.text1.lineHeight))
            .fixedSize(horizontal: false, vertical: true)
            .padding(.horizontal, Metrics.s3 + Metrics.s0_5)
            .padding(.vertical, Metrics.s2_5)
            .background(shape.fill(Palette.bg0).overlay(shape.fill(Palette.bubbleBg)))
        }
      }
      .modifier(PromptMenu(show: edit != nil || !shown.isEmpty, shape: shape) {
        if let edit {
          Button(t("history.edit"), systemImage: "pencil") { actions.editPrompt(edit) }
        }
        if !shown.isEmpty {
          Button(t("common.copy"), systemImage: "doc.on.doc") {
            UIPasteboardBridge.copy(shown)
            actions.toast(t("code.copied"), .copy)
          }
        }
        if goal {
          Button(t("goal.setFromPrompt"), systemImage: "flag") {
            actions.goal(.set, turn.text.trimmingCharacters(in: .whitespacesAndNewlines))
          }
        }
      })
      .padding(.leading, Metrics.bubbleInset)
      .padding(.horizontal, Metrics.pad)
      .frame(maxWidth: .infinity, alignment: .trailing)
    }
  }
}

/// The prompt's long-press menu, left off when it would hold nothing; the lifted preview keeps the bubble's corners
private struct PromptMenu<Items: View>: ViewModifier {
  var show: Bool
  var shape: RoundedRectangle
  @ViewBuilder var items: Items

  func body(content: Content) -> some View {
    if show {
      #if canImport(UIKit)
      content.contentShape(.contextMenuPreview, shape).contextMenu { items }
      #else
      content.contextMenu { items }
      #endif
    } else {
      content
    }
  }
}

/// A prompt steered into a running turn: the same card, inside the turn's process at the point it joined
struct SteeredMessage: View {
  let block: SteerBlock
  var body: some View {
    UserMessageCard(turn: UserTurn(text: block.text, attachments: block.attachments))
      .padding(.horizontal, -Metrics.pad)
  }
}

/// Attachment pills and image tiles, wrapping
struct TurnAttachments: View {
  let attachments: [Attachment]

  var body: some View {
    FlowLayout(spacing: Metrics.gap) {
      ForEach(Array(attachments.enumerated()), id: \.offset) { _, item in
        switch item {
        case .image(let blob, _, let name):
          // Inline pixels when the app staged the photo itself, else the engine's blob
          AttachmentThumbnail(blob: blob, name: name)
        case .file(_, let name), .text(_, let name, _):
          AttachmentPill(name: name, icon: .fileText)
        case .selection(_, let name, let start, let end, _):
          AttachmentPill(name: "\(name) \(start == end ? "L\(start)" : "L\(start)-\(end)")", icon: .textSelect)
        case .quote, .unknown:
          EmptyView()
        }
      }
    }
  }
}

struct AttachmentPill: View {
  let name: String
  let icon: LucideIcon
  /// A staged pill's ✕, after the name
  var onRemove: (() -> Void)? = nil
  var body: some View {
    HStack(spacing: Metrics.s1) {
      Icon(icon, size: WebMetrics.icon).foregroundStyle(Palette.fg3)
      Text(web: name).lineLimit(1).truncationMode(.middle)
      if let onRemove {
        Button(action: onRemove) {
          Icon(.x, size: Metrics.chevron, strokeWidth: 2)
            .foregroundStyle(Palette.fg3)
            .frame(width: WebMetrics.ctlSm, height: WebMetrics.ctlSm)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        // The ✕'s hit square reaches into the pill's padding, so its glyph keeps the same inset as the icon
        .padding(.trailing, (Metrics.chevron - WebMetrics.ctlSm) / 2)
        .accessibilityLabel(Text(web: t("common.remove")))
      }
    }
    .textStyle(WebType.text3, weight: .medium)
    .foregroundStyle(Palette.fg2)
    .padding(.horizontal, Metrics.s2)
    .frame(height: WebMetrics.ctlSm)
    .background(RoundedRectangle(cornerRadius: Metrics.rSm).fill(Palette.chip))
  }
}

/// flex-wrap: items left to right, wrapping at the proposed width, rows aligned to their tops
struct FlowLayout: Layout {
  var spacing: CGFloat

  func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
    let rows = arrange(width: proposal.width ?? .infinity, subviews: subviews)
    let width = rows.map { $0.width }.max() ?? 0
    let height = rows.map(\.height).reduce(0, +) + spacing * CGFloat(max(0, rows.count - 1))
    return CGSize(width: proposal.width ?? width, height: height)
  }

  func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
    var y = bounds.minY
    for row in arrange(width: bounds.width, subviews: subviews) {
      var x = bounds.minX
      for index in row.items {
        let size = subviews[index].sizeThatFits(.unspecified)
        subviews[index].place(at: CGPoint(x: x, y: y), proposal: ProposedViewSize(size))
        x += size.width + spacing
      }
      y += row.height + spacing
    }
  }

  private struct FlowRow { var items: [Int] = []; var width: CGFloat = 0; var height: CGFloat = 0 }

  private func arrange(width: CGFloat, subviews: Subviews) -> [FlowRow] {
    var rows: [FlowRow] = []
    var row = FlowRow()
    for (i, view) in subviews.enumerated() {
      let size = view.sizeThatFits(.unspecified)
      let next = row.items.isEmpty ? size.width : row.width + spacing + size.width
      if next > width && !row.items.isEmpty {
        rows.append(row); row = FlowRow()
      }
      row.width = row.items.isEmpty ? size.width : row.width + spacing + size.width
      row.height = max(row.height, size.height)
      row.items.append(i)
    }
    if !row.items.isEmpty { rows.append(row) }
    return rows
  }
}

// MARK: - Permission

/// One option as the card shows it: known CLI labels shortened, unfamiliar ones verbatim
struct DisplayOption: Identifiable {
  var id: String
  var label: String
  var kind: PermissionKind
  var detail: String?
  var bypass = false
}

/// Complete adapter plan headings (permissionOptions.ts PLAN_TITLES); custom mode names stay the agent's text
private let planTitles: [String: String] = [
  "Approve Plan": "permission.plan.approve", "Ready to code?": "permission.plan.ready",
  "Implement this plan?": "permission.plan.implementTitle", "Enter plan mode": "permission.plan.enterTitle",
  "EnterPlanMode": "permission.plan.enterTitle", "Exit plan mode": "permission.plan.exitTitle",
  "ExitPlanMode": "permission.plan.exitTitle", "exit_plan_mode": "permission.plan.exitTitle",
]

/// planApprovalTitle: a known plan heading in the current language, anything else verbatim
func planApprovalTitle(_ title: String) -> String { planTitles[title].map { t($0) } ?? title }

/// Known plan approval options (PLAN_OPTIONS): Claude's and Codex's exact labels, matched with their kind
private let planOptions: [(kind: PermissionKind, labels: [String], key: String, bypass: Bool)] = [
  (.allow_once, ["Yes, manually approve edits", "Yes, and manually approve edits"], "permission.plan.manual", false),
  (.reject_once, ["No, keep planning"], "permission.plan.continue", false),
  (.allow_always, ["Yes, and use auto mode", "Yes, and use \"auto\" mode"], "permission.plan.auto", false),
  (.allow_always, ["Yes, auto-accept edits", "Yes, and auto-accept edits"], "permission.plan.acceptEdits", false),
  (.allow_always, ["Yes, and bypass permissions"], "permission.plan.bypass", true),
  (.allow_once, ["Yes, implement this plan"], "permission.plan.implement", false),
  // Codex also uses this rejection on ordinary approvals; it keeps its exact meaning
  (.reject_once, ["No, and tell Codex what to do differently"], "permission.feedbackCodex", false),
  (.allow_once, ["Yes, enter plan mode"], "permission.plan.enter", false),
  (.reject_once, ["No, start implementing now"], "permission.plan.start", false),
]

/// permissionOption: ACP kinds say nothing of scope, so only complete, known labels are rewritten. `plan`: the option
/// belongs to a linked plan approval, where Build / Revise and Kimi's reject-and-exit are localized too
func permissionOption(_ option: PermissionOption, plan: Bool = false) -> DisplayOption {
  var out = DisplayOption(id: option.id, label: option.label, kind: option.kind, detail: option.detail)
  let label = option.label
  func matches(_ pattern: String) -> [String]? {
    guard let regex = try? NSRegularExpression(pattern: pattern, options: [.caseInsensitive]),
          let m = regex.firstMatch(in: label, range: NSRange(label.startIndex..., in: label)) else { return nil }
    return (0..<m.numberOfRanges).map { i in
      Range(m.range(at: i), in: label).map { String(label[$0]) } ?? ""
    }
  }
  if let known = planOptions.first(where: { $0.kind == option.kind && $0.labels.contains(label) }) {
    out.label = t(known.key); out.bypass = known.bypass; return out
  }
  if plan {
    switch (option.kind, label) {
    case (.allow_once, "Build"): out.label = t("plan.build"); return out
    case (.reject_once, "Revise"): out.label = t("plan.revise"); return out
    case (.allow_always, "Build with Bypass Permissions"): out.label = t("permission.plan.buildBypass"); out.bypass = true; return out
    case (.reject_once, "Reject and exit Plan mode"): out.label = t("permission.plan.rejectExit"); return out
    default: break
    }
  }
  if option.kind == .allow_once, matches("^(Allow|Allow once|Yes|Yes, allow once|Yes, proceed|Yes, just this once)$") != nil {
    out.label = t("permission.once"); return out
  }
  if option.kind == .reject_once, matches("^(Reject|Reject once|Deny|No|No, reject|Cancel|No, continue without running it)$") != nil {
    out.label = t("permission.reject"); return out
  }
  guard option.kind == .allow_always else { return out }
  if matches("^(Yes, during this session|Yes, and (?:don't|do not) ask again for this command in this session|Allow for this session)$") != nil {
    out.label = t("permission.session"); return out
  }
  // Case-sensitive like the TypeScript, whose pattern carries no `i` flag
  if let m = label.wholeMatch(of: /Yes, clear context(?: \((\d+(?:\.\d+)?)% used\))? and (use auto mode|bypass permissions|auto-accept edits)/) {
    let key = m.2 == "use auto mode" ? "permission.plan.clearAuto" : m.2 == "bypass permissions" ? "permission.plan.clearBypass"
      : "permission.plan.clearAcceptEdits"
    out.label = t(key, ["usage": m.1.map { t("permission.plan.usage", ["percent": String($0)]) } ?? ""])
    out.bypass = m.2 == "bypass permissions"
    return out
  }
  if label == "Yes, switch to bypass mode" { out.label = t("permission.bypass"); out.bypass = true; return out }
  let session = matches("^Yes, allow `([^`]+)` commands \\(this session\\)$") ?? matches("^Always allow `([^`]+)` in this session$")
    ?? matches("^Yes, and (?:don't|do not) ask again for commands that start with `([^`]+)`$")
  let dontAsk = matches("^Yes, and (?:don't|do not) ask again for (.+) commands$").map(\.[1]).flatMap { $0 != "similar" ? $0 : nil }
  let project = matches("^Yes, always allow `([^`]+)` commands in `([^`]+)`$")
  let all = matches("^Yes, always allow `([^`]+)` commands in all projects$")
  guard let command = session?[1] ?? dontAsk ?? project?[1] ?? all?[1] else { return out }
  out.label = session != nil || dontAsk != nil ? t("permission.session")
    : project.map { t("permission.project", ["project": $0[2]]) } ?? t("permission.allProjects")
  out.detail = t("permission.commands", ["command": command])
  return out
}

/// An approval request: title row, the command, then the extra-grants menu on the left and reject / allow on the right
struct PermissionCard: View {
  let block: PermissionBlock
  var onChoose: (String) -> Void = { _ in }

  var body: some View {
    let options = block.options.map { permissionOption($0, plan: block.planId != nil) }
    // Several allow_once and no allow_always: a list of answers, every option an equal button
    let flat = !options.contains { $0.kind == .allow_always } && options.filter { $0.kind == .allow_once }.count > 1
    let allow = flat ? nil : options.first { $0.kind == .allow_once }
    let reject = flat ? nil : options.first { $0.kind == .reject_once }
    let more = flat ? [] : options.filter { $0.id != allow?.id && $0.id != reject?.id }
    let title = block.command != nil ? t("permission.run") : planApprovalTitle(block.title)

    VStack(alignment: .leading, spacing: Metrics.gap) {
      Row(dense: true) {
        if block.command != nil { Icon(.terminal, size: WebMetrics.icon) }
      } content: {
        Text(web: title).textStyle(WebType.text2, weight: .medium).foregroundStyle(Palette.fg2)
      }
      if let command = block.command {
        Text(web: command, size: WebType.mono.size)
          .textStyle(WebType.mono)
          .foregroundStyle(Palette.fg1)
          .fixedSize(horizontal: false, vertical: true)
      }
      if let description = block.description {
        Text(web: description).textStyle(WebType.text3).foregroundStyle(Palette.fg2)
          .fixedSize(horizontal: false, vertical: true)
      }
      if flat {
        FlowLayout(spacing: Metrics.gap) {
          ForEach(options) { o in
            Button(web: o.label) { onChoose(o.id) }.buttonStyle(AcpButtonStyle())
          }
        }
      } else {
        // justify-between with a --gap: the menu left, the answers pushed right, one gap between them at the least
        HStack(spacing: 0) {
          if !more.isEmpty {
            PopupMenu {
              ForEach(more.filter { !$0.bypass } + more.filter(\.bypass)) { o in
                Button { onChoose(o.id) } label: {
                  Text(web: o.label)
                  if let detail = o.detail { Text(web: detail) }
                }
              }
            } label: {
              Text(web: t("permission.more"))
              Icon(.chevronDown, size: WebMetrics.icon)
            }
            .buttonStyle(AcpButtonStyle(fixed: true))
            .menuStyle(.button)
            .fixedSize()
          }
          Spacer(minLength: more.isEmpty ? 0 : Metrics.gap)
          HStack(spacing: Metrics.gap) {
            if let reject {
              Button(web: reject.label) { onChoose(reject.id) }
                .buttonStyle(AcpButtonStyle(block.defaultToNo == true ? .primary : .secondary))
            }
            if let allow {
              Button(web: allow.label) { onChoose(allow.id) }
                .buttonStyle(AcpButtonStyle(block.defaultToNo == true ? .secondary : .primary))
            }
          }
        }
      }
    }
    // p-pad inside a 1 px border
    .padding(Metrics.pad + Metrics.hairline)
    .frame(maxWidth: .infinity, alignment: .leading)
    .background(RoundedRectangle(cornerRadius: Metrics.rLg).fill(Palette.bg1))
    .overlay(RoundedRectangle(cornerRadius: Metrics.rLg).strokeBorder(Palette.conversationLine))
  }
}

// MARK: - Turn actions

/// Retry · copy the reply · fork from this turn · response statistics; hugs the reply (-mt-gap) at --ctl-sm. Retry
/// shows only on the newest reply when it failed in a way Retry answers (`retryableTurn`, the alert card's rule), so it
/// stays at hand after the card is dismissed. A reply with nothing in it offers no copy or statistics
struct TurnActions: View {
  let turn: AgentTurn
  var index = 0
  /// The prompt's selections, which name the model when the peer did not
  var settings: TurnSettings?
  var retry = false
  /// The session's newest turn (`-popup stats` opens its card)
  var last = false
  @Environment(\.sessionActions) private var sessionActions
  @Environment(\.planContext) private var context
  @Environment(\.initialPopup) private var initialPopup
  @State private var copied = false
  @State private var stats = false

  var body: some View {
    let content = hasTurnContent(turn)
    HStack(spacing: Metrics.s0_5) {
      if retry {
        IconButton(.rotateCcw, size: .web, label: t("common.retry")) { sessionActions.retry() }
          .accessibilityHint(Text(web: t("alert.retryHint")))
      }
      let reply = replyMarkdown(turn)
      if !reply.isEmpty {
        IconButton(copied ? .check : .copy, size: .web, label: copied ? t("code.copied") : t("turn.copy")) {
          UIPasteboardBridge.copy(reply)
          copied = true
          sessionActions.toast(t("code.copied"), .copy)
        }
      }
      IconButton(.gitBranch, size: .web, label: t("turn.fork")) { sessionActions.fork(index) }
      if content {
        IconButton(.chartNoAxesColumn, size: .web, label: t("turn.stats"), open: stats) { stats.toggle() }
          .popover(isPresented: $stats) {
            TurnStats(turn: turn, model: modelLabel(turn.usage, settings: settings, controls: context.controls))
              .presentationCompactAdaptation(.popover)
          }
      }
    }
    .frame(maxWidth: .infinity, minHeight: WebMetrics.ctlSm, maxHeight: WebMetrics.ctlSm, alignment: .trailing)
    .padding(.top, -Metrics.gap)
    .task(id: copied) {
      // The check stands for a moment, then the copy glyph returns
      guard copied else { return }
      try? await Task.sleep(for: .milliseconds(1500))
      copied = false
    }
    .task {
      // `-popup stats`: the newest reply's card, once the thread has settled on screen (screenshots)
      guard initialPopup == "stats", last, content else { return }
      try? await Task.sleep(for: .seconds(1))
      stats = true
    }
  }
}

/// The reply text a copy takes: the turn's final prose, else (a tail the split finds nothing in) its plain text blocks
func replyMarkdown(_ turn: AgentTurn) -> String {
  let reply = splitCodexBlocks(turn.blocks).reply.map(\.markdown).joined(separator: "\n\n")
    .trimmingCharacters(in: .whitespacesAndNewlines)
  if !reply.isEmpty { return reply }
  return turn.blocks.compactMap { if case .text(let b) = $0 { b.markdown } else { nil } }.joined(separator: "\n\n")
    .trimmingCharacters(in: .whitespacesAndNewlines)
}

/// StatsCard: the title, the reported rows in their groups under thin dividers, and the vendor's request id with its
/// copy action at the foot
struct TurnStats: View {
  let turn: AgentTurn
  var model: String?
  @State private var copied = false

  var body: some View {
    let groups = turnStatGroups(turn, model: model)
    VStack(alignment: .leading, spacing: Metrics.s1) {
      Text(web: t("turn.stats"))
        .textStyle(WebType.text2, weight: .medium)
        .foregroundStyle(Palette.fg1)
        .padding(.horizontal, Metrics.s2)
        .frame(minHeight: WebMetrics.ctl)
      VStack(alignment: .leading, spacing: 0) {
        ForEach(Array(groups.enumerated()), id: \.offset) { i, rows in
          if i > 0 { divider.padding(.vertical, Metrics.s1) }
          ForEach(rows, id: \.label) { stat($0.label, $0.value) }
        }
      }
      if let requestId = turn.usage?.requestId { footer(requestId) }
    }
    .monospacedDigit()
    .padding(Metrics.s1)
    // A set width: the popover sizes itself from the card's ideal size, which must already hold the wrapped labels
    .frame(width: Metrics.popLg)
    .task(id: copied) {
      guard copied else { return }
      try? await Task.sleep(for: .milliseconds(1500))
      copied = false
    }
  }

  private var divider: some View {
    Rectangle().fill(Palette.line).frame(height: Metrics.hairline).padding(.horizontal, Metrics.s2)
  }

  func stat(_ label: String, _ value: String) -> some View {
    // The figure keeps its width; a label longer than the phone's popover leaves wraps under itself
    HStack(alignment: .firstTextBaseline, spacing: Metrics.s3) {
      Text(web: label).foregroundStyle(Palette.fg1).fixedSize(horizontal: false, vertical: true)
      Spacer(minLength: 0)
      Text(web: value).foregroundStyle(Palette.fg2).lineLimit(1).truncationMode(.middle).layoutPriority(1)
    }
    .textStyle(WebType.text3)
    .padding(.horizontal, Metrics.s2)
    .padding(.vertical, Metrics.s0_5)
  }

  /// PanelFooter: the id in faint text and the square copy button, under one divider
  private func footer(_ requestId: String) -> some View {
    let label = copied ? t("stats.requestIdCopied") : t("stats.copyRequestId")
    return VStack(spacing: Metrics.s1) {
      divider.padding(.horizontal, -Metrics.s2)
      HStack(spacing: Metrics.s1) {
        Text(web: requestId)
          .textStyle(WebType.text3)
          .foregroundStyle(Palette.fg3)
          .lineLimit(1)
          .truncationMode(.middle)
          .frame(maxWidth: .infinity, alignment: .leading)
        IconButton(copied ? .check : .copy, size: .web, label: label) {
          UIPasteboardBridge.copy(requestId)
          copied = true
        }
      }
      .padding(.leading, Metrics.s2)
    }
  }
}
