import AcpiraDesign
import AcpiraModels
import SwiftUI

// chat/ToolCall.tsx, ToolGroup.tsx, Terminal.tsx and the DiffBlock of CodeBlock.tsx under toolLine = icon: every
// process row leads with its kind's icon, and an open row draws its rail (ui/ConnectedRail) from that icon.

/// chat/icons.ts toolIcon: to-do, image generation and shown images by their verb, the rest by kind
func toolIcon(_ block: ToolCallBlock) -> LucideIcon {
  switch block.verbKey {
  case "verb.todo": .listTodo
  case "verb.imagegen": .imagePlus
  case "verb.showImage": .image
  default: kindIcon(block.kind)
  }
}

/// TOOL_ICON
func kindIcon(_ kind: ToolKind) -> LucideIcon {
  switch kind {
  case .read: .fileText
  case .edit: .pencil
  case .delete: .trash2
  case .move: .moveRight
  case .search: .search
  case .execute: .terminal
  case .think: .brain
  case .fetch: .globe
  case .switch_mode: .`repeat`
  default: .wrench
  }
}

// MARK: - One tool call

struct ToolCallView: View {
  let block: ToolCallBlock
  var grouped = false
  @Environment(\.preOpen) private var preOpen
  @State private var manual: Bool?

  var body: some View {
    EntranceOnce(id: "\(block.id):tool") { rows }
  }

  @ViewBuilder private var rows: some View {
    let running = block.status == .in_progress
    let execute = block.kind == .execute
    let target = block.target?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
    // A shell command handed over verbatim: one line on the row, the whole command in the card
    let command: String? = execute && block.targetMono == true && block.verbKey == nil && !target.isEmpty ? block.target : nil
    let files = toolFiles(block)
    let visible = visibleToolContents(block)
    let span = block.kind == .edit ? editSpan([block]) : nil

    let label = ToolLabel(verb: toolVerb(block), shimmer: running, command: command,
                          target: block.kind == .read && !files.isEmpty ? nil : block.target,
                          mono: block.targetMono == true, span: span)
    let trailing = ToolTrailing(block: block, execute: execute)
    let icon = toolIcon(block)

    if let todos = block.todoEntries {
      PlanDetails(entries: todos) { label } trailing: { trailing }
    } else if isImageResult(block) {
      // Generated and shown images: under their row in an open process fold, after the fold once it closes (CodexFold)
      VStack(alignment: .leading, spacing: Metrics.gap) {
        Row(tone: .action) { LeadIcon(icon) } content: { label } trailing: { trailing }
        ImageResults(block: block, grouped: grouped)
      }
    } else if !files.isEmpty && block.kind == .search {
      Disclosure(open: foldBinding($manual, preOpen.contains(.results)), icon: icon, indent: false, rail: .rows) { label } trailing: { trailing } content: {
        ResultList(items: files, kind: block.kind).environment(\.rowEnterDelay, Motion.rowEnterFile)
      }
    } else if files.count == 1 && block.kind == .read {
      let hit = splitHit(files[0])
      Row(tone: .action) { LeadIcon(icon) } content: {
        label
        HStack(alignment: .firstTextBaseline, spacing: Metrics.s1) {
          RowTarget(hit.main, mono: true, ink: Palette.fg2)
          if let aside = hit.aside { Aside(aside) }
        }
      } trailing: { trailing }
    } else if !files.isEmpty {
      VStack(alignment: .leading, spacing: 0) {
        Row(tone: .action) { LeadIcon(icon) } content: { label } trailing: { trailing }
        EntranceOnce(id: "\(block.id):files") { ResultList(items: files, kind: block.kind) }
      }
      .connectedRail(enabled: true, endAtLastRow: true)
    } else if block.kind == .read || block.kind == .search || (block.kind == .edit && visible.isEmpty && block.asyncTask == nil)
                || (grouped && block.content == nil && command == nil) {
      Row(tone: .action) { LeadIcon(icon) } content: { label } trailing: { trailing }
    } else if execute || (!visible.isEmpty && visible.allSatisfy({ if case .text = $0 { true } else { false } })) {
      // The command card sits in the label column; an open card keeps --command-after below it
      CommandDisclosure(block: block, command: command, items: visible, icon: icon, label: label, trailing: trailing,
                        defaultOpen: execute && !grouped && running)
    } else {
      Disclosure(open: foldBinding($manual, preOpen.contains(.results)), icon: icon, indent: false,
                 rail: { if case .list = block.content { .rows } else { .off } }()) { label } trailing: { trailing } content: {
        ToolBody(block: block, items: visible)
      }
    }
  }
}

