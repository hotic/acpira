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
    TimelineView(.animation) { context in
      IconShape(icon: .loaderCircle)
        .stroke(style: StrokeStyle(lineWidth: 1.5 * size / 24, lineCap: .round, lineJoin: .round))
        .frame(width: size, height: size)
        .rotationEffect(.degrees(context.date.timeIntervalSinceReferenceDate.truncatingRemainder(dividingBy: 1) * 360))
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
  @Environment(\.codeSheet) private var inSheet

  /// The lines the transcript's card can show. It is cut at --code-output-max and never scrolls there, so a running
  /// command's streamed output is built and laid out only down to the cut, not whole on every patch; one line more
  /// keeps the card past the cut so its fade still shows. The code sheet shows every line
  private static let previewLines = Int((WebMetrics.codeOutputMax / WebType.mono.lineHeight).rounded(.up)) + 1

  var body: some View {
    let text = outputOf(block)
    if !text.isEmpty || command != nil {
      VStack(alignment: .leading, spacing: 0) {
        if let command { CommandPrompt(command: command, last: text.isEmpty) }
        if !text.isEmpty {
          OutputPane(top: command != nil ? Metrics.gapHalf : Metrics.commandY) {
            terminalLines(text, limit: inSheet ? nil : Self.previewLines)
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

  /// npm banners muted, a leading ✓ in the success colour; the first `limit` lines only when given
  func terminalLines(_ text: String, limit: Int? = nil) -> Text {
    var lines = text.trimmingTrailingWhitespace().components(separatedBy: "\n")
    if let limit, lines.count > limit { lines.removeSubrange(limit...) }
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
    return Text.joined(parts)
  }
}

/// Text a non-command tool returned, wrapped (reports and prose, not column-aligned logs)
struct ToolOutput: View {
  let text: String
  @Environment(\.codeSheet) private var inSheet

  var body: some View {
    let body = text.trimmingTrailingWhitespace()
    if !body.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
      Group {
        if inSheet {
          Scroller(.vertical) { content(body) }
            .scrollBounceBehavior(.basedOnSize)
        } else {
          // The transcript preview never scrolls; laying it out directly avoids `UIScrollView`'s late
          // self-sizing inside `FoldLayout` (which popped the card in several frames late over rows below).
          CappedPreview(limit: WebMetrics.codeOutputMax) { content(body) }
        }
      }
      .modifier(CodeOutputFrame())
      .codeViewer(title: t("code.copyToolOutput"), copy: body, copyLabel: t("code.copyToolOutput")) { ToolOutput(text: text) }
    }
  }

  private func content(_ body: String) -> some View {
    Text(web: body, size: WebType.mono.size)
      .textStyle(WebType.mono)
      .foregroundStyle(Palette.fg2)
      .fixedSize(horizontal: false, vertical: true)
      .frame(maxWidth: .infinity, alignment: .leading)
      .padding(.horizontal, Metrics.commandX)
      .padding(.vertical, Metrics.commandY)
  }
}

/// The scrolling output area: capped by --code-output-max, sideways scrolling for long lines (in the transcript too)
struct OutputPane<Content: View>: View {
  var top: CGFloat
  @ViewBuilder let content: () -> Content
  @Environment(\.codeSheet) private var inSheet

  var body: some View {
    if inSheet {
      Scroller([.vertical, .horizontal]) { styledContent }
        .scrollBounceBehavior(.basedOnSize, axes: [.vertical, .horizontal])
        .scrollIndicators(.hidden)
    } else {
      HorizontalCappedPreview(limit: WebMetrics.codeOutputMax) {
        styledContent
      } content: {
        Scroller(.horizontal) { styledContent.codeCut(inSheet: false) }
          .scrollBounceBehavior(.basedOnSize, axes: .horizontal)
          .scrollIndicators(.hidden)
      }
    }
  }

  private var styledContent: some View {
    content()
      .textStyle(WebType.mono)
      .foregroundStyle(Palette.fg2)
      .fixedSize()
      .padding(.horizontal, Metrics.commandX)
      .padding(.top, top)
      .padding(.bottom, Metrics.commandY)
      .frame(maxWidth: .infinity, alignment: .leading)
  }
}

/// The full command, cut to three lines until expanded; wrapped lines hang under the command, not the `$`
struct CommandPrompt: View {
  let command: String
  var last: Bool
  @State private var expanded = false
  @State private var clipped = false
  @Environment(\.motionEnabled) private var motion
  @Environment(\.holdScroll) private var holdScroll

  var body: some View {
    let mono = WebType.mono
    HStack(alignment: .firstTextBaseline, spacing: 0) {
      Text(web: "$ ", size: WebType.mono.size).accessibilityHidden(true)
      VStack(alignment: .leading, spacing: 0) {
        Group {
          if clipped {
            // The height eases between the cut and the whole command through the layout, as a fold's reveal does.
            // Switching the line limit inside the animation resized the hosted thread row in one step, and the rows
            // above the card jumped and slid back
            LineClamp(progress: expanded ? 1 : 0) {
              Text(web: command, size: WebType.mono.size)
                .fixedSize(horizontal: false, vertical: true)
                .opacity(expanded ? 1 : 0)
                .accessibilityHidden(!expanded)
              cut.opacity(expanded ? 0 : 1).accessibilityHidden(expanded)
            }
            .clipped()
          } else {
            cut
          }
        }
          .background {
            // Measure the uncut height to know whether the cut hides anything
            Text(web: command, size: WebType.mono.size).textStyle(mono).fixedSize(horizontal: false, vertical: true).hidden()
              .onGeometryChange(for: CGFloat.self) { $0.size.height } action: {
                clipped = $0 > CGFloat(Metrics.commandPromptLines) * mono.lineHeight + 1
              }
          }
        if clipped {
          Button {
            toggle()
          } label: {
            Icon(.chevronDown, size: WebMetrics.chevron, strokeWidth: 1.75)
              .rotationEffect(.degrees(expanded ? 180 : 0))
              .padding(.horizontal, Metrics.gapHalf)
              .background(Palette.fg3.opacity(0.14), in: RoundedRectangle(cornerRadius: Metrics.rSm))
          }
          .buttonStyle(.plain)
          .foregroundStyle(Palette.fg3)
          .accessibilityLabel(Text(web: t(expanded ? "code.commandCollapse" : "code.commandExpand")))
        }
      }
    }
    .textStyle(mono)
    .foregroundStyle(Palette.fg3)
    .padding(.horizontal, Metrics.commandX)
    .padding(.top, Metrics.commandY)
    .padding(.bottom, last ? Metrics.commandY : 0)
    .frame(maxWidth: .infinity, alignment: .leading)
    // The chevron alone is far below a finger's size, and a miss opened the code sheet: a cut command expands from
    // anywhere on its lines, ahead of the card's own tap
    .contentShape(Rectangle())
    .gesture(TapGesture().onEnded { toggle() }, including: clipped ? .all : .subviews)
  }

  /// The command cut at `commandPromptLines`
  private var cut: some View {
    Text(web: command, size: WebType.mono.size)
      .lineLimit(Metrics.commandPromptLines)
      .truncationMode(.tail)
      .fixedSize(horizontal: false, vertical: true)
  }

  private func toggle() {
    let next = !expanded
    holdScroll()
    withAnimation(foldAnimation(opening: next, motion: motion)) { expanded = next }
  }
}

/// The whole text (first subview) shown down to a height between the cut text's (second subview) and its own:
/// `FoldLayout` with the cut as its floor. Both sit at the top; the caller clips and cross-fades them
private struct LineClamp: Layout {
  var progress: CGFloat
  var animatableData: CGFloat {
    get { progress }
    set { progress = newValue }
  }

  func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
    let free = ProposedViewSize(width: proposal.width, height: nil)
    let whole = subviews.first?.sizeThatFits(free) ?? .zero
    let cut = subviews.dropFirst().first?.sizeThatFits(free) ?? whole
    let shown = min(1, max(0, progress))
    return CGSize(width: proposal.width ?? max(whole.width, cut.width), height: cut.height + (whole.height - cut.height) * shown)
  }

  func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
    for subview in subviews {
      subview.place(at: bounds.origin, anchor: .topLeading, proposal: ProposedViewSize(width: bounds.width, height: nil))
    }
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
  /// The whole files behind the diff (a page view keeps the path only) and where the diff sits, to ask for them
  var source: DiffSource?
  var locate: DiffLocation?
  @Environment(\.diffSource) private var fetchSource
  @Environment(\.codeSheet) private var inSheet
  @Environment(\.snapshot) private var snapshot
  @Environment(\.motionEnabled) private var motion
  @Environment(\.colorScheme) private var scheme
  @State private var viewport: CGFloat = 0
  @State private var rowWidth: CGFloat = 0
  @State private var colored: Colored?

  /// A pass's tokens, one entry per line of `lines`
  struct Colored { var lines: [DiffLine]; var theme: CodeTheme; var tokens: [[CodeToken]?] }
  struct Pass: Equatable { var lines: [DiffLine]; var title: String; var theme: CodeTheme }

  /// Only a completed pass is read while drawing. The full-length render tokenizes in place because it runs no tasks.
  private func tokens(_ lines: [DiffLine], theme: CodeTheme) -> [[CodeToken]?]? {
    if let colored, colored.theme == theme, colored.lines == lines { return colored.tokens }
    if snapshot { return SyntaxHighlighter.shared.diff(lines, path: title, theme: theme) }
    // Only the bounded preview reads synchronously; full diffs never rebuild their cache keys while drawing.
    return inSheet ? nil : SyntaxHighlighter.shared.cachedDiff(lines, path: title, theme: theme)
  }

  var body: some View {
    let mono = WebType.mono
    // One `ch` of Menlo (0.602 em), the face the desktop draws code in
    let ch = mono.size * 0.602
    let theme = CodeTheme(scheme)
    // The transcript never shows beyond its cap. Do not build or highlight the hidden tail just to clip it later.
    let display = inSheet ? lines : diffPreview(lines)
    let tokens = tokens(display, theme: theme)
    Group {
      if inSheet {
        // The reader owns the whole bounded viewport, even when the diff has only a few lines.
        sheetScroller(display, tokens: tokens, ch: ch)
          .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
      } else {
        // Resolve preview height and minimum row width synchronously on the first pass so opening the fold
        // glides from frame 1 instead of stalling on `UIScrollView` self-sizing and per-row geometry callbacks.
        HorizontalCappedPreview(limit: WebMetrics.codeOutputMax) {
          Color.clear.frame(width: 0, height: diffPreviewNaturalHeight(display))
        } content: {
          GeometryReader { proxy in
            Scroller(.horizontal) {
              VStack(alignment: .leading, spacing: 0) {
                rows(display, tokens: tokens, ch: ch, measureWidth: false)
              }
              .frame(minWidth: proxy.size.width, alignment: .leading)
              .padding(.vertical, Metrics.gapHalf)
              .codeCut(inSheet: false)
            }
            .scrollBounceBehavior(.basedOnSize, axes: .horizontal)
            .scrollIndicators(.hidden)
          }
        }
        .modifier(CodeOutputFrame())
      }
    }
    // Full-source cache lookup and tokenization stay off the main actor; a stale pass must not replace a newer source.
    .task(id: Pass(lines: display, title: title, theme: theme)) {
      guard !snapshot, tokens == nil, codeLanguage(title) != nil else { return }
      let started = ContinuousClock.now
      let title = title
      let result = await Task.detached(priority: .utility) { SyntaxHighlighter.shared.diff(display, path: title, theme: theme) }.value
      guard !Task.isCancelled, let result else { return }
      // Avoid rebuilding every diff row in the middle of an opening fold spring; apply the highlight once settled.
      if !inSheet && motion {
        let elapsed = started.duration(to: .now)
        let settle = Duration.seconds(Motion.foldSettle)
        if elapsed < settle { try? await Task.sleep(for: settle - elapsed) }
        guard !Task.isCancelled else { return }
      }
      colored = Colored(lines: display, theme: theme, tokens: result)
    }
    // CodeBlock.tsx: copy takes the new file, asked for when the view left its text out, else the visible new code.
    // The diff owns its viewport: an outer vertical scroller would make even a lazy table lay out its entire height.
    .codeViewer(title: title, copy: copyText, copyLabel: t(copiesSource ? "code.copySource" : "code.copyVisible"), read: readSource,
                wrapInScrollView: false, edgeToEdge: true) {
      DiffBlockView(lines: lines, title: title, source: source, locate: locate)
    }
  }

  private func sheetScroller(_ display: [DiffLine], tokens: [[CodeToken]?]?, ch: CGFloat) -> some View {
    Scroller([.vertical, .horizontal]) {
      Group {
        if !snapshot {
          LazyVStack(alignment: .leading, spacing: 0) { rows(display, tokens: tokens, ch: ch, measureWidth: true) }
        } else {
          VStack(alignment: .leading, spacing: 0) { rows(display, tokens: tokens, ch: ch, measureWidth: false) }
        }
      }
      // Keep the widest mounted row even after it is recycled, so sideways range never shrinks while reading.
      .frame(minWidth: max(viewport, rowWidth), alignment: .leading)
      .padding(.vertical, Metrics.gapHalf)
    }
    .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { viewport = $0 }
    .scrollBounceBehavior(.basedOnSize, axes: [.vertical, .horizontal])
    .scrollIndicators(.hidden)
  }

  private func rows(_ display: [DiffLine], tokens: [[CodeToken]?]?, ch: CGFloat, measureWidth: Bool) -> some View {
    let mono = WebType.mono
    let rows = display.indices.filter { i in !(display[i].kind == .hunk && (i == 0 || i == display.count - 1)) }
    return ForEach(rows, id: \.self) { i in
      let line = display[i]
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
        let row = HStack(alignment: .firstTextBaseline, spacing: 0) {
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
        .fixedSize(horizontal: true, vertical: false)
        Group {
          if measureWidth {
            row.onGeometryChange(for: CGFloat.self) { $0.size.width } action: { rowWidth = max(rowWidth, $0) }
          } else {
            row
          }
        }
        .frame(minHeight: WebMetrics.codeOutputLine)
        .frame(maxWidth: .infinity, alignment: .leading)
        // A plain rectangle: the shape-style background took a rounded container shape and cut the tint's ends
        .background(Rectangle().fill(line.kind == .add ? Palette.diffAdd : line.kind == .del ? Palette.diffDel : .clear))
      }
    }
  }

  private var copyText: String { diffCopyText(lines, source) }

  private var fetchable: Bool {
    if case .omitted? = source, locate != nil, fetchSource != nil { true } else { false }
  }

  private var copiesSource: Bool {
    if case .full? = source { true } else { fetchable }
  }

  /// The new file from the engine; the visible code when it no longer has the call
  private var readSource: (() async -> String)? {
    guard fetchable, let locate, let fetchSource else { return nil }
    let fallback = copyText
    return { await fetchSource(locate)?.newText ?? fallback }
  }
}

// MARK: - Code sheet

extension EnvironmentValues {
  /// Inside the code sheet: a code card grows to its whole height and scrolls there
  @Entry var codeSheet = false
}

/// Non-scrolling transcript preview capped at `limit` on the first layout pass, drawing the bottom fade only when
/// natural height exceeds `limit` (no `@State` measurement loop or late `UIScrollView` sizing inside `FoldLayout`).
private struct CappedPreview<Content: View>: View {
  var limit: CGFloat
  @ViewBuilder let content: () -> Content

  var body: some View {
    CappedBox(limit: limit) {
      content()
      LinearGradient(colors: [Palette.bgCode.opacity(0), Palette.bgCode], startPoint: .top, endPoint: .bottom)
        .allowsHitTesting(false)
    }
    .clipped()
  }
}

private struct CappedBox: Layout {
  var limit: CGFloat

  func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
    guard let content = subviews.first else { return .zero }
    let natural = content.sizeThatFits(ProposedViewSize(width: proposal.width, height: nil))
    return CGSize(width: proposal.width ?? natural.width, height: min(natural.height, limit))
  }

  func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
    guard let content = subviews.first else { return }
    let natural = content.sizeThatFits(ProposedViewSize(width: bounds.width, height: nil))
    content.place(at: bounds.origin, anchor: .topLeading,
                  proposal: ProposedViewSize(width: bounds.width, height: natural.height))
    guard let fade = subviews.dropFirst().first else { return }
    if natural.height > limit {
      let h = min(Metrics.scrollFade, bounds.height)
      fade.place(at: CGPoint(x: bounds.minX, y: bounds.maxY - h), anchor: .topLeading,
                 proposal: ProposedViewSize(width: bounds.width, height: h))
    } else {
      fade.place(at: bounds.origin, anchor: .topLeading, proposal: ProposedViewSize(width: 0, height: 0))
    }
  }
}

/// Horizontally scrolling transcript preview whose vertical height comes synchronously from `ruler` on frame 1,
/// keeping `FoldLayout` reveals smooth without waiting for UIKit's `UIScrollView` to self-size.
private struct HorizontalCappedPreview<Ruler: View, Content: View>: View {
  var limit: CGFloat
  @ViewBuilder let ruler: () -> Ruler
  @ViewBuilder let content: () -> Content

  var body: some View {
    HorizontalCappedLayout(limit: limit) {
      ruler()
        .hidden()
        .allowsHitTesting(false)
        .accessibilityHidden(true)
      content()
      LinearGradient(colors: [Palette.bgCode.opacity(0), Palette.bgCode], startPoint: .top, endPoint: .bottom)
        .allowsHitTesting(false)
    }
    .clipped()
  }
}

private struct HorizontalCappedLayout: Layout {
  var limit: CGFloat

  func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
    guard let ruler = subviews.first else { return .zero }
    let natural = ruler.sizeThatFits(ProposedViewSize(width: nil, height: nil))
    return CGSize(width: proposal.width ?? natural.width, height: min(natural.height, limit))
  }

  func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
    guard subviews.count >= 2 else { return }
    let natural = subviews[0].sizeThatFits(ProposedViewSize(width: nil, height: nil))
    let height = min(natural.height, limit)
    subviews[0].place(at: bounds.origin, anchor: .topLeading, proposal: ProposedViewSize(width: 0, height: 0))
    subviews[1].place(at: bounds.origin, anchor: .topLeading, proposal: ProposedViewSize(width: bounds.width, height: height))
    guard subviews.count >= 3 else { return }
    if natural.height > limit {
      let h = min(Metrics.scrollFade, height)
      subviews[2].place(at: CGPoint(x: bounds.minX, y: bounds.maxY - h), anchor: .topLeading,
                        proposal: ProposedViewSize(width: bounds.width, height: h))
    } else {
      subviews[2].place(at: bounds.origin, anchor: .topLeading, proposal: ProposedViewSize(width: 0, height: 0))
    }
  }
}

