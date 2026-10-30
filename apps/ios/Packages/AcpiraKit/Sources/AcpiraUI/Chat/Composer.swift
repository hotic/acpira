import AcpiraDesign
import AcpiraFixtures
import AcpiraModels
import QuickLook
import SwiftUI
#if canImport(PhotosUI)
import PhotosUI
#endif
#if canImport(UIKit)
import UIKit
#endif

// The phone composer. The webview's composer (a rounded-lg field over a dense toolbar row) reads as a desktop control
// on a phone, so iOS draws its own: one Liquid Glass bar with `+` and the model's mark at its start and send at its
// end, growing into a card while typing. The mark opens liquid beans holding everything the desktop toolbar spreads
// over chips: the model, its effort and parameters, the mode and any other agent option (Goo.swift). Picked
// attachments wait in a strip above the field. A `/` or `@` token under the caret raises the command or mention list
// over the strip (Suggestions.swift), and the context ring sits beside send (ContextUsage.swift).

struct Composer: View {
  let session: SessionView
  @Binding var text: String
  @Binding var staged: [Attachment]
  var onSend: (String, [Attachment]) -> Void = { _, _ in }
  var onStop: () -> Void = {}
  var onSetMode: (String) -> Void = { _ in }
  var onSetConfig: (String, String) -> Void = { _, _ in }
  /// The new-session screen's prompt: no run to stop, the first message starts the session
  var placeholder: String?
  /// A new value focuses the field (a sent prompt taken in for editing, HistoryEditor's autoFocus)
  var focusKey: Int?
  @FocusState private var focused: Bool
  /// The card form, owned here rather than read off `focused`. FocusState turns true only once the text view has
  /// become first responder, a round trip through the keyboard service: 140–250 ms after the tap in the simulator, and
  /// on a phone after the keyboard had already risen, so the card began to grow late, in a transaction of its own
  @State private var card = false
  @Environment(\.snapshot) private var snapshot
  @Environment(\.motionEnabled) private var motion
  /// The liquid chip that is open (attach, model), if any, the bean whose panel is out (1…, 0 for none), the list the
  /// sidekick's panel opened in place of its rows, and the picker `+` opened
  @State private var goo: String?
  @State private var gooStage = 0
  @State private var gooDeeper: String?
  @State private var source: AttachSource?
  /// The width of the bar's group of `+` and the mark (the mark is gone when the agent offers nothing to pick)
  @State private var lead = Metrics.barCtl * 2
  /// The chips' top on the screen below its bar (`GooColumn.space`): how far their beans may rise
  @State private var chipTop = CGFloat.infinity
  @Environment(\.initialPopup) private var initialPopup
  /// The field's selection: the caret the `/` and `@` tokens are read at
  @State private var selection: TextSelection?
  /// The text as the field last wrote it or a pick set it; any other change (a restored draft, an edited queue item)
  /// puts the caret at the end, where the field left it at its old offset
  @State private var typed: String?
  /// The pasteboard holds an image, so `+` offers to paste it
  @State private var pasteReady = false
  /// The field's height, so the expand corner shows once the text runs to `expandLines`
  @State private var fieldHeight: CGFloat = 0
  /// The staged strip's height with its gap, so the expand corner sits on the first line under it
  @State private var stripHeight: CGFloat = 0
  /// The draft is open in the whole-screen editor (ExpandedComposer)
  @State private var fullEditor = false
  /// The editor closed for a file page rather than back to the card
  @State private var leftEditor = false
  /// The card shows a plain plate for its glass: from the moment a picker opens over it until the last drop is home
  @State private var plate = false
  @Environment(FileSearch.self) private var files: FileSearch?
  @Environment(\.mentionPersonas) private var personas
  @Environment(\.compactAt) private var compactAt
  @Environment(\.sessionActions) private var actions

  private var hint: String { placeholder ?? (session.running ? t("composer.placeholder.queue") : t("composer.placeholder")) }

  /// The bar's hint at rest: its first sentence, cut before the `@` tip, on one line. The tip shows once the bar has
  /// grown into the card. A sentence too long for the bar ends in an ellipsis, where the wrapped hint stopped halfway
  /// through a phrase
  private var restHint: String {
    hint.split(separator: "@", maxSplits: 1).first.map { $0.trimmingCharacters(in: .whitespaces) } ?? hint
  }

  private var filled: Bool { !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !staged.isEmpty }

  /// The card form: opened for typing, or a draft waiting
  private var expanded: Bool { card || filled }