/// Verb, then the command or target (with an edit's line span)
struct ToolLabel: View {
  let verb: String
  var shimmer: Bool
  var command: String?
  var target: String?
  var mono: Bool
  var span: String?

  var body: some View {
    RowLabel(verb, shimmer: shimmer)
    if let command {
      RowTarget(command, mono: true)
    } else if let target, !target.isEmpty {
      if let span {
        HStack(alignment: .firstTextBaseline, spacing: Metrics.s1) {
          RowTarget(target, mono: mono)
          Aside(span)
        }
      } else {
        RowTarget(target, mono: mono)
      }
    }
  }
}

/// Commands end with ✓ / ✗ and their run time; other rows show their diff stat
struct ToolTrailing: View {
  let block: ToolCallBlock
  let execute: Bool

  var body: some View {
    if execute {
      let duration = block.startedAt.flatMap { start in block.endedAt.map { commandDuration(ms: $0 - start) } }
      if block.status == .completed {
        Icon(.check, size: WebMetrics.chevron, strokeWidth: 2).foregroundStyle(Palette.ok)
      } else if block.status == .failed {
        Icon(.x, size: WebMetrics.chevron, strokeWidth: 2).foregroundStyle(Palette.danger)
      }
      if let duration {
        Text(web: duration)
      } else if block.kind == .execute, block.status == .in_progress, let start = block.startedAt {
        // A live command ticks in whole seconds; a finished one reads its host timestamps
        CommandSeconds(since: start)
      } else if block.status != .completed && block.status != .failed {
        // A command row always has its trailing slot in the webview (ToolCall.tsx), so an empty one still takes the
        // row's gap and a long command is cut before it; Row drops a slot that renders nothing
        Color.clear.frame(width: 0, height: 0)
      }
    } else if let stat = block.diffStat {
      DiffStatView(add: stat.add, del: stat.del)
    }
  }
}

/// An execute (or text-only) call: the row opens the command card
struct CommandDisclosure<Label: View, Trailing: View>: View {
  let block: ToolCallBlock
  let command: String?
  let items: [ToolContent]
  let icon: LucideIcon
  let label: Label
  let trailing: Trailing
  var defaultOpen: Bool
  @State private var manual: Bool?
  @Environment(\.preOpen) private var preOpen

  init(block: ToolCallBlock, command: String?, items: [ToolContent], icon: LucideIcon, label: Label, trailing: Trailing,
       defaultOpen: Bool) {
    self.block = block; self.command = command; self.items = items; self.icon = icon; self.label = label; self.trailing = trailing
    self.defaultOpen = defaultOpen
  }

  var body: some View {
    let open = manual ?? (defaultOpen || preOpen.contains(.command))
    // The card sits in the label column on the row's rail; --command-after stays outside it so the end dot meets the card.
    // The phone keeps it there too: a column-wide card without the rail lost the desktop's reading of the row (2026-10-27)
    Disclosure(open: foldBinding($manual, defaultOpen || preOpen.contains(.command)), icon: icon,
               bodyTop: Metrics.gapHalf) { label } trailing: { trailing } content: {
      if block.kind == .execute {
        // An image the command produced (screenshot tools) sits below its output (ToolCall.tsx ToolBody)
        VStack(alignment: .leading, spacing: Metrics.gap) {
          TerminalOutput(block: block, command: command)
          ForEach(Array(toolImages(block).enumerated()), id: \.offset) { _, image in AgentImageView(blob: image.blob, uri: image.uri) }
        }
      } else {
        ToolOutput(text: items.compactMap { if case .text(let t) = $0 { t } else { nil } }.joined(separator: "\n"))
      }
    }
    .padding(.bottom, open ? Metrics.commandAfter : 0)
  }
}

/// Diff / list / image bodies, stacked in wire order
struct ToolBody: View {
  let block: ToolCallBlock
  let items: [ToolContent]