/// Natural height of `display` in the transcript diff preview (`VStack` rows + vertical padding), matching
/// `DiffBlockView.rows` synchronously so `HorizontalCappedPreview` never waits on `UIScrollView` measurement.
func diffPreviewNaturalHeight(_ display: [DiffLine]) -> CGFloat {
  var total = Metrics.gapHalf * 2
  for (i, line) in display.enumerated() {
    if line.kind == .hunk {
      guard i != 0, i != display.count - 1 else { continue }
      total += Metrics.diffHunk + Metrics.gapHalf * 2
    } else {
      total += WebMetrics.codeOutputLine
    }
  }
  return total
}

extension View {
  /// The transcript's cut inside a sideways-only scroller: the content keeps its top --code-output-max, so the strip
  /// is no taller than the card and a swipe across it scrolls long lines
  @ViewBuilder func codeCut(inSheet: Bool) -> some View {
    if inSheet {
      self
    } else {
      frame(maxHeight: WebMetrics.codeOutputMax, alignment: .top).clipped()
    }
  }

  /// A tap on a code card opens the whole card in the code sheet; inside the sheet the card stays as it is
  /// read: the text copy takes when it has to be fetched first (a diff's whole file); `copy` until then
  /// wrapInScrollView: false when the content owns a viewport, so its lazy rows see a bounded height
  func codeViewer<Full: View>(title: String, copy: String, copyLabel: String, read: (() async -> String)? = nil,
                              wrapInScrollView: Bool = true, edgeToEdge: Bool = false,
                              @ViewBuilder full: @escaping () -> Full) -> some View {
    modifier(CodeViewer(title: title, copy: copy, copyLabel: copyLabel, read: read,
                        wrapInScrollView: wrapInScrollView, edgeToEdge: edgeToEdge, full: full))
  }
}