  /// The field: a snapshot draws the placeholder (or the draft) where the field would be
  private var field: some View {
    let rest = !expanded
    return Group {
      if snapshot {
        Text(web: text.isEmpty ? rest ? restHint : hint : text)
          .foregroundStyle(text.isEmpty ? Palette.fg3 : Palette.fgStrong)
          .frame(maxWidth: .infinity, alignment: .leading)
      } else {
        VStack(alignment: .leading, spacing: 0) {
          // The card's cap in whole lines of the field's own line box. lineLimit's cap counts the font's natural line
          // height (about 20.6 pt at this size), not the exact one the field draws, so the last line ran cut under it
          FieldHeight(cap: rest ? TypeScale.field.lineHeight : CGFloat(Self.maxLines) * TypeScale.field.lineHeight) {
            TextField(text: fieldText, selection: $selection, axis: .vertical) {
              // At rest the hint is drawn over the field instead (below)
              Text(web: rest ? "" : hint).foregroundStyle(Palette.fg3)
            }
            .focused($focused)
          }
          // At rest the field takes no touches (the bar's tap focuses it) and its hint is plain text over it: a drag
          // on the bar scrolled the field's own text view, and the hint stayed cut by the one-line frame
          .allowsHitTesting(!rest)
          .overlay(alignment: .topLeading) {
            if rest && text.isEmpty {
              Text(web: restHint).foregroundStyle(Palette.fg3).allowsHitTesting(false)
                // Gone at once, the frame the field's own hint appears: faded over the card's growth it showed under
                // that hint, set with other spacing, and the two read as garbled doubled text
                .transition(.identity)
            }
          }
          .accessibilityLabel(Text(web: hint))
          // The command's own wording for what follows it, a faint second line while its arguments are empty
          if let commandHint {
            Text(web: commandHint, paragraph: false)
              .textStyle(TypeScale.mono)
              .foregroundStyle(Palette.fg3)
              .lineLimit(1)
          }
        }
      }
    }
    .textStyle(TypeScale.field)
    .foregroundStyle(Palette.fgStrong)
    // The field's height comes from FieldHeight; this bound only has to sit past anything it lets through
    .lineLimit(rest ? 1...1 : 1...Self.growLines)
    .frame(minHeight: TypeScale.field.lineHeight, alignment: .topLeading)
  }

  /// The lines the card grows to before the field scrolls inside itself
  private static let maxLines = 8
  /// lineLimit's bound, past anything FieldHeight lets through
  private static let growLines = 1000
  /// The lines from which the card offers the whole-screen editor (ChatGPT's expand corner)
  private static let expandLines = 3

  /// The field's top inset in both forms: the text centred in the bar, or below the staged strip
  private var fieldTop: CGFloat {
    staged.isEmpty ? (Metrics.barCtl - TypeScale.field.lineHeight) / 2 + Metrics.fieldInkDrop : Metrics.s2
  }

  /// The card holds enough lines to offer the whole-screen editor
  private var offersExpand: Bool {
    expanded && fieldHeight >= TypeScale.field.lineHeight * CGFloat(Self.expandLines) - Metrics.hairline
  }

  /// The expand corner at the card's top trailing side, centred on the first line, in the column the card keeps free
  /// for it, so its arrival never rewraps the text
  @ViewBuilder private var expandCorner: some View {
    if offersExpand {
      Button { fullEditor = true } label: {
        Icon(.maximize2, size: Metrics.icon)
          .foregroundStyle(Palette.cmpFg2)
          .frame(width: Metrics.s3 * 2 + Metrics.icon, height: TypeScale.field.lineHeight)
          .contentShape(Rectangle())
      }
      .buttonStyle(.plain)
      .accessibilityLabel(Text(web: t("mobile.composer.expand")))
      .padding(.top, (staged.isEmpty ? 0 : stripHeight) + fieldTop)
      .transition(.opacity)
    }
  }

  private var send: some View {
    SendButton(running: session.running, filled: filled) {
      if session.running && !filled {
        onStop()
      } else {
        onSend(text, staged)
        text = ""
        staged = []
      }
    }
  }