  var body: some View {
    VStack(alignment: .leading, spacing: Metrics.gap) {
      ForEach(Array(items.enumerated()), id: \.offset) { _, item in
        switch item {
        case .diff(let lines, _): DiffBlockView(lines: lines, title: block.target ?? "")
        case .list(let list): ResultList(items: list, kind: block.kind)
        case .text(let text): ToolOutput(text: text)
        case .image(let image): if !isWidget(image) { AgentImageView(blob: image.blob, uri: image.uri) }
        case .unknown: EmptyView()
        }
      }
    }
  }
}

// MARK: - Result list

/// File / hit rows under a read or search: dense action rows, the name in mono fg-2 and the line as a faint note
struct ResultList: View {
  let items: [String]
  let kind: ToolKind

  var body: some View {
    let icon: LucideIcon = kind == .fetch ? .globe : .fileText
    VStack(alignment: .leading, spacing: 0) {
      ForEach(items, id: \.self) { item in
        let hit = splitHit(item)
        if kind == .read || kind == .search {
          Row(tone: .action, dense: true) { LeadIcon(icon) } content: {
            HStack(alignment: .firstTextBaseline, spacing: Metrics.s1) {
              RowTarget(hit.main, mono: true, ink: Palette.fg2)
              if let aside = hit.aside { Aside(aside) }
            }
          }
        } else {
          Row(tone: .action, dense: true) { LeadIcon(icon) } content: {
            RowTarget(hit.main, mono: kind != .fetch, ink: Palette.fg2)
          } trailing: {
            if let aside = hit.aside { Text(web: aside) }
          }
        }
      }
    }
  }
}

// MARK: - Groups

/// A run of consecutive reads or edits under one head: the verb, the files it touched, an edit run's total change
struct ToolGroupView: View {
  let kind: GroupKind
  let blocks: [ToolCallBlock]
  @Environment(\.preOpen) private var preOpen
  @State private var manual: Bool?

  var body: some View {
    let current = blocks.last(where: { $0.status == .pending || $0.status == .in_progress }) ?? blocks.last!
    let names = groupNames(blocks)
    let stat = kind == .edit ? diffStatOf(blocks) : nil
    Disclosure(open: foldBinding($manual, preOpen.contains(.results)), icon: kind == .read ? .fileText : .pencil,
               indent: false, rail: .rows, bodyTop: 0) {
      RowLabel(toolVerb(current), shimmer: current.status == .pending || current.status == .in_progress)
      if !names.isEmpty { RowTarget(namesLabel(names), mono: true) }
    } trailing: {
      if let stat { DiffStatView(add: stat.add, del: stat.del) }
    } content: {
      VStack(alignment: .leading, spacing: 0) {
        if kind == .read {
          ForEach(blocks, id: \.id) { block in EntranceOnce(id: itemEntrance(block.id)) { ReadEntry(block: block) } }
        } else {
          ForEach(editEntries(blocks), id: \.key) { entry in
            EntranceOnce(id: itemEntrance(entry.blocks[0].id)) { EditEntry(blocks: entry.blocks) }
          }
        }
      }
      // `tool-results`: file rows enter after the head
      .environment(\.rowEnterDelay, Motion.rowEnterFile)
    }
  }
}

struct ReadEntry: View {
  let block: ToolCallBlock
  var body: some View {
    let files = toolFiles(block)
    if !files.isEmpty {
      ResultList(items: files, kind: .read)
    } else {
      Row(tone: .action, dense: true) { LeadIcon(.fileText) } content: {
        Shimmer(block.target ?? toolVerb(block), active: block.status == .pending || block.status == .in_progress)
          .inlineFont(WebType.mono)
          .lineLimit(1)
      }
    }
  }
}

/// One file of an edit group: its changed lines and total change; the diffs open on a tap
struct EditEntry: View {
  let blocks: [ToolCallBlock]
  @State private var manual: Bool?
  @Environment(\.preOpen) private var preOpen

