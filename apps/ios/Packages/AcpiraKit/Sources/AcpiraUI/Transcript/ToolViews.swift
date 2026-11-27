import AcpiraDesign
import AcpiraModels
import SwiftUI

// chat/ToolCall.tsx, ToolGroup.tsx, Terminal.tsx and the DiffBlock of CodeBlock.tsx under toolLine = icon: every
// process row leads with its kind's icon, and an open row draws its rail (ui/ConnectedRail) from that icon.

/// chat/icons.ts toolIcon: to-do, image generation, shown images and skill loads by their verb, the rest by kind
func toolIcon(_ block: ToolCallBlock) -> LucideIcon {
  switch block.verbKey {
  case "verb.todo": .listTodo
  case "verb.imagegen": .imagePlus
  case "verb.showImage": .image
  case "verb.showVideo": .fileVideo
  case "verb.skill": .bookOpen
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
  @Environment(\.foldMemory) private var folds
  @Environment(\.entranceScope) private var entrance
  /// The hand-set state, kept by the thread too so a reused cell builds the fold as the reader left it
  private var fold: Binding<Bool?> { rememberedFold($manual, folds, "\(entrance.scope):tool:\(block.id)") }

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
                          mono: block.targetMono == true, span: span, sessionTool: SessionLinks.isTool(block) ? block : nil)
    let trailing = ToolTrailing(block: block, execute: execute)
    let icon = toolIcon(block)

    if let todos = block.todoEntries {
      PlanDetails(entries: todos) { label } trailing: { trailing }
    } else if block.verbKey == "verb.skill" && !(block.status == .failed && !visible.isEmpty) {
      // A skill load is one row naming the skill; only a failure opens to its message
      Row(tone: .action) { LeadIcon(icon) } content: { label } trailing: { trailing }
    } else if isImageResult(block) {
      // A process fold owns a separate, persistent media part; standalone tool rows keep their own result.
      VStack(alignment: .leading, spacing: Metrics.gap) {
        Row(tone: .action) { LeadIcon(icon) } content: { label } trailing: { trailing }
        ImageResults(block: block, grouped: grouped)
      }
    } else if !files.isEmpty && block.kind == .search {
      Disclosure(open: foldBinding(fold, preOpen.contains(.results)), icon: icon, indent: false, rail: .rows) { label } trailing: { trailing } content: {
        ResultList(items: files, kind: block.kind).environment(\.rowEnterDelay, Motion.rowEnterFile)
      }
    } else if files.count == 1 && block.kind == .read {
      FileRow(hit: files[0], image: toolImages(block).first, icon: icon) { label } trailing: { trailing }
    } else if !files.isEmpty {
      VStack(alignment: .leading, spacing: 0) {
        Row(tone: .action) { LeadIcon(icon) } content: { label } trailing: { trailing }
        EntranceOnce(id: "\(block.id):files") {
          ResultList(items: files, kind: block.kind, images: block.kind == .read ? toolImages(block) : [])
        }
      }
      .connectedRail(enabled: true, endAtLastRow: true)
    } else if block.kind == .read || block.kind == .search || (block.kind == .edit && visible.isEmpty && block.asyncTask == nil)
                || (grouped && block.content == nil && command == nil) {
      Row(tone: .action) { LeadIcon(icon) } content: { label } trailing: { trailing }
    } else if execute || (!visible.isEmpty && visible.allSatisfy({ if case .text = $0 { true } else { false } })) {
      // The command card sits in the label column, `codeCardGap` from the rows around it
      CommandDisclosure(block: block, command: command, items: visible, icon: icon, label: label, trailing: trailing,
                        defaultOpen: execute && !grouped && running)
    } else if case .list = block.content {
      Disclosure(open: foldBinding(fold, preOpen.contains(.results)), icon: icon, indent: false, rail: .rows) { label } trailing: { trailing } content: {
        ToolBody(block: block, items: visible)
      }
    } else {
      // A diff is a code card too: in the label column on the row's rail, spaced as the command card is (the webview
      // runs it full width, which gave the transcript two card edges)
      Disclosure(open: foldBinding(fold, preOpen.contains(.results)), icon: icon,
                 bodyTop: Metrics.codeCardGap, bodyAfter: Metrics.codeCardAfter) { label } trailing: { trailing } content: {
        VStack(alignment: .leading, spacing: Metrics.gap) {
          TaskMeta(task: block.asyncTask)
          ToolBody(block: block, items: visible)
        }
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
  var sessionTool: ToolCallBlock? = nil

  var body: some View {
    RowLabel(verb, shimmer: shimmer)
    if let command {
      RowTarget(command, mono: true)
    } else if let sessionTool {
      SessionToolTarget(block: sessionTool)
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

/// Only session target rows observe the list, in both the chat and the child inspector.
struct SessionToolTarget: View {
  let block: ToolCallBlock
  @Environment(AppModel.self) private var model: AppModel?

  var body: some View {
    let target = SessionLinks.splitTarget(block.target)
    let rest = block.verbKey == "verb.readSession" ? SessionLinks.readLabel(target.rest) : target.rest
    if let id = target.id {
      HStack(alignment: .firstTextBaseline, spacing: Metrics.s1) {
        RowTarget(SessionLinks.title(id, in: model?.summaries ?? []))
        if let rest { Aside(rest) }
      }
    } else if let rest {
      RowTarget(rest)
    }
  }
}

/// Commands end with ✓ / ✗ and their run time; other rows show their diff stat. A background task's state tag (and its
/// Stop while it runs) stands in for the glyph and, unlike the webview, for the run time too: the tag is long, a row's
/// trailing never truncates, and the command it names must keep room on a phone. A running command the session can
/// stop on its own leads with Stop
struct ToolTrailing: View {
  let block: ToolCallBlock
  let execute: Bool
  @Environment(\.canStopCommands) private var canStop

  var body: some View {
    if execute, let task = block.asyncTask {
      TaskTag(task: task)
    } else if execute {
      if canStop && commandStoppable(block) { CommandStop(id: block.id) }
      let duration = block.startedAt.flatMap { start in block.endedAt.map { commandDuration(ms: $0 - start) } }
      CommandOutcome(status: block.status, duration: duration,
                     since: block.kind == .execute && block.status == .in_progress ? block.startedAt : nil)
    } else {
      if let stat = block.diffStat { DiffStatView(add: stat.add, del: stat.del) }
      if let task = block.asyncTask { TaskTag(task: task) }
    }
  }
}

/// CommandOutcome.tsx: a command's ✓ / ✗ in a slot held while it runs, so its arrival moves nothing on the row, then its
/// run time (a live command ticks in whole seconds from `since`; a finished one reads its host timestamps). A command that
/// ends on screen draws its glyph with a small pop and fades its final time in; one already ended when the row mounted
/// stays still. Two parts, so the row's trailing slot spaces them as its own
struct CommandOutcome: View {
  let status: ToolStatus
  var duration: String?
  var since: Double?
  @Environment(\.quietEntrance) private var quiet
  @Environment(\.motionEnabled) private var motion
  @Environment(\.accessibilityReduceMotion) private var reduceMotion
  @Environment(\.scrollGesture) private var gesture
  @State private var memo = OutcomeMemo()

  /// Whether the command was seen running, and the ends played so far. A class, so noting the end while the body is
  /// evaluated starts the draw in the frame the glyph appears; the animators sit on views that never come and go, or a
  /// new one would take the changed trigger as its first value and play nothing
  final class OutcomeMemo {
    var live: Bool?
    var ends = 0
  }

  var body: some View {
    let done = status == .completed || status == .failed
    let _ = note(done)
    ZStack {
      if done { OutcomeGlyph(ok: status == .completed) }
    }
    .frame(width: WebMetrics.chevron, height: WebMetrics.chevron)
    // The stroke runs out along its own path, as the webview's dash offset draws it
    .keyframeAnimator(initialValue: 1.0, trigger: memo.ends) { glyph, p in
      glyph.environment(\.outcomeDraw, min(1, max(0, p)))
        .scaleEffect(OutcomeKeys.popFrom + (1 - OutcomeKeys.popFrom) * p)
    } keyframes: { _ in
      KeyframeTrack {
        MoveKeyframe(0.0)
        SpringKeyframe(1.0, duration: Motion.outcomeDraw, spring: .bouncy)
      }
    }
    .accessibilityHidden(true)
    // The slot keeps at least the width of the widest time below a minute ("0.1 s"), so the ticking whole seconds
    // turning into the final tenths never shift the command beside them
    ZStack(alignment: .trailing) {
      if duration != nil || since != nil { Text(web: commandDuration(ms: 50)).hidden() }
      if let duration {
        Text(web: duration)
      } else if let since {
        CommandSeconds(since: since)
      }
    }
    .keyframeAnimator(initialValue: 1.0, trigger: memo.ends) { time, p in
      time.opacity(p).offset(y: (1 - p) * Metrics.hairline * 2)
    } keyframes: { _ in
      KeyframeTrack {
        MoveKeyframe(0.0)
        LinearKeyframe(0.0, duration: Motion.outcomeTimeDelay)
        LinearKeyframe(1.0, duration: Motion.outcomeTime, timingCurve: .easeOut)
      }
    }
  }

  private func note(_ done: Bool) {
    if memo.live == nil { memo.live = !done; return }
    if done && memo.live == true {
      memo.live = false
      if !quiet && motion && !reduceMotion && !gesture.active { memo.ends += 1 }
    }
  }
}

/// The draw's start: the glyph grows from this scale as its stroke runs out
private enum OutcomeKeys {
  static let popFrom = 0.6
}

// Nonisolated: the draw's keyframe closure forms the key path off the main actor
nonisolated extension EnvironmentValues {
  /// How much of the outcome glyph's stroke is drawn, 0 to 1 (at rest)
  @Entry fileprivate var outcomeDraw: Double = 1
}

/// The ✓ / ✗ stroke, drawn as far as `outcomeDraw` says
private struct OutcomeGlyph: View {
  let ok: Bool
  @Environment(\.outcomeDraw) private var draw

  var body: some View {
    IconShape(icon: ok ? .check : .x)
      .trim(from: 0, to: draw)
      .stroke(style: StrokeStyle(lineWidth: 2 * WebMetrics.chevron / 24, lineCap: .round, lineJoin: .round))
      .foregroundStyle(ok ? Palette.ok : Palette.danger)
  }
}

extension EnvironmentValues {
  /// The session on screen can stop a running command on its own (SessionView.canStopCommands)
  @Entry var canStopCommands = false
}

/// A command the agent handed over verbatim and still running: the only rows whose process the engine can find
func commandStoppable(_ block: ToolCallBlock) -> Bool {
  block.kind == .execute && block.status == .in_progress && block.observation != .unknown && block.targetMono == true
    && block.verbKey == nil && !(block.target ?? "").trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
}

/// Stop on a running command's row, the same square as a background task's: ends that command's process and the turn
/// goes on. Gone once tapped, so a second tap has nothing to land on while the process dies
struct CommandStop: View {
  let id: String
  @Environment(\.sessionActions) private var actions
  @State private var asked = false

  var body: some View {
    if !asked {
      Button {
        Haptics.play(.stop)
        asked = true
        actions.stopCommand(id)
      } label: {
        Icon(.square, size: WebMetrics.chevron, strokeWidth: 1.5)
      }
      .buttonStyle(IconButtonStyle(side: WebMetrics.ctlSm))
      .padding(.vertical, -Metrics.s1)
      .padding(.trailing, -Metrics.s1)
      .accessibilityLabel(Text(web: t("command.stop")))
    }
  }
}

/// An AIR async task on its row: the state ("Stopping…" once a stop is asked; done in the ok tone, failed in danger),
/// then Stop while the task runs and the adapter can stop it. A refusal comes back as the engine's error toast, and the
/// row is live again with its Stop
struct TaskTag: View {
  let task: AsyncTaskInfo
  @Environment(\.sessionActions) private var actions

  var body: some View {
    let state = taskRowState(task)
    Text(web: state.tag)
      .foregroundStyle(state.tone == .danger ? Palette.danger : state.tone == .ok ? Palette.ok : Palette.fg3)
      .lineLimit(1)
    if state.stoppable {
      // Small enough to sit in a process row without growing it, as the webview's sm button with a size-3 glyph
      Button {
        Haptics.play(.stop)
        actions.stopTask(task.id)
      } label: {
        Icon(.square, size: WebMetrics.chevron, strokeWidth: 1.5)
      }
      .buttonStyle(IconButtonStyle(side: WebMetrics.ctlSm))
      .padding(.vertical, -Metrics.s1)
      .padding(.trailing, -Metrics.s1)
      .accessibilityLabel(Text(web: t("asyncTask.stop")))
    }
  }
}

/// TaskMeta: the task's own report under its row, its latest summary, the tool it was last on and its usage. The
/// output file stays hidden: the card already shows the command
struct TaskMeta: View {
  let task: AsyncTaskInfo?

  var body: some View {
    if let task {
      let usage = taskUsage(task)
      // A workflow's last tool is whichever of its agents reported last, which says nothing on its own
      let tool = task.taskType == "workflow" ? nil : task.lastToolName
      if task.summary != nil || tool != nil || usage != nil {
        VStack(alignment: .leading, spacing: Metrics.s1) {
          if let summary = task.summary { Text(web: summary).fixedSize(horizontal: false, vertical: true) }
          if let tool { Text(web: tool) }
          if let usage { Text(web: usage) }
        }
        .textStyle(WebType.text3)
        .foregroundStyle(Palette.fg3)
        .frame(maxWidth: .infinity, alignment: .leading)
      }
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
  @Environment(\.foldMemory) private var folds
  @Environment(\.entranceScope) private var entrance
  /// The hand-set state, kept by the thread too so a reused cell builds the fold as the reader left it
  private var fold: Binding<Bool?> { rememberedFold($manual, folds, "\(entrance.scope):command:\(block.id)") }
  @Environment(\.preOpen) private var preOpen

  init(block: ToolCallBlock, command: String?, items: [ToolContent], icon: LucideIcon, label: Label, trailing: Trailing,
       defaultOpen: Bool) {
    self.block = block; self.command = command; self.items = items; self.icon = icon; self.label = label; self.trailing = trailing
    self.defaultOpen = defaultOpen
  }

  var body: some View {
    // The card sits in the label column on the row's rail; the room after it stays outside the rail root (animated
    // through `bodyAfter`'s `FoldLayout`) so the end dot meets the card without snapping the hosted cell height.
    // The phone keeps it there too: a column-wide card without the rail lost the desktop's reading of the row (2026-10-27)
    Disclosure(open: foldBinding(fold, defaultOpen || preOpen.contains(.command)), icon: icon,
               bodyTop: Metrics.codeCardGap, bodyAfter: Metrics.codeCardAfter) { label } trailing: { trailing } content: {
      VStack(alignment: .leading, spacing: Metrics.gap) {
        TaskMeta(task: block.asyncTask)
        if block.kind == .execute {
          // An image the command produced (screenshot tools) sits below its output (ToolCall.tsx ToolBody)
          TerminalOutput(block: block, command: command)
          ForEach(Array(toolImages(block).enumerated()), id: \.offset) { _, image in AgentImageView(blob: image.blob, uri: image.uri) }
        } else {
          ToolOutput(text: items.compactMap { if case .text(let t) = $0 { t } else { nil } }.joined(separator: "\n"))
        }
      }
    }
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
        case .diff(let lines, let source):
          DiffBlockView(lines: lines, title: block.target ?? "", source: source, locate: diffLocation(block, lines))
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

/// File / hit rows under a read or search: dense action rows, the name in mono fg-2 and the line as a faint note.
/// `images`: a read's saved images, which its file rows open
struct ResultList: View {
  let items: [String]
  let kind: ToolKind
  var images: [ImageRef] = []

  var body: some View {
    let icon: LucideIcon = kind == .fetch ? .globe : .fileText
    VStack(alignment: .leading, spacing: 0) {
      ForEach(Array(items.enumerated()), id: \.element) { i, item in
        if kind == .read || kind == .search {
          FileRow(hit: item, image: rowImage(item, at: i, of: items, images: images), dense: true, icon: icon)
        } else {
          let hit = splitHit(item)
          Row(tone: .action, dense: true) { LeadIcon(icon) } content: {
            RowTarget(hit.main, mono: kind != .fetch, ink: Palette.fg2)
            // A URL's host is arbitrary text, not fixed-width row metadata.
            if let aside = hit.aside { Aside(aside) }
          }
        }
      }
    }
  }
}

/// ResultList's pairing of a file row with the call's saved images: the one image of a lone row, else the image whose
/// uri names the row's file, else the image at the row's place when there is one per row or per image row
func rowImage(_ item: String, at index: Int, of items: [String], images: [ImageRef]) -> ImageRef? {
  guard !images.isEmpty else { return nil }
  if items.count == 1 && images.count == 1 { return images[0] }
  let main = splitHit(item).main
  let named = images.first { image in
    guard let uri = image.uri else { return false }
    let path = fileOfURI(uri) ?? uri
    return uri == item || path == item || (!main.isEmpty && (path.hasSuffix("/\(main)") || path.hasSuffix("\\\(main)")))
  }
  if let named { return named }
  if images.count == items.count { return images[index] }
  var imageIndex: Int?
  var imageCount = 0
  for (j, hit) in items.enumerated() {
    let f = fileReference(hit)
    if f.line == nil, imageMimeOf(f.path) != nil {
      if j == index { imageIndex = imageCount }
      imageCount += 1
    }
  }
  return imageIndex.flatMap { imageCount == images.count ? images[$0] : nil }
}

/// What a file row opens in the Lightbox (ToolCall.tsx FileRef): the call's saved copy of the image, else the file
/// itself where the link can stage it. nil for a line reference and any other file: the row stays plain, since the
/// phone opens nothing else from a process row
func fileRowImage(_ hit: String, image: ImageRef?, session: String?, canStage: Bool) -> MediaSource? {
  let file = fileReference(hit)
  guard file.line == nil, let session else { return nil }
  if let stored = MediaSource.stored(image?.blob, session: session) { return stored }
  guard canStage, imageMimeOf(file.path) != nil else { return nil }
  return .file(session: session, path: file.path, shown: false)
}

/// attachments.ts imageMimeOf: the image types the engine stages (`image_mime_of`), by extension
func imageMimeOf(_ name: String) -> String? {
  guard let dot = name.lastIndex(of: "."), !name[dot...].contains(where: { $0 == "/" || $0 == "\\" }) else { return nil }
  let mimes = ["png": "image/png", "jpg": "image/jpeg", "jpeg": "image/jpeg", "gif": "image/gif", "webp": "image/webp"]
  return mimes[name[name.index(after: dot)...].lowercased()]
}

/// A read or search row's file (FileResultRow / FileRef): its name in mono fg-2 and the line as a faint note. A file
/// with an image to show (`fileRowImage`) is underlined, as the desktop underlines it under the pointer (a phone has no
/// hover), and the whole row opens it in the Lightbox. Its pixels may still have to come over the link: the file glyph
/// then turns into the desktop's spinner until they land, and a picture that cannot load says so in a toast
struct FileRow<Label: View, Trailing: View>: View {
  let hit: String
  var image: ImageRef?
  var dense: Bool
  var icon: LucideIcon
  let label: Label
  let trailing: Trailing
  @Environment(\.media) private var media
  @Environment(\.sessionActions) private var actions
  @Environment(\.motionEnabled) private var motion
  @Environment(\.initialPopup) private var initialPopup
  /// The tap's load while the pixels are on their way
  @State private var pending: Task<Void, Never>?

  init(hit: String, image: ImageRef? = nil, dense: Bool = false, icon: LucideIcon = .fileText,
       @ViewBuilder label: () -> Label, @ViewBuilder trailing: () -> Trailing) {
    self.hit = hit; self.image = image; self.dense = dense; self.icon = icon
    self.label = label(); self.trailing = trailing()
  }

  var body: some View {
    let split = splitHit(hit)
    let source = fileRowImage(hit, image: image, session: media.session, canStage: media.loader.canStage)
    let loading = pending != nil
    Row(tone: .action, dense: dense) {
      // The glyph stays in place, hidden, so the rail keeps measuring the file icon under the spinner
      LeadIcon(icon)
        .opacity(loading ? 0 : 1)
        .overlay { if loading { LoadingGlyph(size: WebMetrics.icon).transition(.opacity) } }
    } content: {
      label
      HStack(alignment: .firstTextBaseline, spacing: Metrics.s1) {
        RowTarget(split.main, mono: true, ink: Palette.fg2, underline: source != nil)
        if let aside = split.aside { Aside(aside) }
      }
    } trailing: { trailing }
    .onTapGesture { if let source { open(source, name: split.main) } }
    .accessibilityAddTraits(source != nil ? .isButton : [])
    .onDisappear { settle() }
    .task {
      // `-popup readimage` taps the first file row with an image, for screenshots
      guard initialPopup == "readimage", let source, !ReadImageProbe.claimed else { return }
      ReadImageProbe.claimed = true
      try? await Task.sleep(for: .seconds(1))
      open(source, name: split.main)
    }
  }

  /// Opens the picture at once when it is at hand, else after its load; a second tap while it loads does nothing
  private func open(_ source: MediaSource, name: String) {
    guard pending == nil, let lightbox = media.lightbox else { return }
    if let image = media.loader.peek(source) {
      lightbox.open(LightboxItem(image: image, name: name, source: source))
      return
    }
    let (loader, toast, motion) = (media.loader, actions.toast, motion)
    let task = Task {
      let image = try? await loader.image(source)
      // A row that left the screen meanwhile opens nothing: the reader has moved on (the load still fills the cache)
      guard !Task.isCancelled else { return }
      withAnimation(motion ? Motion.quick : nil) { pending = nil }
      if let image {
        lightbox.open(LightboxItem(image: image, name: name, source: source))
      } else {
        toast(t("attach.loadFailed", ["name": name]), .image)
      }
    }
    withAnimation(motion ? Motion.quick : nil) { pending = task }
  }

  private func settle() {
    pending?.cancel()
    pending = nil
  }
}

extension FileRow where Label == EmptyView, Trailing == EmptyView {
  init(hit: String, image: ImageRef? = nil, dense: Bool = false, icon: LucideIcon = .fileText) {
    self.init(hit: hit, image: image, dense: dense, icon: icon, label: { EmptyView() }, trailing: { EmptyView() })
  }
}

/// `-popup readimage` opens once per launch
enum ReadImageProbe {
  static var claimed = false
}

/// The desktop's loading glyph (Loader2 under animate-spin: one turn a second, linear) at a row icon's size and stroke.
/// It keeps turning with motion off, as the list's working ring does: a still one reads as stuck. Drawn as a bare shape,
/// so it offers no rail anchor of its own
struct LoadingGlyph: View {
  var size: CGFloat

  var body: some View {
    Loop(period: 1) {
      IconShape(icon: .loaderCircle)
        .stroke(style: StrokeStyle(lineWidth: 1.5 * size / 24, lineCap: .round, lineJoin: .round))
        .frame(width: size, height: size)
    } move: { glyph, far in
      glyph.rotationEffect(.degrees(far ? 360 : 0))
    }
    .accessibilityLabel(Text(web: t("attach.loading")))
  }
}

// MARK: - Groups

/// A run of consecutive reads or edits under one head: the verb, the files it touched, an edit run's total change
struct ToolGroupView: View {
  let kind: GroupKind
  let blocks: [ToolCallBlock]
  @Environment(\.preOpen) private var preOpen
  @State private var manual: Bool?
  @Environment(\.foldMemory) private var folds
  @Environment(\.entranceScope) private var entrance
  /// The hand-set state, kept by the thread too so a reused cell builds the fold as the reader left it
  private var fold: Binding<Bool?> { rememberedFold($manual, folds, "\(entrance.scope):group:\(kind):\(blocks.first?.id ?? "")") }

  var body: some View {
    let current = blocks.last(where: { $0.status == .pending || $0.status == .in_progress }) ?? blocks.last!
    let names = groupNames(blocks)
    let stat = kind == .edit ? diffStatOf(blocks) : nil
    // Edit entries have indented cards below their rows; keep the group's rail beside those cards too.
    Disclosure(open: foldBinding(fold, preOpen.contains(.results)), icon: kind == .read ? .fileText : .pencil,
               indent: false, rail: .rows, endAtLastRow: kind == .read, bodyTop: 0) {
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
      ResultList(items: files, kind: .read, images: toolImages(block))
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
  @Environment(\.foldMemory) private var folds
  @Environment(\.entranceScope) private var entrance
  /// The hand-set state, kept by the thread too so a reused cell builds the fold as the reader left it
  private var fold: Binding<Bool?> { rememberedFold($manual, folds, "\(entrance.scope):edit:\(blocks.first?.id ?? "")") }
  @Environment(\.preOpen) private var preOpen

  var body: some View {
    let stat = diffStatOf(blocks)
    let span = editSpan(blocks)
    let diffs = blocks.flatMap { block in
      visibleToolContents(block).filter { if case .diff = $0 { true } else if case .text = $0 { true } else { false } }.map { (block, $0) }
    }
    let label = HStack(alignment: .firstTextBaseline, spacing: Metrics.s1) {
      RowTarget(fileName(blocks[0]), mono: true, ink: Palette.fg2)
      if let span { Aside(span) }
    }
    let trailing = Group { if let stat { DiffStatView(add: stat.add, del: stat.del) } }
    if diffs.isEmpty {
      Row(tone: .action, dense: true) { LeadIcon(.pencil) } content: { label } trailing: { trailing }
    } else {
      // EntryFold: no rail root of its own, so the group's rail runs on through its icon. The diff is a code card in
      // the label column like every other (entries stack without the process rows' gap, so it takes the whole gap)
      Disclosure(open: foldBinding(fold, preOpen.contains(.edit)), dense: true, icon: .pencil, root: false,
                 bodyTop: Metrics.codeCardGap, bodyAfter: Metrics.codeCardGap) {
        label
      } trailing: { trailing } content: {
        VStack(alignment: .leading, spacing: Metrics.gap) {
          ForEach(Array(diffs.enumerated()), id: \.offset) { _, entry in
            let (block, item) = entry
            switch item {
            case .diff(let lines, let source):
              DiffBlockView(lines: lines, title: fileName(blocks[0]), source: source, locate: diffLocation(block, lines))
            case .text(let text): ToolOutput(text: text)
            default: EmptyView()
            }
          }
        }
      }
    }
  }
}