  /// At rest one glass bar: `+` and the model's mark at its start, the field, and send at
  /// its end, where Messages keeps it (dim with nothing to send, stop while a turn runs). `+` and the mark are liquid
  /// chips. Typing (focused, or a draft waiting) the bar grows into one glass card as in Codex: the field across the
  /// whole width over a row of the chips and send, so the text is never squeezed between controls. Nothing comes or
  /// goes between the two: the chips keep the bottom-left corner and send the bottom-right one, the glass grows up
  /// around them, and the field stays the same view throughout, so focus holds. The chips and send are glass circles
  /// of their own, as tall as the bar, that its container melts into the bar: their glyphs are glass content, a press
  /// lights only the circle under the finger, and the beans melt out of a chip. The beans are glass in the same
  /// container (`gooGlass()`), over a dim the bar spreads under itself
  var body: some View {
    let corner = expanded ? Metrics.rComposer : Metrics.barCtl / 2
    let line = (Metrics.barCtl - TypeScale.field.lineHeight) / 2
    let room = chipTop - Metrics.s2
    return VStack(alignment: .leading, spacing: Metrics.s1_5) {
      suggestions
      GlassEffectContainer(spacing: Metrics.gooMerge) {
        ZStack(alignment: .bottomLeading) {
          VStack(alignment: .leading, spacing: 0) {
            // Staged attachments ride inside the card over the text, as in the desktop composer; anything staged
            // keeps the card open. The strip runs to the card's trailing side and its tiles start on the text's edge
            if !staged.isEmpty {
              StagedAttachments(items: $staged)
                .padding(.top, Metrics.s1)
                .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { stripHeight = $0 }
                .padding(.leading, Metrics.s3 + Metrics.s1)
            }
            field
              // Room for the context ring beside send in the bar
              .padding(.trailing, expanded || session.usage == nil ? 0 : Metrics.ctlSm)
              .onGeometryChange(for: CGFloat.self) { $0.size.height } action: { fieldHeight = $0 }
              // Lowered by the ink's offset in both forms, so the text is centred in the bar and does not move as it
              // grows
              .padding(.top, fieldTop)
              .padding(.bottom, expanded ? Metrics.s1 + Metrics.barCtl + Metrics.s1 : line - Metrics.fieldInkDrop)
              // In the card the text starts on the `+`'s ink below it
              .padding(.leading, expanded ? Metrics.s3 + Metrics.s1 * 2 : lead + Metrics.s1)
              // In the card the text stops short of the expand corner's column
              .padding(.trailing, expanded ? Metrics.s3 + Metrics.icon + Metrics.s1 : Metrics.barCtl + Metrics.s1)
          }
          // While a picker is out the card's text and strip recede with the page, so only the picker reads. Under
          // beans lying on the draft they blur as well, or the lines read through between the beans
          .animation(recedeCurve) { $0.opacity(goo != nil ? Metrics.gooRecede : 1).blur(radius: receded ? Metrics.s1 : 0) }
          // Strip tiles scrolled to the side end at the card's rounded edge
          .clipShape(RoundedRectangle(cornerRadius: corner, style: .continuous))
          // Under the beans the glass gives way to a plain plate, dimmed with the page: beans over glass in the same
          // container melt into it, over a plate they stay whole. The chips keep their own glass circles, so the beans
          // still leave the chip. Glass cannot fade, so the plate stands in for it at once, outlined as a card, and the
          // glass only comes back once the last drop is home: dropped while the plate faded in, the card was gone for
          // a moment, and back as the beans left, the drops crossed sharp glass on their way home
          .background {
            let shape = RoundedRectangle(cornerRadius: corner, style: .continuous)
            ZStack {
              shape.fill(Palette.cardBg)
                .overlay(shape.strokeBorder(Palette.cardLine))
                .opacity(plate ? 1 : 0)
                .transaction { $0.animation = nil }
              shape.fill(Palette.gooDim)
                .animation(recedeCurve) { $0.opacity(receded ? 1 : 0) }
            }
          }
          .glassEffect(plate ? .identity : GlassStyle.control, in: RoundedRectangle(cornerRadius: corner, style: .continuous))
          .contentShape(RoundedRectangle(cornerRadius: corner, style: .continuous))
          .onTapGesture(perform: focusField)
          // The chips' row at the start; the room after them takes no taps, so they reach the field under it
          HStack(spacing: 0) {
            HStack(spacing: 0) {
              LiquidAttach(open: $goo, stage: $gooStage, room: room, paste: paste, personas: personas,
                           onSummon: summon) { source = $0 }
              LiquidControls(controls: session.controls, open: $goo, stage: $gooStage, deeper: $gooDeeper, room: room,
                             onSetMode: onSetMode, onSetConfig: onSetConfig)
            }
            .gooGlass()
            // On the card the chips are bare glyphs with no glass circle of their own (`GooChip`)
            .environment(\.gooBare, expanded)
            .onGeometryChange(for: CGFloat.self) { $0.size.width } action: { lead = $0 }
            .onGeometryChange(for: CGFloat.self) { $0.frame(in: .named(GooColumn.space)).minY } action: { chipTop = $0 }
            Spacer(minLength: 0)
          }
          // Inset from the card's corner as send is from its own: flush with the card's side, a pressed chip's glass
          // (the whole slot) crossed the card's rounded corner
          .padding([.leading, .bottom], expanded ? Metrics.s1 : 0)
        }
      }
      // Send sits over the container, not in it: the container draws its glass over every plain view inside, so a
      // solid button there came out refracted into a soft blob
      .overlay(alignment: .bottomTrailing) {
        HStack(spacing: 0) {
          ring
          send
        }
        .padding([.trailing, .bottom], expanded ? Metrics.s1 : 0)
        .animation(recedeCurve) { $0.opacity(goo != nil ? Metrics.gooRecede : 1) }
      }
      // Over the container for the same reason as send
      .overlay(alignment: .topTrailing) {
        expandCorner.animation(recedeCurve) { $0.opacity(goo != nil ? Metrics.gooRecede : 1) }
      }
      .animation(motion ? Motion.composerMorph : nil, value: expanded)
      .animation(motion ? Motion.quick : nil, value: offersExpand)
    }
    // The backdrop under an open picker, as Messages blurs and dims its page under `+`: under the bar's glass, over the
    // transcript. Blurred as well as dimmed, so a caption never reads against the words or cards behind it (a dim alone
    // left the floating to-do card legible under the beans). Taps on it go to the screen's `gooLayer()`
    .background {
      Rectangle()
        .fill(.ultraThinMaterial)
        .overlay(Palette.gooDim)
        .frame(width: Metrics.gooScrim, height: Metrics.gooScrim)
        .opacity(goo == nil ? 0 : 1)
        .allowsHitTesting(false)
        // It lifts only as the pieces get home, so the last drop never crosses the bare transcript
        .animation(motion ? goo == nil ? Motion.gooDimOut : Motion.gooSpring : nil, value: goo)
    }
    .onAppear(perform: startPopup)
    // Collapsing gives the card its focus back; leaving for a file page does not, or the keyboard rose over that page
    .modifier(FullEditorCover(open: $fullEditor, onDismiss: { if leftEditor { leftEditor = false } else { focused = true } }) {
      ExpandedComposer(text: $text, staged: $staged, filled: filled, onSend: {
        onSend(text, staged)
        text = ""
        staged = []
        fullEditor = false
      }, onCollapse: { fullEditor = false }, onLeave: {
        leftEditor = true
        fullEditor = false
      })
    })
    .modifier(AttachPickers(staged: $staged, source: $source))
    .modifier(PasteWatch(ready: $pasteReady, forced: initialPopup == "attach.paste"))
    // Mention.tsx useMentionHits: the query goes out a moment after the last keystroke, and the list closing drops
    // whatever is still on its way
    .task(id: mentionSpan?.query) { await searchFiles(mentionSpan?.query) }
    .onChange(of: text) { _, now in
      guard now != typed else { return }
      typed = now
      selection = TextSelection(insertionPoint: now.endIndex)
    }
    .onChange(of: focusKey) { _, key in if key != nil { focusField() } }
    // The plate stands in for the card's glass as a picker opens over it, until its last drop is home
    .onChange(of: goo) { _, now in if now != nil, expanded { plate = true } }
    .task(id: goo) {
      guard goo == nil, plate, (try? await Task.sleep(for: .seconds(motion ? Motion.gooHome : 0))) != nil else { return }
      plate = false
    }
    // The card follows focus that comes or goes some other way (the keyboard swiped away, a pick that sets `focused`)
    .onChange(of: focused) { _, now in
      guard now != card else { return }
      withAnimation(motion ? Motion.composerMorph : nil) { card = now }
    }
    // A focus that never lands gives the card back
    .task(id: card) {
      guard card, !focused, (try? await Task.sleep(for: Self.focusGrace)) != nil, card, !focused else { return }
      withAnimation(motion ? Motion.composerMorph : nil) { card = false }
    }
    // A screen with something of its own over the composer's beans (the new session's hero) hides it while they are out
    .preference(key: ComposerPickerKey.self, value: goo != nil)
    .environment(\.composerInk, true)
  }