private struct CodeViewer<Full: View>: ViewModifier {
  var title: String
  var copy: String
  var copyLabel: String
  var read: (() async -> String)?
  var wrapInScrollView: Bool
  var edgeToEdge: Bool
  let full: () -> Full
  @Environment(\.codeSheet) private var inSheet
  @Environment(\.initialPopup) private var initialPopup
  @State private var shown = false

  func body(content: Content) -> some View {
    if !inSheet {
      content
        .contentShape(Rectangle())
        .onTapGesture { shown = true }
        .sheet(isPresented: $shown) {
          CodeSheet(title: title, copy: copy, copyLabel: copyLabel, read: read,
                    wrapInScrollView: wrapInScrollView, edgeToEdge: edgeToEdge, content: full)
        }
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
  var read: (() async -> String)? = nil
  var wrapInScrollView = true
  /// Readers with their own viewport fill the sheet without an inset card.
  var edgeToEdge = false
  var detents: Set<PresentationDetent> = [.medium, .large]
  var copyEnabled = true
  /// Optional raw / rendered toggle; copying always uses the original text.
  var markdownPreview: Binding<Bool>? = nil
  let content: () -> Content
  @Environment(\.dismiss) private var dismiss
  @State private var copied = false

  var body: some View {
    NavigationStack {
      Group {
        if wrapInScrollView {
          ScrollView(.vertical) { sheetContent }
        } else {
          sheetContent
        }
      }
      .frame(maxWidth: .infinity, maxHeight: edgeToEdge ? .infinity : nil, alignment: .topLeading)
      .background(edgeToEdge ? Palette.bgCode : Palette.bg0)
      .navigationTitle(title)
      .navigationBarTitleDisplayModeInline()
      .toolbar {
        ToolbarItem(placement: .cancellationAction) {
          CloseButton { dismiss() }
        }
        ToolbarItemGroup(placement: .primaryAction) {
          if let markdownPreview {
            Button { markdownPreview.wrappedValue.toggle() } label: {
              Image(systemName: markdownPreview.wrappedValue ? "book.fill" : "book")
            }
            .barButtonBase()
            .accessibilityLabel(Text(web: t("panel.markdownPreview")))
            .accessibilityAddTraits(markdownPreview.wrappedValue ? .isSelected : [])
          }
          Button {
            guard let read else {
              UIPasteboardBridge.copy(copy)
              Haptics.play(.copy)
              copied = true
              return
            }
            Task {
              UIPasteboardBridge.copy(await read())
              Haptics.play(.copy)
              copied = true
            }
          } label: {
            Image(systemName: copied ? "checkmark" : "doc.on.doc")
          }
          .barButtonBase()
          .accessibilityLabel(Text(web: copied ? t("code.copied") : copyLabel))
          .disabled(!copyEnabled)
        }
      }
    }
    .presentationDetents(detents)
    .task(id: copied) {
      guard copied else { return }
      try? await Task.sleep(for: .milliseconds(1500))
      copied = false
    }
  }

  private var sheetContent: some View {
    content()
      .environment(\.codeSheet, true)
      .padding(.horizontal, edgeToEdge ? 0 : Metrics.pad)
      .padding(.vertical, edgeToEdge ? 0 : Metrics.s2)
  }
}

/// Enough rows to paint past the transcript's cut, including a leading hunk header (which is not drawn).
/// Keep the original array for the sheet and copy; only the preview's layout and highlighter use this prefix.
nonisolated func diffPreview(_ lines: [DiffLine]) -> [DiffLine] {
  // Hunk separators can be shorter than code rows; even a card full of them must still paint past the fade.
  let shortest = min(WebMetrics.codeOutputLine, Metrics.diffHunk + Metrics.gapHalf * 2)
  let limit = Int((WebMetrics.codeOutputMax / shortest).rounded(.up)) + 1
  return Array(lines.prefix(limit + (lines.first?.kind == .hunk ? 1 : 0)))
}

// MARK: - Diff sources

/// Where a diff sits in the session: its tool call and which of the call's diffs (CodeBlock.tsx DiffLocation)
nonisolated struct DiffLocation: Hashable, Sendable {
  var toolCallId: String
  var nth: Int
}

/// The location of the diff with these lines, counted among the call's diffs (`contents`, or `content` alone)
nonisolated func diffLocation(_ block: ToolCallBlock, _ lines: [DiffLine]) -> DiffLocation? {
  let diffs = (block.contents ?? block.content.map { [$0] } ?? []).compactMap { item -> [DiffLine]? in
    if case .diff(let lines, _) = item { lines } else { nil }
  }
  return diffs.firstIndex(of: lines).map { DiffLocation(toolCallId: block.id, nth: $0) }
}

/// codeDiff.ts diffCopyText: the new file when the diff carries it, else the visible new code without signs
nonisolated func diffCopyText(_ lines: [DiffLine], _ source: DiffSource?) -> String {
  if case .full(let full)? = source { return full.newText }
  return lines.filter { $0.kind == .add || $0.kind == .ctx }.map(\.code).joined(separator: "\n")
}

extension EnvironmentValues {
  /// Asks the engine for a diff's whole files in the chat's session (nil: no engine, or it no longer has the call)
  @Entry var diffSource: ((DiffLocation) async -> FullDiffSource?)? = nil
}