  var body: some View {
    let stat = diffStatOf(blocks)
    let span = editSpan(blocks)
    let diffs = blocks.flatMap { visibleToolContents($0) }.filter { if case .diff = $0 { true } else if case .text = $0 { true } else { false } }
    let label = HStack(alignment: .firstTextBaseline, spacing: Metrics.s1) {
      RowTarget(fileName(blocks[0]), mono: true, ink: Palette.fg2)
      if let span { Aside(span) }
    }
    let trailing = Group { if let stat { DiffStatView(add: stat.add, del: stat.del) } }
    if diffs.isEmpty {
      Row(tone: .action, dense: true) { LeadIcon(.pencil) } content: { label } trailing: { trailing }
    } else {
      // EntryFold: full width, no rail root of its own, so the group's rail runs on through its icon
      Disclosure(open: foldBinding($manual, preOpen.contains(.edit)), dense: true, icon: .pencil, indent: false, root: false) {
        label
      } trailing: { trailing } content: {
        VStack(alignment: .leading, spacing: Metrics.gap) {
          ForEach(Array(diffs.enumerated()), id: \.offset) { _, item in
            switch item {
            case .diff(let lines, _): DiffBlockView(lines: lines, title: fileName(blocks[0]))
            case .text(let text): ToolOutput(text: text)
            default: EmptyView()
            }
          }
        }
      }
    }
  }
}

// MARK: - Command card

/// The `code-output` frame: rounded-md, a hairline, the code background. The CSS border takes layout room and the
/// stroke overlay does not, so the content sits a hairline in on every side
struct CodeOutputFrame: ViewModifier {
  func body(content: Content) -> some View {
    content
      .padding(Metrics.hairline)
      .frame(maxWidth: .infinity, alignment: .leading)
      .background(Palette.bgCode, in: RoundedRectangle(cornerRadius: Metrics.rMd))
      .clipShape(RoundedRectangle(cornerRadius: Metrics.rMd))
      .overlay(RoundedRectangle(cornerRadius: Metrics.rMd).strokeBorder(Palette.line))
  }
}

/// Command output (Codex-style Shell card): the full command behind a muted `$`, then the output
struct TerminalOutput: View {
  let block: ToolCallBlock
  let command: String?

  var body: some View {
    let text = outputOf(block)
    if !text.isEmpty || command != nil {
      VStack(alignment: .leading, spacing: 0) {
        if let command { CommandPrompt(command: command, last: text.isEmpty) }
        if !text.isEmpty {
          OutputPane(top: command != nil ? Metrics.gapHalf : Metrics.commandY) {
            terminalLines(text)
          }
        }
      }
      .modifier(CodeOutputFrame())
      .codeViewer(title: command ?? toolVerb(block), copy: [command.map { "$ \($0)" }, text].compactMap { $0 }.joined(separator: "\n"),
                  copyLabel: t("code.copyOutput")) {
        TerminalOutput(block: block, command: command)
      }
    }
  }

  /// npm banners muted, a leading ✓ in the success colour
  func terminalLines(_ text: String) -> Text {
    let lines = text.trimmingTrailingWhitespace().components(separatedBy: "\n")
    var parts: [Text] = []
    for (i, line) in lines.enumerated() {
      let end = i < lines.count - 1 ? "\n" : ""
      if line.hasPrefix("> ") || line.hasPrefix("Done in ") {
        parts.append(Text(web: line + end, paragraph: false).foregroundStyle(Palette.fg3))
      } else if let check = line.firstIndex(of: "✓"), line[..<check].allSatisfy(\.isWhitespace) {
        parts.append(Text(web: String(line[..<check]), size: WebType.mono.size, paragraph: false))
        parts.append(Text(web: "✓", size: WebType.mono.size, paragraph: false).foregroundStyle(Palette.ok))
        parts.append(Text(web: String(line[line.index(after: check)...]) + end, paragraph: false))
      } else {
        parts.append(Text(web: line + end, paragraph: false))
      }
    }
    return parts.reduce(Text(web: "")) { Text("\($0)\($1)") }
  }
}

/// Text a non-command tool returned, wrapped (reports and prose, not column-aligned logs)
struct ToolOutput: View {
  let text: String
  @Environment(\.codeSheet) private var inSheet
  @State private var natural: CGFloat = 0

  var body: some View {
    let body = text.trimmingTrailingWhitespace()
    if !body.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
      Scroller(.vertical) {
        Text(web: body, size: WebType.mono.size)
          .textStyle(WebType.mono)
          .foregroundStyle(Palette.fg2)
          .frame(maxWidth: .infinity, alignment: .leading)
          .padding(.horizontal, Metrics.commandX)
          .padding(.vertical, Metrics.commandY)
          .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { natural = $0 }
      }
      .scrollBounceBehavior(.basedOnSize)
      .codeCap(natural: natural, inSheet: inSheet)
      .modifier(CodeOutputFrame())
      .codeViewer(title: t("code.copyToolOutput"), copy: body, copyLabel: t("code.copyToolOutput")) { ToolOutput(text: text) }
    }
  }
}