  /// The bar's tap and every other way in: the card opens at once, in an animated transaction that also moves the
  /// thread above it, and the field asks for the keyboard. The card no longer waits for focus to land, so it grows as
  /// the keyboard rises instead of after it
  private func focusField() {
    if !card { withAnimation(motion ? Motion.composerMorph : nil) { card = true } }
    focused = true
  }

  /// How long the card stays open waiting for a focus that has not landed
  private static let focusGrace = Duration.seconds(2)

  /// A picker is out over the card: its beans rise from the chip straight over the draft, and the card recedes under
  /// them. Lifted over the card's top edge instead they stood in a pile over the thread whenever the keyboard was up;
  /// the keyboard stays, so the draft is still being written when the picker closes
  private var receded: Bool { goo != nil && expanded }

  /// What the card recedes and comes back on: the picker's spring on the way out, the page's dim on the way home, so
  /// the draft is only clear again once the beans are back in their chip
  private var recedeCurve: Animation? { motion ? goo == nil ? Motion.gooDimOut : Motion.gooSpring : nil }

  /// The context window ring beside send, once the agent has reported usage
  @ViewBuilder private var ring: some View {
    if let usage = session.usage {
      ContextRing(usage: usage, canCompact: session.commands.contains { $0.name == "compact" }, compactAt: compactAt,
                  running: session.running, onCompact: actions.compact)
    }
  }

  /// The prompt as the field edits it, noting what it wrote
  private var fieldText: Binding<String> {
    Binding(get: { text }, set: { typed = $0; text = $0 })
  }

  /// The paste bean's action while the pasteboard holds an image
  private var paste: (() -> Void)? { pasteReady ? { pasteImage() } : nil }

  private func searchFiles(_ query: String?) async {
    guard let query else {
      files?.close()
      return
    }
    try? await Task.sleep(for: Self.debounce)
    if !Task.isCancelled { files?.search(query) }
  }

  /// Mention.tsx's wait before a query goes out
  private static let debounce = Duration.milliseconds(60)

  // MARK: Commands and mentions

  /// The caret as a character offset (the end until the field reports a selection), and whether nothing is selected
  private var caret: (at: Int, collapsed: Bool) {
    guard let selection, case .selection(let range) = selection.indices else { return (text.count, true) }
    let end = min(range.upperBound, text.endIndex)
    return (text.distance(from: text.startIndex, to: end), range.isEmpty)
  }

  private var slashSpan: SigilSpan? {
    let c = caret
    return c.collapsed ? SlashCommands.commandAt(text, caret: c.at) : nil
  }

  private var mentionSpan: SigilSpan? {
    let c = caret
    guard c.collapsed, files != nil || !personas.isEmpty else { return nil }
    return SlashCommands.mentionAt(text, caret: c.at)
  }

  /// The commands matching the `/` token, the list shown only with hits: without one the slash stays plain text
  private var slashHits: [SlashCommand] {
    slashSpan.map { SlashCommands.match(session.commands, query: $0.query) } ?? []
  }

  /// The input hint of the command the text names, while the list is not showing it
  private var commandHint: String? { slashHits.isEmpty ? SlashCommands.hint(session.commands, text: text) : nil }

  /// The `/` list, else the `@` list (the personas, then the files the engine found for the query)
  @ViewBuilder private var suggestions: some View {
    let commands = slashHits
    if !commands.isEmpty {
      SuggestionList(items: commands, key: \.name, pick: pickCommand) { CommandRow(command: $0) }
    } else if let span = mentionSpan {
      let hits = MentionPersona.hits(personas, query: span.query) + (files?.hits ?? [])
      let answered = files?.answered == span.query
      if !hits.isEmpty || (answered && !span.query.isEmpty) {
        SuggestionList(items: hits, empty: t("mention.noFiles"), key: \.uri, pick: pickMention) { hit in
          MentionRow(hit: hit, persona: MentionPersona.of(hit, in: personas))
        }
      }
    }
  }

