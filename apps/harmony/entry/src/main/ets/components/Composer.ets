import { Attachment, FileHit, SessionControls, SlashCommand, Usage } from '../wire/Wire';
import { Cmp, Metrics, Type } from '../design/Tokens';
import { draftBytes, MESSAGE_BYTES, PastedText } from '../logic/Composing';
import { ModelChoices } from '../logic/ModelCatalog';
import { MentionPersona, PromptEdit, SigilSpan, SlashCommands } from '../logic/SlashCommands';
import { AttachSource, ComposerDraft, FileSearch, Goo } from '../store/Composing';
import { pickFiles, pickPhotos, takePhoto } from '../store/AttachPickers';
import { t } from '../store/L10n';
import { AgentMark, Lucide } from './Glyphs';
import { ContextRing, FastGlyph, SendButton } from './composer/ComposerBits';
import { StagedStrip } from './composer/Staged';
import { CommandList, MentionList } from './composer/Suggestions';

// The phone composer (apps/ios Chat/Composer.swift, apps/android ui/Composer.kt). At rest one bar: `+` and the model's
// mark at its start, the field, the context ring and send at its end (dim with nothing to send, stop while a turn runs).
// Typing (focused, or a draft waiting) the bar grows into one card: the field across the whole width over a row of the
// chips and send, so the text is never squeezed between controls. The chips keep the bottom-left corner and send the
// bottom-right one, the card grows up around them, and the field stays the same view throughout, so focus holds. `+`
// and the mark are liquid chips whose beans the screen draws over its page (composer/GooLayer.ets).

const EMPTY_HIDDEN: Map<string, string[]> = new Map<string, string[]>();

@ComponentV2
export struct Composer {
  /** The chat's draft (AppModel.draftFor), or the new-session page's */
  @Param draft: ComposerDraft = new ComposerDraft();
  /** A hint of the screen's own; empty for the chat's (new / follow-up while idle, queue while a turn runs) */
  @Param placeholder: string = '';
  @Param running: boolean = false;
  @Param controls: SessionControls | undefined = undefined;
  /** The agent's hidden model families by control id, which thin the model chip's lists */
  @Param hidden: Map<string, string[]> = EMPTY_HIDDEN;
  @Param usage: Usage | undefined = undefined;
  @Param compactAt: number | undefined = undefined;
  @Param commands: SlashCommand[] = [];
  @Param personas: MentionPersona[] = [];
  /** The `@` file search; none hides files from the list */
  @Param files: FileSearch | undefined = undefined;
  /** The screen's liquid picker, whose beans the screen's GooLayer draws */
  @Param goo: Goo = new Goo();
  /** Screenshots (`-popup` on iOS): focus, context, quotes, expand */
  @Param popup: string = '';
  @Event onSend: (text: string, staged: Attachment[]) => void = () => {};
  @Event onStop: () => void = () => {};
  @Event onCompact: () => void = () => {};
  /** The card form, opened at once on a tap rather than waiting for focus to land */
  @Local card: boolean = false;
  /** The caret (and selection end) the `/` and `@` tokens are read at; -1 before the field reports one */
  @Local selStart: number = -1;
  @Local selEnd: number = -1;
  /** The field's laid-out height and the strip's, for the expand corner */
  @Local fieldHeight: number = 0;
  @Local stripHeight: number = 0;
  /** The draft is open in the whole-screen editor */
  @Local fullEditor: boolean = false;
  private controller: TextAreaController = new TextAreaController();
  private fullController: TextAreaController = new TextAreaController();
  private mentionTimer: number = -1;
  /** The text as the field last wrote it; any other change puts the caret at the end */
  private typed: string = '';

  aboutToAppear(): void {
    this.card = this.popup === 'focus' || this.popup === 'edit';
    this.fullEditor = this.popup === 'expand';
  }

  private d(): ComposerDraft {
    return this.draft;
  }

  private filled(): boolean {
    return this.d().text.trim().length > 0 || this.d().staged.length > 0;
  }

  private expanded(): boolean {
    return this.card || this.filled();
  }

  /** The hint as drawn: its first sentence, cut before the `@` tip, on one line */
  private hint(): string {
    const hint = this.placeholder.length > 0 ? this.placeholder :
      this.running ? t('composer.placeholder.queue') : t('composer.placeholder');
    return (hint.split('@')[0] ?? hint).trim();
  }

  private choices(): ModelChoices {
    return new ModelChoices(this.controls, this.hidden, () => {});
  }

  private showsModel(): boolean {
    return this.choices().any;
  }

  @Monitor('draft.focusKey')
  onFocusKey(): void {
    this.focusField();
  }

