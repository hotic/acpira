import AcpiraDesign
import AcpiraModels
import SwiftUI

// The user's prompt card (Turns.tsx UserMessage + Attachments.tsx TurnAttachments), the permission card
// (Permission.tsx + permissionOptions.ts) and the row under a finished reply (TurnActions.tsx).

// MARK: - User message

/// A sent prompt: a right-aligned bubble on the chip fill, no outline; attachments above it, also right-aligned. The
/// bubble's right edge meets the prose's (the agent turn's --pad)
struct UserMessageCard: View {
  let turn: UserTurn

  var body: some View {
    if turn.auto != true {
      let shown = turn.text.trimmingTrailingWhitespace()
      let attachments = turn.attachments ?? []
      let shape = RoundedRectangle(cornerRadius: Metrics.rBubble)
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
      .padding(.leading, Metrics.bubbleInset)
      .padding(.horizontal, Metrics.pad)
      .frame(maxWidth: .infinity, alignment: .trailing)
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
  var body: some View {
    HStack(spacing: Metrics.s1) {
      Icon(icon, size: WebMetrics.icon).foregroundStyle(Palette.fg3)
      Text(web: name).lineLimit(1).truncationMode(.middle)
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

/// permissionOption: ACP kinds say nothing of scope, so only complete, known labels are rewritten
func permissionOption(_ option: PermissionOption) -> DisplayOption {
  var out = DisplayOption(id: option.id, label: option.label, kind: option.kind, detail: option.detail)
  let label = option.label
  func matches(_ pattern: String) -> [String]? {
    guard let regex = try? NSRegularExpression(pattern: pattern, options: [.caseInsensitive]),
          let m = regex.firstMatch(in: label, range: NSRange(label.startIndex..., in: label)) else { return nil }
    return (0..<m.numberOfRanges).map { i in
      Range(m.range(at: i), in: label).map { String(label[$0]) } ?? ""
    }
  }
  if option.kind == .allow_once, matches("^(Allow|Allow once|Yes, allow once)$") != nil { out.label = t("permission.once"); return out }
  if option.kind == .reject_once, matches("^(Reject|Reject once|Deny|No, reject)$") != nil { out.label = t("permission.reject"); return out }
  guard option.kind == .allow_always else { return out }
  if label == "Yes, switch to bypass mode" { out.label = t("permission.bypass"); out.bypass = true; return out }
  if let m = matches("^Yes, allow `([^`]+)` commands \\(this session\\)$") {
    out.label = t("permission.session"); out.detail = t("permission.commands", ["command": m[1]])
  } else if let m = matches("^Yes, always allow `([^`]+)` commands in `([^`]+)`$") {
    out.label = t("permission.project", ["project": m[2]]); out.detail = t("permission.commands", ["command": m[1]])
  } else if let m = matches("^Yes, always allow `([^`]+)` commands in all projects$") {
    out.label = t("permission.allProjects"); out.detail = t("permission.commands", ["command": m[1]])
  }
  return out
}

/// An approval request: title row, the command, then the extra-grants menu on the left and reject / allow on the right
struct PermissionCard: View {
  let block: PermissionBlock
  var onChoose: (String) -> Void = { _ in }

  var body: some View {
    let options = block.options.map(permissionOption)
    // Several allow_once and no allow_always: a list of answers, every option an equal button
    let flat = !options.contains { $0.kind == .allow_always } && options.filter { $0.kind == .allow_once }.count > 1
    let allow = flat ? nil : options.first { $0.kind == .allow_once }
    let reject = flat ? nil : options.first { $0.kind == .reject_once }
    let more = flat ? [] : options.filter { $0.id != allow?.id && $0.id != reject?.id }
    let title = block.command != nil ? t("permission.run") : block.title

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

/// Copy the reply · fork from this turn · response statistics; hugs the reply (-mt-gap) at --ctl-sm
struct TurnActions: View {
  let turn: AgentTurn
  var index = 0
  @Environment(\.sessionActions) private var sessionActions
  @State private var copied = false
  @State private var stats = false

  var body: some View {
    HStack(spacing: Metrics.s0_5) {
      let reply = replyMarkdown(turn)
      if !reply.isEmpty {
        IconButton(copied ? .check : .copy, size: .web, label: copied ? t("code.copied") : t("turn.copy")) {
          UIPasteboardBridge.copy(reply)
          copied = true
          sessionActions.toast(t("code.copied"), .copy)
        }
      }
      IconButton(.gitBranch, size: .web, label: t("turn.fork")) { sessionActions.fork(index) }
      IconButton(.chartNoAxesColumn, size: .web, label: t("turn.stats"), open: stats) { stats.toggle() }
        .popover(isPresented: $stats) {
          TurnStats(turn: turn).presentationCompactAdaptation(.popover)
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
  }
}

/// The reply text a copy takes: the turn's final prose
func replyMarkdown(_ turn: AgentTurn) -> String {
  splitCodexBlocks(turn.blocks).reply.map { $0.markdown.trimmingTrailingWhitespace() }.joined(separator: "\n\n")
}

/// Per-turn statistics: dense label / value rows
struct TurnStats: View {
  let turn: AgentTurn
  var body: some View {
    let calls = turn.blocks.filter { $0.toolCall != nil }.count
    VStack(alignment: .leading, spacing: 0) {
      if turn.startedAt != nil && turn.endedAt != nil { stat(t("stats.duration"), elapsedDuration(turn)) }
      stat(t("stats.toolCalls"), "\(calls)")
      if let input = turn.usage?.input { stat(t("stats.input"), input.formatted()) }
      if let output = turn.usage?.output { stat(t("stats.output"), output.formatted()) }
    }
    .padding(Metrics.s1)
    .frame(minWidth: Metrics.popLg)
  }

  func stat(_ label: String, _ value: String) -> some View {
    HStack(spacing: Metrics.s3) {
      Text(web: label).foregroundStyle(Palette.fg1)
      Spacer(minLength: 0)
      Text(web: value).foregroundStyle(Palette.fg2).lineLimit(1)
    }
    .textStyle(WebType.text3)
    .padding(.horizontal, Metrics.s2)
    .padding(.vertical, Metrics.s0_5)
  }
}