  /// A command completes the token in place (`/name `); it goes out through the ordinary send, the agent runs it
  private func pickCommand(_ command: SlashCommand) {
    guard let span = slashSpan else { return }
    apply(SlashCommands.complete(text, span: span, caret: caret.at, name: command.name))
  }

  /// A persona is summoned by name; a file leaves the text and rides along as an attachment
  private func pickMention(_ hit: FileHit) {
    guard let span = mentionSpan else { return }
    if let persona = MentionPersona.of(hit, in: personas) {
      apply(SlashCommands.summon(text, name: persona.name, at: span.start, cut: caret.at - span.start))
      return
    }
    apply(SlashCommands.dropToken(text, span: span, caret: caret.at))
    if !staged.contains(where: { if case .file(let uri, _) = $0 { uri == hit.uri } else { false } }) {
      staged.append(.file(uri: hit.uri, name: hit.path))
    }
  }

  /// SummonMenu: `@name ` at the caret
  private func summon(_ persona: MentionPersona) {
    apply(SlashCommands.summon(text, name: persona.name, at: caret.at))
    focusField()
  }

  /// The edited prompt, the caret after what was written (set again once the field has taken the new text)
  private func apply(_ edit: PromptEdit) {
    typed = edit.text
    text = edit.text
    let at = min(edit.caret, text.count)
    selection = TextSelection(insertionPoint: text.index(text.startIndex, offsetBy: at))
    Task { selection = TextSelection(insertionPoint: text.index(text.startIndex, offsetBy: min(at, text.count))) }
  }

  /// The pasteboard's image, staged as a picked photo is (reading it asks the system's paste permission)
  private func pasteImage() {
    #if canImport(UIKit)
    guard let data = UIPasteboard.general.image?.jpegData(compressionQuality: 0.9),
          let image = AttachmentImage.thumbnail(data) else { return }
    staged.append(.image(blob: image, mimeType: "image/jpeg", name: nil))
    #endif
  }

  /// `-popup attach`, `-popup model` or `-popup model.2` (a bean's panel out; `attach.N` for `+`, `model.N.model` or
  /// `model.N.effort` for a list the sidekick's panel opens, and `attach.paste`
  /// offers the paste bean whatever the pasteboard holds) for screenshots; a `focus.` prefix
  /// (`-popup focus`, `-popup focus.model`) raises the keyboard first. With motion on both wait a moment after launch,
  /// so a recording shows them change. `-popup demo` plays every liquid transition in turn, for a recording read
  /// frame by frame
  private func startPopup() {
    guard var popup = initialPopup else { return }
    // A `.staged` suffix only has the chat seed the strip (`demo.staged`, `focus.model.staged`)
    if popup.hasSuffix(".staged") { popup = String(popup.dropLast(".staged".count)) }
    if popup == "demo" {
      Task { await demo() }
      return
    }
    // `-popup expand`: the draft in the whole-screen editor (`staged.expand` with the staged attachments the chat seeds)
    if popup == "expand" || popup == "staged.expand" {
      Task {
        if motion { try? await Task.sleep(for: .seconds(1)) }
        fullEditor = true
      }
      return
    }
    let focus = popup.hasPrefix("focus")
    if focus { popup = String(popup.dropFirst("focus".count).drop { $0 == "." }) }
    let parts = popup.split(separator: ".", maxSplits: 2).map(String.init)
    let chip = ["attach", "model"].contains(parts.first ?? "") ? parts.first : nil
    let stage = parts.count > 1 ? Int(parts[1]) ?? 0 : 0
    let deeper = parts.count > 2 ? parts[2] : nil
    Task {
      if motion { try? await Task.sleep(for: .seconds(1)) }
      if focus { focused = true }
      if motion, focus, chip != nil { try? await Task.sleep(for: .seconds(1)) }
      withAnimation(motion ? Motion.gooSpring : nil) {
        goo = chip
        gooStage = stage
        gooDeeper = deeper
      }
    }
  }

  /// The demo's steps, each after a pause: the model's beans, two panels in and out, one panel back and another bean
  /// tapped as soon as the beans take taps (the first panel's spring still settling), closing, `+`, then the same from
  /// the card, closing from an open panel; last the sidekick's bean where the agent has one: its panel, the model list
  /// in and back, the level list, and closing from inside it
  private func demo() async {
    var steps: [(Double, () -> Void)] = [
      (1.0, { goo = "model" }), (1.2, { gooStage = 2 }), (1.4, { gooStage = 0 }), (1.2, { gooStage = 1 }),
      (1.4, { gooStage = 0 }), (1.2, { gooStage = 3 }), (1.4, { gooStage = 0 }), (0.5, { gooStage = 2 }),
      (1.4, { goo = nil; gooStage = 0 }), (1.4, { goo = "attach" }), (1.2, { goo = nil }), (1.2, { focused = true }),
      (1.4, { goo = "model" }), (1.2, { gooStage = 3 }), (1.4, { goo = nil; gooStage = 0 }), (1.4, { focused = false }),
    ]
    if let k = ModelChoices(controls: session.controls, onSetConfig: { _, _ in }).sidekickStage {
      steps += [(1.4, { goo = "model" }), (1.2, { gooStage = k }), (1.4, { gooDeeper = "model" }), (1.4, { gooDeeper = nil }),
                (1.2, { gooDeeper = "effort" }), (1.4, { goo = nil; gooStage = 0 }), (1.0, { gooDeeper = nil })]
    }
    for (pause, step) in steps {
      try? await Task.sleep(for: .seconds(pause))
      withAnimation(motion ? Motion.gooSpring : nil) { step() }
    }
  }
}