  @Monitor('draft.text')
  onDraftText(): void {
    // A restored draft or a taken-back queue item: the caret goes to the end
    if (this.d().text !== this.typed) {
      this.typed = this.d().text;
      this.selStart = -1;
      this.selEnd = -1;
    }
  }

  @Monitor('goo.source')
  onSource(): void {
    const goo = this.goo;
    const source = goo.source;
    if (source === undefined) {
      return;
    }
    goo.source = undefined;
    const context = this.getUIContext().getHostContext() as Context;
    const picked: Promise<Attachment[]> = source === AttachSource.Photos ? pickPhotos() :
      source === AttachSource.Camera ? takePhoto(context) : pickFiles(context);
    picked.then((more: Attachment[]) => this.stage(more)).catch(() => {});
  }

  /** Picked attachments join the strip while the draft still fits one message */
  private stage(more: Attachment[]): void {
    if (more.length === 0) {
      return;
    }
    const next = this.d().staged.concat(more);
    if (draftBytes(this.d().text, next) > MESSAGE_BYTES) {
      this.toast(t('mobile.video.tooBig', new Map<string, string>([['mb', `${Math.floor(MESSAGE_BYTES / 4 * 3 / 1048576)}`]])));
      return;
    }
    this.d().staged = next;
  }

  private toast(message: string): void {
    try {
      this.getUIContext().getPromptAction().showToast({ message: message });
    } catch (e) {
      // No toast host (the Previewer)
    }
  }

  /** The bar's tap and every other way in: the card opens at once and the field asks for the keyboard */
  private focusField(): void {
    this.card = true;
    setTimeout(() => {
      focusControl.requestFocus('composer.field');
    }, 0);
  }

  private send(): void {
    const d = this.d();
    if (this.running && !this.filled()) {
      this.onStop();
      return;
    }
    if (draftBytes(d.text, d.staged) > MESSAGE_BYTES) {
      this.toast(t('mobile.video.tooBig', new Map<string, string>([['mb', `${Math.floor(MESSAGE_BYTES / 4 * 3 / 1048576)}`]])));
      return;
    }
    const text = d.text;
    const staged = d.staged;
    // Cleared before the send, which may put an edit's kept draft back
    this.typed = '';
    d.text = '';
    d.staged = [];
    this.onSend(text, staged);
  }

  // Commands and mentions

  private caret(): number {
    const len = this.d().text.length;
    return this.selStart < 0 || this.selEnd > len ? len : this.selEnd;
  }

  private collapsed(): boolean {
    return this.selStart < 0 || this.selStart === this.selEnd || this.selEnd > this.d().text.length;
  }

  private slashSpan(): SigilSpan | undefined {
    return this.collapsed() ? SlashCommands.commandAt(this.d().text, this.caret()) : undefined;
  }

  private mentionSpan(): SigilSpan | undefined {
    if (!this.collapsed() || (this.files === undefined && this.personas.length === 0)) {
      return undefined;
    }
    return SlashCommands.mentionAt(this.d().text, this.caret());
  }

  private slashHits(): SlashCommand[] {
    const span = this.slashSpan();
    return span !== undefined ? SlashCommands.match(this.commands, span.query) : [];
  }

  private mentionHits(): FileHit[] {
    const span = this.mentionSpan();
    if (span === undefined) {
      return [];
    }
    return SlashCommands.personaHits(this.personas, span.query).concat(this.files?.hits ?? []);
  }

  private showsMentions(): boolean {
    const span = this.mentionSpan();
    return span !== undefined &&
      (this.mentionHits().length > 0 || (this.files?.answered === span.query && span.query.length > 0));
  }

  /** The input hint of the command the text names, while the list is not showing it */
  private commandHint(): string | undefined {
    return this.slashHits().length === 0 ? SlashCommands.hint(this.commands, this.d().text) : undefined;
  }

  /** Mention.tsx useMentionHits: the query goes out a moment after the last keystroke; the list closing drops it */
  private refreshMentions(): void {
    const files = this.files;
    if (files === undefined) {
      return;
    }
    clearTimeout(this.mentionTimer);
    const span = this.mentionSpan();
    if (span === undefined) {
      files.close();
      return;
    }
    const query = span.query;
    this.mentionTimer = setTimeout(() => files.search(query), Cmp.mentionDebounceMs);
  }

  private pickCommand(command: SlashCommand): void {
    const span = this.slashSpan();
    if (span !== undefined) {
      this.apply(SlashCommands.complete(this.d().text, span, this.caret(), command.name));
    }
  }