/// The scrolling output area: capped by --code-output-max, sideways scrolling for long lines
struct OutputPane<Content: View>: View {
  var top: CGFloat
  @ViewBuilder let content: () -> Content
  @Environment(\.codeSheet) private var inSheet
  @State private var natural: CGFloat = 0

  var body: some View {
    Scroller([.vertical, .horizontal]) {
      content()
        .textStyle(WebType.mono)
        .foregroundStyle(Palette.fg2)
        .fixedSize()
        .padding(.horizontal, Metrics.commandX)
        .padding(.top, top)
        .padding(.bottom, Metrics.commandY)
        .frame(maxWidth: .infinity, alignment: .leading)
        .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { natural = $0 }
    }
    .scrollBounceBehavior(.basedOnSize, axes: [.vertical, .horizontal])
    .scrollIndicators(.hidden)
    .codeCap(natural: natural, inSheet: inSheet)
  }
}

/// The full command, cut to three lines until expanded; wrapped lines hang under the command, not the `$`
struct CommandPrompt: View {
  let command: String
  var last: Bool
  @State private var expanded = false
  @State private var clipped = false
  @Environment(\.motionEnabled) private var motion

  var body: some View {
    let mono = WebType.mono
    HStack(alignment: .firstTextBaseline, spacing: 0) {
      Text(web: "$ ", size: WebType.mono.size).accessibilityHidden(true)
      VStack(alignment: .leading, spacing: 0) {
        Text(web: command, size: WebType.mono.size)
          .lineLimit(expanded ? nil : Metrics.commandPromptLines)
          .truncationMode(.tail)
          .fixedSize(horizontal: false, vertical: true)
          .background {
            // Measure the uncut height to know whether the cut hides anything
            Text(web: command, size: WebType.mono.size).textStyle(mono).fixedSize(horizontal: false, vertical: true).hidden()
              .onGeometryChange(for: CGFloat.self) { $0.size.height } action: {
                clipped = $0 > CGFloat(Metrics.commandPromptLines) * mono.lineHeight + 1
              }
          }
        if clipped {
          Button {
            withAnimation(motion ? Motion.open : nil) { expanded.toggle() }
          } label: {
            Icon(.chevronDown, size: WebMetrics.chevron, strokeWidth: 1.75)
              .rotationEffect(.degrees(expanded ? 180 : 0))
              .padding(.horizontal, Metrics.gapHalf)
              .background(Palette.fg3.opacity(0.14), in: RoundedRectangle(cornerRadius: Metrics.rSm))
          }
          .buttonStyle(.plain)
          .foregroundStyle(Palette.fg3)
        }
      }
    }
    .textStyle(mono)
    .foregroundStyle(Palette.fg3)
    .padding(.horizontal, Metrics.commandX)
    .padding(.top, Metrics.commandY)
    .padding(.bottom, last ? Metrics.commandY : 0)
    .frame(maxWidth: .infinity, alignment: .leading)
  }
}

func outputOf(_ b: ToolCallBlock) -> String {
  if let contents = b.contents, !contents.isEmpty {
    let texts = contents.compactMap { if case .text(let t) = $0 { t } else { nil } }
    if !texts.isEmpty { return texts.joined(separator: "\n") }
  }
  switch b.content {
  case .text(let t): return t
  case .list(let items): return items.joined(separator: "\n")
  case .diff(let lines, _): return lines.map(\.code).joined(separator: "\n")
  default: return ""
  }
}

extension String {
  func trimmingTrailingWhitespace() -> String {
    var s = self
    while let last = s.last, last.isWhitespace { s.removeLast() }
    return s
  }
}

// MARK: - Diff

/// DiffBlock: line numbers and signs in a gutter, tinted add / del rows, `···` between hunks. The rows fill at least
/// the card's width (`diff-table min-w-full w-max`), so a tint runs edge to edge however short its line is. Code is
/// coloured by the file's grammar (codeDiffHighlight.ts, the visible segments: the app has no whole-file source)
struct DiffBlockView: View {
  let lines: [DiffLine]
  /// The file, for the code sheet's title and the grammar
  var title = ""
  @Environment(\.codeSheet) private var inSheet
  @Environment(\.snapshot) private var snapshot
  @Environment(\.colorScheme) private var scheme
  @State private var natural: CGFloat = 0
  @State private var viewport: CGFloat = 0
  @State private var colored: Colored?