/// Whether a composer's liquid picker is out, for the screen around it
struct ComposerPickerKey: PreferenceKey {
  static let defaultValue = false
  static func reduce(value: inout Bool, nextValue: () -> Bool) { value = value || nextValue() }
}

/// Whether the pasteboard holds an image, checked without reading it (which would ask the paste permission): on
/// launch, on a change made in the app, and on coming back from another app
struct PasteWatch: ViewModifier {
  @Binding var ready: Bool
  /// `-popup attach.paste`: offered whatever the pasteboard holds (screenshots)
  var forced = false

  func body(content: Content) -> some View {
    #if canImport(UIKit)
    content
      .onAppear(perform: check)
      .onReceive(NotificationCenter.default.publisher(for: UIPasteboard.changedNotification)) { _ in check() }
      .onReceive(NotificationCenter.default.publisher(for: UIApplication.didBecomeActiveNotification)) { _ in check() }
    #else
    content
    #endif
  }

  private func check() {
    #if canImport(UIKit)
    ready = forced || UIPasteboard.general.hasImages
    #endif
  }
}

// MARK: - Attach

/// The system pickers behind `+`, shown for `source`: photos and files go through the system pickers and wait in the
/// strip above the field; the camera opens where the device has one, else a toast says there is none. Hosted by a
/// view that stays put (the composer, not its `+`, which moves between the bar and the card)
struct AttachPickers: ViewModifier {
  @Binding var staged: [Attachment]
  @Binding var source: AttachSource?
  #if canImport(PhotosUI)
  @State private var picked: [PhotosPickerItem] = []
  #endif
  @Environment(\.sessionActions) private var actions

  private static var cameraAvailable: Bool {
    #if canImport(UIKit)
    UIImagePickerController.isSourceTypeAvailable(.camera)
    #else
    false
    #endif
  }

  private func shown(_ s: AttachSource) -> Binding<Bool> {
    Binding(get: { source == s && (s != .camera || Self.cameraAvailable) }, set: { if !$0, source == s { source = nil } })
  }

  func body(content: Content) -> some View {
    content
      #if canImport(PhotosUI)
      .photosPicker(isPresented: shown(.photos), selection: $picked, maxSelectionCount: 4, matching: .images)
      .onChange(of: picked) { _, items in
        guard !items.isEmpty else { return }
        picked = []
        Task {
          for item in items {
            if let data = try? await item.loadTransferable(type: Data.self), let image = AttachmentImage.thumbnail(data) {
              staged.append(.image(blob: image, mimeType: "image/jpeg", name: nil))
            }
          }
        }
      }
      #endif
      .fileImporter(isPresented: shown(.files), allowedContentTypes: [.item], allowsMultipleSelection: true) { result in
        guard case .success(let urls) = result else { return }
        staged += urls.compactMap(AttachmentFile.copy)
      }
      #if canImport(UIKit)
      .fullScreenCover(isPresented: shown(.camera)) {
        CameraPicker { data in
          if let image = AttachmentImage.thumbnail(data) { staged.append(.image(blob: image, mimeType: "image/jpeg", name: nil)) }
        }
        .ignoresSafeArea()
      }
      #endif
      .onChange(of: source) { _, now in
        guard now == .camera, !Self.cameraAvailable else { return }
        source = nil
        actions.toast(t("mobile.camera.unavailable"), .camera)
      }
  }
}

/// Where an attachment comes from
enum AttachSource: Equatable { case photos, camera, files }

/// A picked file is readable only while the picker's grant lasts, so it is copied into the app's temporary folder at
/// once; the send reads the copy and hands its bytes to the engine (AppModel.drafts)
enum AttachmentFile {
  static var folder: URL { FileManager.default.temporaryDirectory.appendingPathComponent("attachments", isDirectory: true) }

  /// A picked file's copy, as opposed to a path on the computer
  static func isCopy(_ url: URL) -> Bool { url.isFileURL && url.standardizedFileURL.path.hasPrefix(folder.standardizedFileURL.path + "/") }

  static func copy(_ url: URL) -> Attachment? {
    let scoped = url.startAccessingSecurityScopedResource()
    defer { if scoped { url.stopAccessingSecurityScopedResource() } }
    let folder = folder.appendingPathComponent(UUID().uuidString, isDirectory: true)
    let copy = folder.appendingPathComponent(url.lastPathComponent)
    do {
      try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
      try FileManager.default.copyItem(at: url, to: copy)
    } catch {
      return nil
    }
    return .file(uri: copy.absoluteString, name: url.lastPathComponent)
  }
}

/// What a tap on an attached file opens, wherever its pill is (the composer's strip, the whole-screen editor, a sent
/// prompt): the phone's own copy of a picked file in Quick Look, a file on the computer in the chat's file page at its
/// line. Nil when neither applies: a copy already cleaned up, or a session that does not exist yet (the new-session
/// screen's draft has no project file reads)
struct FileOpener {
  var model: AppModel?
  var session: String?
  /// Runs before the file page is pushed: a screen covering the chat (the whole-screen editor) gets out of its way
  var leave: (() -> Void)? = nil
  var preview: (URL) -> Void