  /** A persona is summoned by name; a file leaves the text and rides along as an attachment */
  private pickMention(hit: FileHit): void {
    const span = this.mentionSpan();
    if (span === undefined) {
      return;
    }
    const at = this.caret();
    const persona = SlashCommands.personaOf(hit, this.personas);
    if (persona !== undefined) {
      this.apply(SlashCommands.summon(this.d().text, persona.name, span.start, at - span.start));
      return;
    }
    this.apply(SlashCommands.dropToken(this.d().text, span, at));
    if (!this.d().staged.some((a: Attachment) => a.kind === 'file' && a.uri === hit.uri)) {
      const file: Attachment = { kind: 'file', uri: hit.uri, name: hit.path };
      this.d().staged = this.d().staged.concat([file]);
    }
  }

  /** SummonMenu: `@name ` at the caret */
  summon(persona: MentionPersona): void {
    this.apply(SlashCommands.summon(this.d().text, persona.name, this.caret(), 0));
    this.focusField();
  }

  /** The edited prompt, the caret after what was written */
  private apply(edit: PromptEdit): void {
    this.typed = edit.text;
    this.d().text = edit.text;
    const at = Math.min(edit.caret, edit.text.length);
    this.selStart = at;
    this.selEnd = at;
    setTimeout(() => this.controller.caretPosition(at), 0);
    this.files?.close();
  }

  /** A long paste becomes a text attachment before it reaches the field */
  private paste(value: string, event: PasteEvent): void {
    const kind = PastedText.classify(value);
    if (kind === 'inline') {
      return;
    }
    if (event.preventDefault !== undefined) {
      event.preventDefault();
    }
    const name = t('attach.pastedText');
    if (kind === 'attachment') {
      this.d().staged = this.d().staged.concat([PastedText.attachment(value, name)]);
    } else {
      this.toast(t('attach.tooBigFile', new Map<string, string>([['name', name], ['mb', `${PastedText.maxBytes / 1048576}`]])));
    }
  }

  build() {
    Column({ space: Cmp.s1_5 }) {
      // The `/` list, else the `@` list
      if (this.slashHits().length > 0) {
        CommandList({ items: this.slashHits(), pick: (c: SlashCommand) => this.pickCommand(c) })
      } else if (this.showsMentions()) {
        MentionList({
          items: this.mentionHits(), personas: this.personas, empty: t('mention.noFiles'),
          pick: (hit: FileHit) => this.pickMention(hit)
        })
      }
      Stack({ alignContent: Alignment.BottomStart }) {
        Column() {
          // Staged attachments ride inside the card over the text; anything staged keeps the card open
          if (this.d().staged.length > 0) {
            StagedStrip({ draft: this.d(), popup: this.popup })
              .padding({ left: Cmp.s3 + Metrics.s1, top: Metrics.s1 })
              .onAreaChange((_: Area, now: Area) => {
                this.stripHeight = now.height as number;
              })
          }
          this.field()
        }
        .width('100%')
        // While a picker is out the card's text and strip recede with the page
        .opacity(this.goo.open !== undefined ? Cmp.gooRecede : 1)
        .blur(this.goo.open !== undefined && this.expanded() ? Metrics.s1 : 0)
        .animation({ duration: Cmp.quickMs })
        // The chips' row at the start, inset from the card's corner as send is from its own
        Row() {
          this.attachChip()
          if (this.showsModel()) {
            this.modelChip()
          }
        }
        .margin({ left: this.expanded() ? Metrics.s1 : 0, bottom: this.expanded() ? Metrics.s1 : 0 })
        .onAreaChange((_: Area, now: Area) => {
          this.goo.attachLeft = now.globalPosition.x as number;
          this.goo.modelLeft = (now.globalPosition.x as number) + Cmp.bar;
          this.goo.chipTop = now.globalPosition.y as number;
        })
        // Ring and send at the end
        Row() {
          if (this.usage !== undefined && this.usage.size > 0) {
            ContextRing({
              usage: this.usage, canCompact: this.commands.some((c: SlashCommand) => c.name === 'compact'),
              compactAt: this.compactAt, running: this.running, startOpen: this.popup === 'context',
              onCompact: () => this.onCompact()
            })
          }
          SendButton({ running: this.running, filled: this.filled(), action: () => this.send() })
        }
        .position({ right: this.expanded() ? Metrics.s1 : 0, bottom: this.expanded() ? Metrics.s1 : 0 })
        .opacity(this.goo.open !== undefined ? Cmp.gooRecede : 1)
        // The expand corner at the card's top trailing side, on the first line under the strip
        if (this.offersExpand()) {
          Stack() {
            Lucide({ name: 'maximize2', tint: $r('app.color.cmp_fg2'), extent: Metrics.icon })
          }
          .id('composer.expand')
          .width(Cmp.s3 * 2 + Metrics.icon)
          .height(Cmp.fieldLine)
          .position({ right: 0, top: (this.d().staged.length > 0 ? this.stripHeight : 0) + this.fieldTop() })
          .opacity(this.goo.open !== undefined ? Cmp.gooRecede : 1)
          .accessibilityText(t('mobile.composer.expand'))
          .transition(TransitionEffect.OPACITY)
          .onClick(() => {
            this.fullEditor = true;
          })
        }
      }
      .width('100%')
      .backgroundColor($r('app.color.bg1'))
      .borderRadius(this.expanded() ? Metrics.rComposer : Cmp.bar / 2)
      .border({ width: Metrics.hairline, color: $r('app.color.line') })
      .shadow({ radius: Cmp.shadow, color: $r('app.color.line'), offsetY: Cmp.s0_5 })
      .animation({ curve: Curve.Friction, duration: Cmp.quickMs * 2 })
      .onClick(() => this.focusField())
    }
    .constraintSize({ maxWidth: Metrics.contentW })
    .width('100%')
    .bindContentCover(this.fullEditor, this.expandedEditor(), {
      modalTransition: ModalTransition.DEFAULT,
      backgroundColor: $r('app.color.bg0'),
      onDisappear: () => {
        this.fullEditor = false;
      }
    })
  }