  /// A pass's tokens, one entry per line of `lines`
  struct Colored { var lines: [DiffLine]; var theme: CodeTheme; var tokens: [[CodeToken]?] }
  struct Pass: Equatable { var lines: [DiffLine]; var title: String; var theme: CodeTheme }

  /// The tokens for these lines in this scheme: the last pass's, else the highlighter's cache; the full-length render
  /// tokenizes in place, as its tasks may not run before the image is taken
  private func tokens(_ theme: CodeTheme) -> [[CodeToken]?]? {
    if let colored, colored.theme == theme, colored.lines == lines { return colored.tokens }
    return snapshot ? SyntaxHighlighter.shared.diff(lines, path: title, theme: theme)
      : SyntaxHighlighter.shared.cachedDiff(lines, path: title, theme: theme)
  }

  var body: some View {
    let mono = WebType.mono
    // One `ch` of Menlo (0.602 em), the face the desktop draws code in
    let ch = mono.size * 0.602
    let theme = CodeTheme(scheme)
    let tokens = tokens(theme)
    let rows = lines.indices.filter { i in !(lines[i].kind == .hunk && (i == 0 || i == lines.count - 1)) }
    Scroller([.vertical, .horizontal]) {
      VStack(alignment: .leading, spacing: 0) {
        ForEach(rows, id: \.self) { i in
          let line = lines[i]
          if line.kind == .hunk {
            Text(web: "···")
              .textStyle(WebType.text3)
              .foregroundStyle(Palette.fg3)
              .frame(width: (3.7 + 2.2) * ch, height: Metrics.diffHunk)
              .frame(maxWidth: .infinity, alignment: .leading)
              .overlay(alignment: .top) { Rectangle().fill(Palette.line).frame(height: 1) }
              .overlay(alignment: .bottom) { Rectangle().fill(Palette.line).frame(height: 1) }
              .padding(.vertical, Metrics.gapHalf)
          } else {
            HStack(alignment: .firstTextBaseline, spacing: 0) {
              // diff-gutter: the number right-aligned in --diff-number-width, the sign centred in --diff-sign-width.
              // A context row keeps the sign's room with a space: an empty Text has no baseline, and aligned by its
              // line box's bottom it stretched the row
              Text(web: (line.kind == .del ? line.oldLine : line.newLine).map(String.init) ?? "", size: mono.size)
                .foregroundStyle(Palette.fg3)
                .monospacedDigit()
                .frame(minWidth: 3.7 * ch, alignment: .trailing)
              Text(web: line.kind == .add ? "+" : line.kind == .del ? "−" : " ", size: mono.size)
                .foregroundStyle(line.kind == .add ? Palette.ok : line.kind == .del ? Palette.danger : Palette.fg3)
                .frame(width: 2.2 * ch)
              Group {
                if let runs = tokens?[i], !line.code.isEmpty {
                  Text(codeRuns(runs, size: mono.size))
                } else {
                  Text(web: line.code.isEmpty ? "\u{200b}" : line.code, size: mono.size)
                }
              }
                .foregroundStyle(Palette.fg1)
                .fixedSize()
                .padding(.trailing, Metrics.pad)
            }
            .textStyle(WebType.codeOutput)
            .frame(minHeight: WebMetrics.codeOutputLine)
            .frame(maxWidth: .infinity, alignment: .leading)
            // A plain rectangle: the shape-style background took a rounded container shape and cut the tint's ends
            .background(Rectangle().fill(line.kind == .add ? Palette.diffAdd : line.kind == .del ? Palette.diffDel : .clear))
          }
        }
      }
      .frame(minWidth: viewport, alignment: .leading)
      .padding(.vertical, Metrics.gapHalf)
      .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { natural = $0 }
    }
    .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { viewport = $0 }
    .scrollBounceBehavior(.basedOnSize, axes: [.vertical, .horizontal])
    .scrollIndicators(.hidden)
    .codeCap(natural: natural, inSheet: inSheet)
    .modifier(CodeOutputFrame())
    // Tokenized off the main actor: a first grammar costs a pass over its regexes
    .task(id: Pass(lines: lines, title: title, theme: theme)) {
      guard !snapshot, tokens == nil, codeLanguage(title) != nil else { return }
      let (lines, title) = (lines, title)
      let result = await Task.detached(priority: .utility) { SyntaxHighlighter.shared.diff(lines, path: title, theme: theme) }.value
      if let result { colored = Colored(lines: lines, theme: theme, tokens: result) }
    }
    // The rows as a unified diff: the wire text already starts with its sign
    .codeViewer(title: title, copy: rows.map { lines[$0] }.filter { $0.kind != .hunk }.map(\.text).joined(separator: "\n"), copyLabel: t("code.copyCode")) {
      DiffBlockView(lines: lines, title: title)
    }
  }
}