  func open(uri: String, line: Int? = nil) -> (() -> Void)? {
    guard let url = URL(string: uri), url.isFileURL else { return nil }
    if AttachmentFile.isCopy(url) {
      return FileManager.default.fileExists(atPath: url.path) ? { preview(url) } : nil
    }
    guard let model, let session, model.views[session] != nil else { return nil }
    return { [leave] in
      leave?()
      model.path.append(.file(session: session, path: url.path, line: line))
    }
  }
}

/// The attachments picked for the next prompt: image tiles and file pills, each with its ✕. A tile opens its picture in
/// the Lightbox and a pill its file (FileOpener), as a sent prompt's do
struct StagedAttachments: View {
  @Binding var items: [Attachment]
  /// Runs before a file opens in the chat's file page (FileOpener `leave`)
  var onNavigate: (() -> Void)? = nil
  @State private var preview: URL?
  @Environment(\.media) private var media
  @Environment(AppModel.self) private var model: AppModel?

  var body: some View {
    let opener = FileOpener(model: model, session: media.session, leave: onNavigate) { preview = $0 }
    ScrollView(.horizontal) {
      HStack(spacing: Metrics.s2) {
        // Quotes from the transcript lead as one annotations chip; its ✕ drops them all
        if items.contains(where: Self.quote) {
          QuoteChip(items: $items)
            .overlay(alignment: .topTrailing) {
              RemoveBadge(label: t("quote.removeAll")) { items.removeAll(where: Self.quote) }
            }
        }
        ForEach(Array(items.enumerated()), id: \.offset) { i, item in
          if !Self.quote(item) {
            switch item {
            case .image(let blob, _, let name):
              AttachmentThumbnail(blob: blob, name: name)
                .overlay(alignment: .topTrailing) {
                  RemoveBadge(label: t("common.remove")) { items.remove(at: i) }
                }
            // A pill is lower than the badge, which covered the end of its name: its ✕ sits inside it, after the name
            case .file(let uri, let name):
              AttachmentPill(name: name, icon: .fileText, onOpen: opener.open(uri: uri)) { items.remove(at: i) }
            case .text(_, let name, _):
              AttachmentPill(name: name, icon: .fileText) { items.remove(at: i) }
            // Only a sent prompt taken in for editing brings a selection back
            case .selection(let uri, let name, let start, let end, _):
              AttachmentPill(name: "\(name) \(start == end ? "L\(start)" : "L\(start)-\(end)")", icon: .textSelect,
                             onOpen: opener.open(uri: uri, line: start)) {
                items.remove(at: i)
              }
            default:
              EmptyView()
            }
          }
        }
      }
      .padding(.top, Metrics.s1_5)
      .padding(.trailing, Metrics.s1_5)
      .padding(.horizontal, Metrics.s1)
    }
    .scrollIndicators(.hidden)
    .quickLookPreview($preview)
  }

  private static func quote(_ item: Attachment) -> Bool { if case .quote = item { true } else { false } }
}

/// A staged chip's ✕, riding its top-right corner
struct RemoveBadge: View {
  let label: String
  var action: () -> Void
  var body: some View {
    Button(action: action) {
      Icon(.x, size: Metrics.chevron, strokeWidth: 2)
        .foregroundStyle(Palette.btn1Fg)
        .frame(width: Metrics.lead, height: Metrics.lead)
        .background(Circle().fill(Palette.btn1Bg))
    }
    .buttonStyle(.plain)
    .offset(x: Metrics.s1_5, y: -Metrics.s1_5)
    .accessibilityLabel(Text(web: label))
  }
}

/// Picked photos as inline JPEG, scaled down: the demo keeps them in memory, the engine will take blob names
enum AttachmentImage {
  /// The longest side a staged photo keeps
  static let side: CGFloat = 1024

  static func thumbnail(_ data: Data) -> String? {
    #if canImport(UIKit)
    guard let image = UIImage(data: data) else { return nil }
    let scale = min(1, side / max(image.size.width, image.size.height))
    let size = CGSize(width: image.size.width * scale, height: image.size.height * scale)
    let scaled = UIGraphicsImageRenderer(size: size).image { _ in image.draw(in: CGRect(origin: .zero, size: size)) }
    return scaled.jpegData(compressionQuality: 0.8)?.base64EncodedString()
    #else
    return nil
    #endif
  }

  #if canImport(UIKit)
  static func decode(_ blob: String) -> UIImage? { Data(base64Encoded: blob).flatMap(UIImage.init(data:)) }
  #endif
}

#if canImport(UIKit)
/// The system camera, handing back the photo's JPEG
struct CameraPicker: UIViewControllerRepresentable {
  var onPick: (Data) -> Void
  @Environment(\.dismiss) private var dismiss

  func makeUIViewController(context: Context) -> UIImagePickerController {
    let picker = UIImagePickerController()
    picker.sourceType = .camera
    picker.delegate = context.coordinator
    return picker
  }
  func updateUIViewController(_ controller: UIImagePickerController, context: Context) {}
  func makeCoordinator() -> Coordinator { Coordinator(self) }

  final class Coordinator: NSObject, UIImagePickerControllerDelegate, UINavigationControllerDelegate {
    let parent: CameraPicker
    init(_ parent: CameraPicker) { self.parent = parent }
    func imagePickerController(_ picker: UIImagePickerController, didFinishPickingMediaWithInfo info: [UIImagePickerController.InfoKey: Any]) {
      if let image = info[.originalImage] as? UIImage, let data = image.jpegData(compressionQuality: 0.9) { parent.onPick(data) }
      parent.dismiss()
    }
    func imagePickerControllerDidCancel(_ picker: UIImagePickerController) { parent.dismiss() }
  }
}
#endif