  /** The field's top inset: the text centred in the bar, or below the staged strip */
  private fieldTop(): number {
    return this.d().staged.length > 0 ? Metrics.s2 : (Cmp.bar - Cmp.fieldLine) / 2;
  }

  private offersExpand(): boolean {
    return this.expanded() && this.fieldHeight >= Cmp.fieldLine * Cmp.expandLines - Metrics.hairline;
  }

  private padStart(): number {
    if (this.expanded()) {
      return Cmp.s3 + Metrics.s1 * 2;
    }
    return (this.showsModel() ? Cmp.bar * 2 : Cmp.bar) + Metrics.s1;
  }

  private padEnd(): number {
    if (this.expanded()) {
      return Cmp.s3 + Metrics.icon + Metrics.s1;
    }
    return Cmp.bar + Metrics.s1 + (this.usage !== undefined && this.usage.size > 0 ? Cmp.ctlSm : 0);
  }

  /** The field with its hint drawn over it in both forms, and the command's input hint under it */
  @Builder
  field() {
    Column() {
      Stack({ alignContent: Alignment.TopStart }) {
        TextArea({ text: this.d().text, controller: this.controller, placeholder: '' })
          .id('composer.field')
          .fontSize(Cmp.field)
          .lineHeight(Cmp.fieldLine)
          .fontColor($r('app.color.fg_strong'))
          .caretColor($r('app.color.fg_strong'))
          .backgroundColor(Color.Transparent)
          .borderRadius(0)
          .padding(0)
          .enterKeyType(EnterKeyType.NEW_LINE)
          .maxLines(this.expanded() ? Cmp.maxLines : 1)
          .constraintSize({ minHeight: Cmp.fieldLine, maxHeight: Cmp.fieldLine * (this.expanded() ? Cmp.maxLines : 1) })
          // At rest the field takes no touches: the bar's tap focuses it
          .hitTestBehavior(this.expanded() ? HitTestMode.Default : HitTestMode.Transparent)
          .width('100%')
          .onChange((value: string) => {
            this.typed = value;
            this.d().text = value;
            this.refreshMentions();
          })
          .onTextSelectionChange((start: number, end: number) => {
            this.selStart = start;
            this.selEnd = end;
            this.refreshMentions();
          })
          .onPaste((value: string, event: PasteEvent) => this.paste(value, event))
          .onFocus(() => {
            this.card = true;
          })
          .onBlur(() => {
            this.card = false;
          })
          .onAreaChange((_: Area, now: Area) => {
            this.fieldHeight = now.height as number;
          })
        if (this.d().text.length === 0) {
          Text(this.hint())
            .fontSize(Cmp.field)
            .lineHeight(Cmp.fieldLine)
            .fontWeight(FontWeight.Medium)
            .fontColor($r('app.color.bar_hint'))
            .maxLines(1)
            .textOverflow({ overflow: TextOverflow.Ellipsis })
            .hitTestBehavior(HitTestMode.None)
            .width('100%')
        }
      }
      .width('100%')
      if (this.commandHint() !== undefined) {
        Text(this.commandHint())
          .fontSize(Type.mono)
          .fontFamily('monospace')
          .fontColor($r('app.color.fg3'))
          .maxLines(1)
      }
    }
    .alignItems(HorizontalAlign.Start)
    .padding({
      left: this.padStart(), right: this.padEnd(), top: this.fieldTop(),
      bottom: this.expanded() ? Metrics.s1 + Cmp.bar + Metrics.s1 : (Cmp.bar - Cmp.fieldLine) / 2
    })
    .width('100%')
  }