// MARK: - Code sheet

extension EnvironmentValues {
  /// Inside the code sheet: a code card grows to its whole height and scrolls there
  @Entry var codeSheet = false
}

extension View {
  /// A code card's scrolling body: cut at --code-output-max in the transcript, whole in the code sheet. The cut does
  /// not scroll, so no scroll view nests in the transcript's; a fade at the cut shows there is more, and a tap opens
  /// the whole card. The cut is the webview's, so a card under it reads as
  /// the desktop's (a shorter phone cap sliced a seven-line diff through its last line)
  func codeCap(natural: CGFloat, inSheet: Bool) -> some View {
    let preview = !inSheet
    return frame(maxHeight: inSheet ? nil : WebMetrics.codeOutputMax)
      .fixedSize(horizontal: false, vertical: true)
      .scrollDisabled(preview)
      .overlay(alignment: .bottom) {
        if preview && natural > WebMetrics.codeOutputMax {
          LinearGradient(colors: [Palette.bgCode.opacity(0), Palette.bgCode], startPoint: .top, endPoint: .bottom)
            .frame(height: Metrics.scrollFade)
            .allowsHitTesting(false)
        }
      }
  }

  /// A tap on a code card opens the whole card in the code sheet; inside the sheet the card stays as it is
  func codeViewer<Full: View>(title: String, copy: String, copyLabel: String, @ViewBuilder full: @escaping () -> Full) -> some View {
    modifier(CodeViewer(title: title, copy: copy, copyLabel: copyLabel, full: full))
  }
}

private struct CodeViewer<Full: View>: ViewModifier {
  var title: String
  var copy: String
  var copyLabel: String
  let full: () -> Full
  @Environment(\.codeSheet) private var inSheet
  @Environment(\.initialPopup) private var initialPopup
  @State private var shown = false

  func body(content: Content) -> some View {
    if !inSheet {
      content
        .contentShape(Rectangle())
        .onTapGesture { shown = true }
        .sheet(isPresented: $shown) { CodeSheet(title: title, copy: copy, copyLabel: copyLabel, content: full) }
        .task {
          // `-popup code` opens the first card that shows, for screenshots
          guard initialPopup == "code", !CodeSheetProbe.claimed else { return }
          CodeSheetProbe.claimed = true
          try? await Task.sleep(for: .seconds(1))
          shown = true
        }
    } else {
      content
    }
  }
}

/// `-popup code`: whether a card has already taken the launch's one sheet
enum CodeSheetProbe {
  nonisolated(unsafe) static var claimed = false
}

/// The code sheet: one card whole, both ways scrollable, under a title naming it, the
/// system's close button and copy in its bar
struct CodeSheet<Content: View>: View {
  var title: String
  var copy: String
  var copyLabel: String
  let content: () -> Content
  @Environment(\.dismiss) private var dismiss
  @State private var copied = false

  var body: some View {
    NavigationStack {
      ScrollView(.vertical) {
        content()
          .environment(\.codeSheet, true)
          .padding(.horizontal, Metrics.pad)
          .padding(.vertical, Metrics.s2)
      }
      .background(Palette.bg0)
      .navigationTitle(title)
      .navigationBarTitleDisplayModeInline()
      .toolbar {
        ToolbarItem(placement: .cancellationAction) {
          Button(role: .close) { dismiss() }
        }
        ToolbarItem(placement: .primaryAction) {
          Button {
            UIPasteboardBridge.copy(copy)
            copied = true
          } label: {
            Image(systemName: copied ? "checkmark" : "doc.on.doc")
          }
          .accessibilityLabel(Text(web: copied ? t("code.copied") : copyLabel))
        }
      }
    }
    .presentationDetents([.medium, .large])
    .task(id: copied) {
      guard copied else { return }
      try? await Task.sleep(for: .milliseconds(1500))
      copied = false
    }
  }
}