// MARK: - Project

/// The last component of a path, `~` for a bare home directory
func folderName(_ path: String) -> String {
  let parts = path.split(separator: "/")
  return parts.last.map(String.init) ?? path
}

// MARK: - Mode

/// modeIcons.ts: the glyph is inferred from keywords in id + name; first match wins
func modeIcon(_ mode: SessionOption) -> LucideIcon {
  let key = "\(mode.id) \(mode.name)".lowercased()
  let rules: [(String, LucideIcon)] = [
    (#"bypass|yolo|auto[\s-]?accept|full[\s-]?access|unsafe"#, .shieldOff),
    (#"debug"#, .bug),
    (#"plan"#, .listTodo),
    (#"\bask\b|chat|question"#, .messageCircleQuestionMark),
    (#"smart|\bauto\b"#, .sparkles),
    (#"code|edit|build"#, .code),
    (#"agent|default|normal"#, .infinity),
  ]
  return rules.first { key.range(of: $0.0, options: .regularExpression) != nil }?.1 ?? .circleDashed
}

/// modeIcons.ts `modeLabel`: the short name for a mode the built-in integrations are known to send, matched on the
/// exact id (never the display name, so a user's own agent keeps its name), in the app's language and one length in
/// CJK; anything unknown shows the agent's own name
func modeLabel(_ mode: SessionOption) -> String {
  let short: [String: String] = [
    "default": "composer.modeShort.default", "agent": "composer.modeShort.default", "build": "composer.modeShort.build",
    "plan": "composer.modeShort.plan", "raw": "composer.modeShort.raw", "acceptEdits": "composer.modeShort.autoEdit",
    "auto": "composer.modeShort.auto", "smart": "composer.modeShort.smart", "bypassPermissions": "composer.modeShort.full",
    "bypass": "composer.modeShort.full", "yolo": "composer.modeShort.full", "agent-full-access": "composer.modeShort.full",
    "read-only": "composer.modeShort.readOnly", "workspace-write": "composer.modeShort.workspace",
    "ask": "composer.modeShort.ask", "code": "composer.modeShort.code",
  ]
  guard let key = short[mode.id] ?? (mode.kind == "full_access" ? "composer.modeShort.full" : nil) else { return mode.name }
  return t(key)
}

// MARK: - Send

/// Send / stop in one: a solid circle set into the bar's end with a ring of the bar around it, as the system keyboard's
/// and Messages' send. Lit (the btn-1 fill, crisp) once there is something to send or a run to stop, a quiet chip fill
/// otherwise. Not glass: a white tint on Liquid Glass blooms into a soft glowing blob (tried and rejected), and the
/// bar's glass already frames it. The bar-tall slot keeps the bar's layout
struct SendButton: View {
  @Environment(\.motionEnabled) private var motion
  /// Syncing, stop rests unlit: the run it would stop may be over, and the engine's answer is a moment away
  @Environment(\.syncing) private var syncing
  var running: Bool
  var filled: Bool
  var action: () -> Void

  var body: some View {
    let on = (running && !syncing) || filled
    let side = Metrics.barCtl - Metrics.s1 * 2
    Button(action: action) {
      Group {
        if running && !filled {
          RoundedRectangle(cornerRadius: Metrics.s0_5).frame(width: Metrics.barIcon / 2, height: Metrics.barIcon / 2)
        } else {
          Icon(.arrowUp, size: Metrics.iconCtl, strokeWidth: 2)
        }
      }
      .foregroundStyle(on ? Palette.btn1Fg : Palette.sendInk)
      .frame(width: side, height: side)
      .background(Circle().fill(on ? Palette.btn1Bg : Palette.chip))
      .animation(motion ? Motion.ease : nil, value: on)
      .frame(width: Metrics.barCtl, height: Metrics.barCtl)
      .contentShape(Circle())
    }
    .buttonStyle(PressButtonStyle())
    .disabled(!on)
    .accessibilityLabel(Text(web: running ? t("composer.stop") : t("composer.send")))
  }
}

/// A bare button that gives under the finger: the webview's hover `brightness(1.08)` as a press, with a slight sink
struct PressButtonStyle: ButtonStyle {
  @Environment(\.motionEnabled) private var motion
  func makeBody(configuration: Configuration) -> some View {
    configuration.label
      .brightness(configuration.isPressed ? Motion.pressBrightness : 0)
      .scaleEffect(configuration.isPressed && motion ? Motion.pressScale : 1)
      .animation(motion ? Motion.press : nil, value: configuration.isPressed)
  }
}

/// A vertical text field as tall as its lines up to `cap`, then scrolling inside. The field fills any finite height it
/// is offered and reports its lines only when offered none, so it is measured unconstrained and placed at the capped
/// height; the chat's stack no longer squeezes it to an even share, and no frame stretches it past its text
struct FieldHeight: Layout {
  var cap: CGFloat

  func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
    guard let field = subviews.first else { return .zero }
    let lines = field.sizeThatFits(ProposedViewSize(width: proposal.width, height: nil))
    return CGSize(width: proposal.width ?? lines.width, height: min(lines.height, cap))
  }

  func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
    subviews.first?.place(at: bounds.origin, proposal: ProposedViewSize(bounds.size))
  }
}