  /** `+`, turned × while its beans are out, ‹ over a panel */
  @Builder
  attachChip() {
    Stack() {
      if (this.goo.open === 'attach' && this.goo.stage > 0) {
        Lucide({ name: 'chevronLeft', tint: $r('app.color.fg1'), extent: Cmp.barIcon })
      } else {
        Lucide({ name: 'plus', tint: $r('app.color.fg1'), extent: Cmp.barIcon })
          .rotate({ angle: this.goo.open === 'attach' ? Cmp.plusTurn : 0 })
          .animation({ duration: Cmp.quickMs })
      }
    }
    .id('composer.attach')
    .width(Cmp.bar)
    .height(Cmp.bar)
    .accessibilityText(t('mobile.attach'))
    .onClick(() => this.goo.toggle('attach'))
  }

  /** The model's mark alone, Fast as a bolt on its shoulder; × over the beans, ‹ over a panel or the sidekick's level */
  @Builder
  modelChip() {
    Stack() {
      if (this.goo.open === 'model') {
        Lucide({
          name: this.goo.stage > 0 || this.goo.deeper !== undefined ? 'chevronLeft' : 'x',
          tint: $r('app.color.fg1'), extent: Cmp.gooBack
        })
      } else {
        Stack({ alignContent: Alignment.TopEnd }) {
          if (this.choices().cur !== undefined) {
            AgentMark({
              agent: this.choices().cur?.brand ?? '', name: this.choices().cur?.name ?? '', tint: $r('app.color.fg1'),
              extent: Cmp.barIcon
            })
          } else {
            Lucide({ name: 'slidersHorizontal', tint: $r('app.color.fg1'), extent: Cmp.barIcon })
          }
          if (this.choices().fast?.on === true) {
            FastGlyph({ on: true, extent: Cmp.chevron })
              .translate({ x: Metrics.s1, y: -Metrics.s1 })
          }
        }
        .width(Cmp.barIcon)
        .height(Cmp.barIcon)
      }
    }
    .id('composer.model')
    .width(Cmp.bar)
    .height(Cmp.bar)
    .accessibilityText(this.choices().cur?.name ?? t('stats.model'))
    .onClick(() => this.goo.toggle('model'))
  }

  /**
   * The whole-screen editor (ExpandedComposer.swift): the same draft over the whole page, its attachments over the
   * text, collapse in the bar and send at the corner it holds in the card
   */
  @Builder
  expandedEditor() {
    Column({ space: Metrics.s2 }) {
      Row() {
        Blank()
        Stack() {
          Lucide({ name: 'minimize2', tint: $r('app.color.fg1'), extent: Metrics.iconCtl })
        }
        .width(Metrics.ctl)
        .height(Metrics.ctl)
        .accessibilityText(t('mobile.composer.collapse'))
        .onClick(() => {
          this.fullEditor = false;
          this.focusField();
        })
      }
      .width('100%')
      .padding({ left: Metrics.pad, right: Metrics.s1 })
      if (this.d().staged.length > 0) {
        StagedStrip({ draft: this.d() })
          .padding({ left: Metrics.pad, right: Metrics.pad })
      }
      TextArea({ text: this.d().text, controller: this.fullController })
        .fontSize(Cmp.field)
        .lineHeight(Cmp.fieldLine)
        .fontColor($r('app.color.fg_strong'))
        .caretColor($r('app.color.fg_strong'))
        .backgroundColor(Color.Transparent)
        .borderRadius(0)
        .padding({ left: Metrics.pad, right: Metrics.pad })
        .defaultFocus(true)
        .layoutWeight(1)
        .onChange((value: string) => {
          this.typed = value;
          this.d().text = value;
        })
        .onPaste((value: string, event: PasteEvent) => this.paste(value, event))
      Row() {
        Blank()
        SendButton({ running: false, filled: this.filled(), action: () => {
          this.fullEditor = false;
          this.send();
        } })
      }
      .width('100%')
      .padding({ left: Metrics.pad, right: Metrics.pad, top: Metrics.s2, bottom: Metrics.s2 })
    }
    .width('100%')
    .height('100%')
    .backgroundColor($r('app.color.bg0'))
    .expandSafeArea([SafeAreaType.SYSTEM], [SafeAreaEdge.BOTTOM])
  }
}
