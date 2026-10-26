import { memo, useCallback, useMemo, useRef, useState, type ClipboardEvent, type DragEvent, type KeyboardEvent, type ReactNode } from 'react';
import { Plus, SquareDashedMousePointer, TextSelect, X } from 'lucide-react';
import type { Draft, SessionControls, SessionOption, SlashCommand, Turn, Usage } from '@acpira/shared/transcript';
import type { EditorSelection, FileHit } from '@acpira/shared/protocol';
import type { HiddenMap } from '@acpira/shared/settings';
import { composerControls } from '@acpira/shared/composerControls';
import { MAX_TEXT_BYTES, lineRangeLabel } from '@acpira/shared/attachments';
import { collectPastedText } from '@acpira/shared/pastedText';
import { useAppearance } from '../appearance';
import { getLocale, t, useLocale } from '../i18n';
import { cn } from '../ui/cn';
import { Chip, IconButton } from '../ui/Button';
import { DropdownMenu, optionClass } from '../ui/DropdownMenu';
import { OptionContent } from '../ui/Panel';
import { WorkingBeam } from '../effects/WorkingBeam';
import { SendButton } from '../effects/SendButton';
import { DraftChips } from './Attachments';
import { collectDrafts, collectImageSources, hasPayload, hiddenText, imageSources, pastedText, pathlessWorkbenchDrag, typeInto, type Collected } from './drafts';
import { hostDragPaths } from '../vscodeApi';
import { MentionList, mentionAt, personaHits, personaOfHit, useMentionHits, type MentionPersona } from './Mention';
import { SummonMenu } from './SummonMenu';
import { SlashList, commandAt, commandHint, completeCommand, promptMarks, useSlashHits } from './Slash';
import { modeIcon, modeLabel } from './modeIcons';
import { ModelControl, OptionControl, ReasoningControl } from './ModelPicker';
import { ContextRing } from './ContextUsage';
import { useComposerDraft } from './useComposerDraft';
import { contextDrafts, copiedSelection, markContextSent, markSelectionSent, sameContext, sameRange, selectionDraft, selectionKey, useEditorSelection, useLiveContexts, type LiveContext } from './editorContext';
import { PromptInput } from './PromptInput';
import { imeKey } from '../ui/ime';

export interface ComposerProps {
  running: boolean;
  // Text + send locked for login / error / read-only. Starting still accepts a first prompt into the queue
  disabled?: boolean;
  // Mode / model chips stay inert until session/new has real controls
  controlsLocked?: boolean;
  theme: 'dark' | 'light';
  controls: SessionControls;
  turns?: Turn[];
  turnCount?: number;
  compacting?: boolean;
  // Option families hidden for the current agent (settings page): configOption id → family names
  hidden?: HiddenMap[string];
  usage?: Usage;
  // Slash commands the agent advertised for this session: the / completion list, and `compact` among them enables the context panel's button
  commands?: SlashCommand[];
  // Auto-compact threshold: the ring fills against this budget when it is smaller than the agent's window
  compactAt?: number;
  // Workspace root: dropped and mentioned files are labeled relative to it
  cwd: string;
  onSend: (text: string, attachments: Draft[]) => void | Promise<void>;
  // A send while running steers into the turn instead of queueing (only the placeholder changes here)
  steer?: boolean;
  // A plan card waits on the user: a send revises that plan (only the placeholder changes here)
  revisePlan?: boolean;
  // Inline history editors keep their draft until the host accepts the resend.
  // Outside-dismiss editors omit the cancel button; `attachments` are bare chip items (with their own removal controls)
  // that open the draft row, `hasAttachments` says whether any are still retained.
  edit?: { text: string; attachments?: ReactNode; hasAttachments: boolean; onCancel: () => void; dismissOnOutside?: boolean };
  // Where the unsent draft (text + attachments) is parked while another session is shown; the session id. Absent: nothing is kept across remounts
  draftKey?: string;
  onSearchFiles: (query: string) => Promise<FileHit[]>;
  // Something couldn't be attached; the shell shows it as a toast
  onNotice: (text: string) => void;
  onStop: () => void;
  onSetMode: (id: string) => void;
  onSetConfig: (configId: string, value: string) => void;
  onCompact: () => void;
  // Session-level navigation is omitted from history and queued-message editors.
  toolbarStart?: ReactNode;
  // The agent takes its models from the user's own sources (the built-in agent): with none offered, the model chip's
  // place holds a shortcut to the page where they are added
  onAddModel?: () => void;
  // The session's own composer (not an inline editor): it takes drafts pushed from outside (editor "Add to chat", transcript quotes)
  // and, with `shareSelection`, offers the editor's live selection as a toolbar chip that goes out with the next prompt
  main?: boolean;
  shareSelection?: boolean;
  // Cross-harness subagents the user defined: they lead the @ list and the toolbar's summon menu
  personas?: MentionPersona[];
}

// Composer has three layers: attachment chips (when any), the input area, and a toolbar row below.
// One layout for all ACP agents: mode on the left; context, model with reasoning, and send on the right.
// Attachments come from pasting / dropping (images, OS files, Explorer items) or from an @ mention that searches the workspace
export const Composer = memo(function Composer(p: ComposerProps) {
  useLocale();
  const { composer } = useAppearance();
  const textarea = useRef<HTMLTextAreaElement>(null);
  const focusInput = useCallback(() => textarea.current?.focus(), []);
  const { text, setText, drafts, setDrafts } = useComposerDraft(p.draftKey, p.edit?.text, p.main ? { focus: focusInput } : undefined);
  // The editor's live selection: offered as a chip beside the mode, clicked away to leave it out of this prompt
  const live = useEditorSelection();
  const offered = p.main && p.shareSelection && !p.edit ? live : undefined;
  const [excluded, setExcluded] = useState<string>();
  const offeredOn = !!offered && selectionKey(offered) !== excluded;
  // Other tools' live selections, one chip per source beside the editor's; clicking one leaves its current content out
  const liveContexts = useLiveContexts();
  const contexts = p.main && p.shareSelection && !p.edit ? liveContexts : NO_CONTEXTS;
  const [excludedContexts, setExcludedContexts] = useState<ReadonlySet<string>>(() => new Set());
  // Keyed by source as well: two sources offering the same content are still switched apart
  const excludedKey = (c: LiveContext) => `${c.source.id}\n${c.key}`;
  const contextOn = (c: LiveContext) => !excludedContexts.has(excludedKey(c));
  // Two or more live chips fold to their icons on anything narrower than a wide editor tab, so the model name keeps its room
  const liveNarrow = (offered ? 1 : 0) + Math.min(contexts.length, MAX_CONTEXT_CHIPS + 1) > 1 ? 'icon-xl' : 'icon';
  const toggleContext = (c: LiveContext) => setExcludedContexts(cur => {
    const next = new Set(cur);
    if (!next.delete(excludedKey(c))) next.add(excludedKey(c));
    return next;
  });
  const flush = !p.edit && composer === 'flush';
  // The beam lights up while the composer is focused (focus-within semantics), not while it's working.
  // Overlays portal to the shell root, so opening a menu blurs the composer; "a menu is open" therefore also counts as focused
  const [focused, setFocused] = useState(false);
  const [openMenus, setOpenMenus] = useState(0);
  const onOpenChange = useCallback((open: boolean) => setOpenMenus(n => n + (open ? 1 : -1)), []);
  const beamActive = focused || openMenus > 0;
  const dim = p.running || p.disabled;
  const { models, reasoning, modelConfig, collaboration, permission, sidekick, sidekickEffort, other } = composerControls(p.controls.options);
  const canCompact = !!p.commands?.some(c => c.name === 'compact');
  // Files are read asynchronously after a paste / drop; sending is held until every read has landed, so a message never leaves without its attachments
  const [reading, setReading] = useState(0);
  const [sending, setSending] = useState(false);
  const hasContent = text.trim().length > 0 || drafts.length > 0 || !!p.edit?.hasAttachments;
  const canSend = !p.disabled && !sending && reading === 0 && hasContent;
  // A follow-up uses the same send path as Enter; pending attachment reads / sends keep it from becoming a stop action.
  const showStop = p.running && !hasContent && reading === 0 && !sending;
  const sendingRef = useRef(false);
  const send = async () => {
    if (!canSend || sendingRef.current) return;
    sendingRef.current = true;
    setSending(true);
    try {
      const sel = offeredOn ? selectionDraft(offered, p.cwd) : undefined;
      const included = contexts.filter(contextOn);
      const offeredDrafts = [...(sel ? [sel] : []), ...included.flatMap(c => contextDrafts(c.source, c.items))]
        .filter(l => !drafts.some(d => sameRange(d, l) || sameContext(d, l)));
      await p.onSend(text, [...drafts, ...offeredDrafts]);
      if (offeredOn) markSelectionSent(offered);
      for (const c of included) markContextSent(c);
      setText('');
      setDrafts([]);
      setDismissed(undefined);
      setSlashDismissed(undefined);
    } catch (e) {
      p.onNotice(e instanceof Error ? e.message : String(e));
    } finally {
      sendingRef.current = false;
      setSending(false);
    }
  };

  // Drop zone: the whole field lights up while something attachable hovers over it. Enter / leave are counted rather than trusting relatedTarget,
  // which is null for drags coming from outside the window
  const dragDepth = useRef(0);
  const [dragging, setDragging] = useState(false);
  const fieldRef = useRef<HTMLDivElement>(null);
  // Takes the read already started: a DataTransfer's files and data are only readable during its event
  const addFrom = async (collecting: Promise<Collected>) => {
    setReading(n => n + 1);
    try {
      const { drafts: more, refused } = await collecting;
      if (more.length) setDrafts(d => [...d, ...more.filter(m => m.kind !== 'file' || !!m.data || !d.some(x => x.kind === 'file' && x.uri === m.uri))]);
      if (refused.length) p.onNotice(refused.join(t('common.listSep')));
    } catch (e) {
      p.onNotice(t('composer.attachFailed', { error: e instanceof Error ? e.message : String(e) }));
    } finally {
      setReading(n => n - 1);
    }
  };
  const takes = (dt: DataTransfer | null) => !p.disabled && hasPayload(dt);
  const onDragEnter = (e: DragEvent) => { if (!takes(e.dataTransfer)) return; e.preventDefault(); dragDepth.current++; setDragging(true); };
  // A workbench drag without any readable path is still accepted (no highlight), so its drop can report what arrived instead of vanishing
  const pathless = (dt: DataTransfer | null) => (p.disabled ? undefined : pathlessWorkbenchDrag(dt));
  const onDragOver = (e: DragEvent) => { if (takes(e.dataTransfer) || pathless(e.dataTransfer)) e.preventDefault(); };
  const onDragLeave = (e: DragEvent) => { if (!takes(e.dataTransfer)) return; if (--dragDepth.current <= 0) { dragDepth.current = 0; setDragging(false); } };
  // Plain text drags are left to the textarea's native handling; only attachable payloads are taken over
  const onDrop = (e: DragEvent) => {
    dragDepth.current = 0;
    setDragging(false);
    const types = pathless(e.dataTransfer);
    if (types) {
      e.preventDefault();
      p.onNotice(t('attach.noPaths', { types: types.join(', ') || '—' }));
      return;
    }
    if (!takes(e.dataTransfer)) return;
    e.preventDefault();
    void addFrom(collectDrafts(e.dataTransfer, p.cwd, hostDragPaths()?.()));
  };
  // A paste can carry attachments and text together (a chat message copied with its screenshot, a web selection with images): both are kept.
  // Real files win over the <img> copies in the HTML flavour. Large text bypasses textarea layout entirely.
  const onPaste = (e: ClipboardEvent) => {
    if (p.disabled) return;
    const dt = e.clipboardData;
    const files = hasPayload(dt) && dt.files.length > 0;
    const sources = files ? [] : imageSources(dt);
    if (files) void addFrom(collectDrafts(dt, p.cwd));
    else if (sources.length) void addFrom(collectImageSources(sources));
    const pasted = pastedText(dt);
    // Text that is not the event's text/plain (taken from the HTML flavour) is typed here, the textarea would insert nothing
    const insert = pasted !== dt.getData('text/plain');
    if (!pasted.trim()) {
      // Nothing to type (or only the copied files' names): keep the textarea from inserting anything
      if (files || sources.length || insert) e.preventDefault();
      // Text the event withheld beside the files is read back and typed once it arrives
      void hiddenText(dt)?.then(text => { if (text.trim()) typeText(text, true); });
      return;
    }
    if (insert) e.preventDefault();
    if (typeText(pasted, insert)) e.preventDefault();
  };
  // Pasted text: lines copied from an editor come back as the range they were copied from (like Cursor's code references), large text
  // becomes an attachment, the rest is typed at the caret when `insert` is set or left to the textarea. True when the paste was taken
  const typeText = (pasted: string, insert: boolean): boolean => {
    const copied = copiedSelection(pasted);
    if (copied) {
      const sel = selectionDraft(copied, p.cwd);
      setDrafts(current => (current.some(d => sameRange(d, sel)) ? current : [...current, sel]));
      return true;
    }
    const name = t('attach.pastedText');
    const { draft, tooBig } = collectPastedText(pasted, name);
    if (tooBig) p.onNotice(t('attach.tooBigText', { name, kb: MAX_TEXT_BYTES >> 10 }));
    if (draft) setDrafts(current => [...current, draft]);
    if (draft || tooBig) return true;
    if (insert) typeInto(textarea.current, pasted);
    return insert;
  };

  // @ mention: the span under the caret drives the file list (only with a collapsed selection); Esc parks it for that @ until the caret leaves or the message is sent
  const [caret, setCaret] = useState(0);
  const [collapsed, setCollapsed] = useState(true);
  const [dismissed, setDismissed] = useState<number>();
  const span = collapsed ? mentionAt(text, caret) : undefined;
  const mentionOpen = !!span && span.start !== dismissed && !p.disabled;
  const personas = p.personas;
  const search = useCallback(
    async (q: string) => [...(personas ? personaHits(personas, q) : []), ...(await p.onSearchFiles(q))],
    [personas, p.onSearchFiles],
  );
  const { hits, ready, active, setActive, move } = useMentionHits(mentionOpen ? span.query : undefined, search);
  // `@name ` at `at` (replacing `cut` characters there): a summon is text the agent reads, not an attachment
  const summon = (name: string, at: number, cut = 0) => {
    const before = text.slice(0, at);
    const lead = before && !/\s$/.test(before) ? ' ' : '';
    const token = `${lead}@${name} `;
    const next = before + token + text.slice(at + cut);
    setText(next);
    const end = at + token.length;
    setCaret(end);
    requestAnimationFrame(() => { textarea.current?.focus(); textarea.current?.setSelectionRange(end, end); });
  };
  const pick = (hit: FileHit) => {
    if (!span) return;
    const persona = personaOfHit(hit, personas);
    if (persona) { summon(persona.name, span.start, caret - span.start); return; }
    const next = text.slice(0, span.start) + text.slice(caret);
    setText(next);
    setCaret(span.start);
    setDrafts(d => (d.some(x => x.kind === 'file' && x.uri === hit.uri) ? d : [...d, { kind: 'file', uri: hit.uri, name: hit.path }]));
    requestAnimationFrame(() => textarea.current?.setSelectionRange(span.start, span.start));
  };
  // / command: a slash at the start or after whitespace with the caret in its token lists the commands the agent advertised for
  // this session, so a skill can also be called up mid-sentence like an @ mention. Picking completes the token in place (`/name `)
  // — the command goes out through the ordinary send path and the agent runs it; the list is discovery, never a whitelist, so with
  // no match the slash stays plain text. The two lists stay mutually exclusive: the token under the caret starts with @ or /, never both
  const [slashDismissed, setSlashDismissed] = useState<number>();
  const slashSpan = collapsed && !p.disabled ? commandAt(text, caret) : undefined;
  const slashLive = slashSpan && slashSpan.start !== slashDismissed ? slashSpan : undefined;
  const slash = useSlashHits(p.commands, slashLive?.query);
  const slashOpen = !!slashLive && slash.hits.length > 0;
  const pickCommand = (c: SlashCommand) => {
    if (!slashSpan) return;
    const next = completeCommand(text, slashSpan, caret, c.name);
    setText(next.text);
    setCaret(next.caret);
    requestAnimationFrame(() => textarea.current?.setSelectionRange(next.caret, next.caret));
  };
  // The input hint of the command the text names, while its arguments are still empty (the open list already shows it in the row)
  const hint = !slashOpen && p.commands ? commandHint(p.commands, text, getLocale()) : undefined;
  // Every advertised `/name` token paints the command mark — leading or mid-sentence alike (Cursor / Codex do the same) —
  // and every `@name` of an enabled persona the summon mark
  const summons = useMemo(() => personas?.map(x => x.name) ?? [], [personas]);
  const marks = promptMarks(p.commands ?? [], summons, text);
  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (imeKey(e.nativeEvent)) return;
    if (slashOpen) {
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') { e.preventDefault(); slash.move(e.key === 'ArrowDown' ? 1 : -1); return; }
      if (e.key === 'Escape') { e.preventDefault(); setSlashDismissed(slashSpan!.start); return; }
      // Tab completes; Enter completes too unless the token already is the active command — then it sends, so `/compact⏎` is one keystroke
      const hit = slash.hits[slash.active];
      if (e.key === 'Tab' || (e.key === 'Enter' && !e.shiftKey && hit && hit.name !== slashSpan!.query)) { e.preventDefault(); if (hit) pickCommand(hit); return; }
    }
    if (mentionOpen) {
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') { e.preventDefault(); move(e.key === 'ArrowDown' ? 1 : -1); return; }
      if (e.key === 'Escape') { e.preventDefault(); setDismissed(span!.start); return; }
      // Enter / Tab pick the active hit; while the hits for this query are still on their way, Enter waits instead of sending a half-typed @ (Shift+Enter still breaks the line)
      if ((e.key === 'Enter' && !e.shiftKey) || e.key === 'Tab') {
        if (!ready) { e.preventDefault(); return; }
        if (hits[active]) { e.preventDefault(); pick(hits[active]!); return; }
      }
    }
    if (e.key === 'Escape' && p.edit && !sending) { e.preventDefault(); p.edit.onCancel(); return; }
    if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); void send(); }
  };
  const syncCaret = (el: HTMLTextAreaElement) => {
    setCaret(el.selectionStart);
    setCollapsed(el.selectionStart === el.selectionEnd);
    if (dismissed !== undefined && !mentionAt(el.value, el.selectionStart)) setDismissed(undefined);
    if (slashDismissed !== undefined && !commandAt(el.value, el.selectionStart)) setSlashDismissed(undefined);
  };

  const field = (
    <div
      ref={fieldRef}
      data-busy={dim || undefined}
      data-drag={dragging || undefined}
      data-focused={beamActive || undefined}
      className={cn('composer-field flex flex-col', flush ? 'rounded-none' : 'rounded-lg')}
      onFocus={() => setFocused(true)}
      onBlur={e => { if (!e.currentTarget.contains(e.relatedTarget)) setFocused(false); }}
      onDragEnter={onDragEnter}
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
    >
      {/* Kept attachments of an edited prompt lead the same wrapping row as freshly pasted ones */}
      <DraftChips drafts={drafts} before={p.edit?.hasAttachments ? p.edit.attachments : undefined}
        onRemove={gone => setDrafts(d => d.filter((_, j) => !gone.includes(j)))}
        onUpdate={(i, next) => setDrafts(d => d.map((x, j) => (j === i ? next : x)))} />
      <PromptInput
        marks={marks}
        ref={textarea}
        rows={1}
        value={text}
        disabled={p.disabled || sending}
        autoFocus={!!p.edit}
        aria-label={p.edit ? t('history.edit') : t('composer.placeholder')}
        onChange={e => { setText(e.target.value); syncCaret(e.target); }}
        onKeyUp={e => syncCaret(e.currentTarget)}
        onClick={e => syncCaret(e.currentTarget)}
        onSelect={e => syncCaret(e.currentTarget)}
        onKeyDown={onKeyDown}
        onPaste={onPaste}
        placeholder={p.disabled ? t('composer.notReady') : p.revisePlan ? t('composer.placeholder.revisePlan') : p.running ? t(p.steer ? 'composer.placeholder.steer' : 'composer.placeholder.queue') : t('composer.placeholder')}
        className={cn(
          // Match message text and attachment insets in every composer, including inline editors.
          'min-w-0 resize-none bg-transparent px-pad pt-2.5 pb-1 text-1 outline-none transition-[color,min-height]',
          'max-h-[calc(8*var(--text-1-lh))] placeholder:text-fg-3',
          // Queued / follow-up text while a turn runs stays at full strength; only an unready session dims the field
          p.disabled ? 'text-fg-3/60 placeholder:text-fg-3/60' : 'text-fg-strong',
        )}
      />
      {/* The hint sits under the text like a second, faint line: the agent's own wording for what to type after the command */}
      {hint && <div className="truncate px-pad pb-1 font-mono text-mono text-fg-3">{hint}</div>}
      {slashOpen && <SlashList anchor={fieldRef} hits={slash.hits} active={slash.active} onHover={slash.setActive} onPick={pickCommand} />}
      {mentionOpen && <MentionList anchor={fieldRef} hits={hits} personas={personas} active={active} empty={span!.query.length > 0} onHover={setActive} onPick={pick} />}
      {/* The row is a container: below the sm tier (a 380 sidebar leaves ~324 here) the mode chip collapses to icon + caret so the option chips keep their room —
          the same move Cursor makes in a narrow sidebar; the editor panel is wide enough for the names */}
      {/* The row is always --ctl tall: the context ring is the one --ctl control among --ctl-sm chips, and it comes and goes with
          the agent's usage knowledge (Grok reports none until the first model round, none again right after a model switch) —
          without the floor the whole composer jumped by the difference every time. The floor lives on the row, not the fieldset:
          a fieldset lays children out in an anonymous inner box that ignores the fieldset's min-height, so items-center there
          only centered the chips within their own --ctl-sm height and they rode a few px above the row-centered send button.
          box-content keeps the floor off the padding: under the preflight's border-box, min-h-ctl minus pt-1 / pb-2 left less than a chip */}
      <div className="@container box-content flex min-h-ctl min-w-0 items-center gap-1 px-2 pt-1 pb-2">
        <fieldset disabled={p.disabled || p.controlsLocked || sending} className="m-0 flex min-w-0 shrink-0 items-center gap-1 border-0 p-0">
          <div className="flex shrink-0 items-center gap-1">
            {/* Working mode first (Codex collaboration_mode: Build / Plan), then the permission modes; both are solid chips that never truncate */}
            {collaboration.map(c => (
              <ModeMenu key={c.id} title={c.name} value={c.value} onOpenChange={onOpenChange} onSelect={v => p.onSetConfig(c.id, v)}
                modes={c.options} labelOf={o => (o.id === 'default' ? t('composer.modeShort.build') : modeLabel(o))} />
            ))}
            {p.controls.modes.length > 0 && (
              <ModeMenu title={t('composer.mode')} modes={p.controls.modes} value={p.controls.modeId} onOpenChange={onOpenChange} onSelect={p.onSetMode} />
            )}
            {/* A permission level offered as a config option (custom category `_permission_mode`) follows the working mode */}
            {permission.map(c => (
              <ModeMenu key={c.id} title={c.name} value={c.value} onOpenChange={onOpenChange} onSelect={v => p.onSetConfig(c.id, v)} modes={c.options} />
            ))}
          </div>
        </fieldset>
        {/* The live chips share one slot of the free width with the model side, however many sources there are */}
        {(offered || contexts.length > 0) && <div className="flex min-w-0 max-w-fit flex-1 items-center gap-1">
          {offered && <SelectionChip selection={offered} cwd={p.cwd} on={offeredOn} narrow={liveNarrow} onToggle={() => setExcluded(offeredOn ? selectionKey(offered) : undefined)} />}
          {contexts.length > MAX_CONTEXT_CHIPS
            ? <ContextsMenu contexts={contexts} on={contextOn} narrow={liveNarrow} onToggle={toggleContext} onOpenChange={onOpenChange} />
            : contexts.map(c => <ContextChip key={c.source.id} context={c} on={contextOn(c)} narrow={liveNarrow} onToggle={() => toggleContext(c)} />)}
        </div>}
        {!p.edit && p.toolbarStart}
        {!p.edit && p.main && !!personas?.length && <SummonMenu personas={personas} onOpenChange={onOpenChange} onPick={persona => summon(persona.name, caret)} />}
        <fieldset disabled={p.disabled || p.controlsLocked || sending} className="m-0 flex min-w-0 flex-1 items-center gap-1 border-0 p-0">
          <div className="min-w-0 flex-1" />
          {[...other, ...(!models.length ? [...modelConfig, ...sidekick, ...sidekickEffort] : [])].map(c => (
            <OptionControl key={c.id} end control={c} hidden={p.hidden?.[c.id]} onSelect={v => p.onSetConfig(c.id, v)} onOpenChange={onOpenChange} />
          ))}
          {p.usage && <ContextRing usage={p.usage} turns={p.turns} compacting={p.compacting} canCompact={canCompact} disabled={p.disabled} compactAt={p.compactAt} running={p.running} onCompact={p.onCompact} onOpenChange={onOpenChange} />}
          {models.map((c, i) => (
            <ModelControl key={c.id} control={c} hidden={p.hidden?.[c.id]} reasoning={i === 0 ? reasoning : undefined}
              modelConfig={i === 0 ? modelConfig : undefined} sidekick={i === 0 ? sidekick[0] : undefined} sidekickEffort={i === 0 ? sidekickEffort[0] : undefined}
              hiddenConfig={p.hidden}
              onSetConfig={p.onSetConfig} onSelect={v => p.onSetConfig(c.id, v)} onOpenChange={onOpenChange} />
          ))}
          {p.onAddModel && p.main && !p.edit && !p.controlsLocked && !models.length && (
            <Chip caret={false} className="shrink-0" icon={<Plus strokeWidth={1.75} />} onClick={p.onAddModel}>{t('settings.providers.addModel')}</Chip>
          )}
          {!models.length && reasoning.map(c => (
            <ReasoningControl key={c.id} control={c} onSelect={v => p.onSetConfig(c.id, v)} onOpenChange={onOpenChange} />
          ))}
        </fieldset>
        {p.edit && !p.edit.dismissOnOutside && <IconButton title={t('history.cancel')} aria-label={t('history.cancel')} onClick={p.edit.onCancel}><X /></IconButton>}
        <SendButton running={showStop} filled={canSend} theme={p.theme} onClick={showStop ? p.onStop : () => { void send(); }} />
      </div>
    </div>
  );

  return (
    <div className={cn(p.edit ? 'min-w-0' : flush ? 'pt-0' : 'px-dock pb-dock')}>
      <WorkingBeam active={beamActive} theme={p.theme}>{field}</WorkingBeam>
    </div>
  );
});

// One mode chip and its menu: single-line rows with a glyph each, the agent's own name and description ride along as a tooltip.
// Chip and rows use the same short label (`modeLabel`); the full_access glyph carries the warning color
function ModeMenu({ modes, value, title, onSelect, onOpenChange, labelOf = modeLabel }: {
  modes: SessionOption[];
  value?: string;
  title: string;
  onSelect: (id: string) => void;
  onOpenChange: (open: boolean) => void;
  // Codex's collaboration `default` is its Build mode, not the permission default beside it
  labelOf?: (mode: SessionOption) => string;
}) {
  const mode = modes.find(m => m.id === value);
  const ModeIcon = mode ? modeIcon(mode) : undefined;
  return (
    <DropdownMenu.Root onOpenLifecycle={onOpenChange}>
      <DropdownMenu.Trigger render={<Chip variant="solid" className="ml-0.5 shrink-0" narrow="icon"
        title={mode ? [mode.name, mode.description].filter(Boolean).join(t('common.metaSep')) : title}
        icon={ModeIcon && <ModeIcon strokeWidth={1.75} className={mode?.kind === 'full_access' ? 'text-warn' : undefined} />}>
        {mode ? labelOf(mode) : title}
      </Chip>} />
      <DropdownMenu.Portal><DropdownMenu.Positioner side="top" width="sm"><DropdownMenu.Popup>
        <DropdownMenu.RadioGroup value={value} className="scroll-thin flex max-h-pop flex-col overflow-y-auto">
          {modes.map(m => {
            const Icon = modeIcon(m);
            const label = labelOf(m);
            return <DropdownMenu.RadioItem key={m.id} value={m.id} title={[label === m.name ? undefined : m.name, m.description].filter(Boolean).join(t('common.metaSep')) || undefined} onClick={() => onSelect(m.id)}>
              <OptionContent icon={<Icon strokeWidth={1.75} className={m.kind === 'full_access' ? 'text-warn' : undefined} />} checked={m.id === value} checkSlot={!!mode}>{label}</OptionContent>
            </DropdownMenu.RadioItem>;
          })}
        </DropdownMenu.RadioGroup>
      </DropdownMenu.Popup></DropdownMenu.Positioner></DropdownMenu.Portal>
    </DropdownMenu.Root>
  );
}

const NO_CONTEXTS: LiveContext[] = [];
type LiveNarrow = 'icon' | 'icon-xl';
// Beyond this many sources the toolbar shows one count chip whose menu toggles each
const MAX_CONTEXT_CHIPS = 2;

// A source's chip label: its one item's label, or the source name with the item count
function contextLabel(c: LiveContext): string {
  return c.items.length === 1 ? c.items[0]!.label : t('composer.context.items', { source: c.source.name, n: c.items.length });
}

// Another tool's live selection in the toolbar, styled and folded like the editor's
function ContextChip({ context, on, narrow, onToggle }: { context: LiveContext; on: boolean; narrow: LiveNarrow; onToggle: () => void }) {
  const label = contextLabel(context);
  const hint = on ? t('composer.context.include') : t('composer.selection.excluded');
  return (
    <Chip caret={false} narrow={narrow} aria-pressed={on} onClick={onToggle}
      className={cn('min-w-0 max-w-fit flex-1', !on && 'opacity-50 hover:opacity-80')}
      title={`${context.source.name}: ${label}\n${hint}`} aria-label={`${context.source.name}: ${label}. ${hint}`}
      icon={<SquareDashedMousePointer strokeWidth={1.5} />}>
      <span className="min-w-0 truncate">{label}</span>
    </Chip>
  );
}

// More live sources than the toolbar holds: one chip with the count, its menu switching each source in or out
function ContextsMenu({ contexts, on, narrow, onToggle, onOpenChange }: {
  contexts: LiveContext[];
  narrow: LiveNarrow;
  on: (c: LiveContext) => boolean;
  onToggle: (c: LiveContext) => void;
  onOpenChange: (open: boolean) => void;
}) {
  const included = contexts.filter(on).length;
  return (
    <DropdownMenu.Root onOpenLifecycle={onOpenChange}>
      <DropdownMenu.Trigger render={<Chip caret={false} narrow={narrow} className={cn('min-w-0 max-w-fit flex-1', !included && 'opacity-50 hover:opacity-80')}
        title={t('composer.context.sources', { n: contexts.length })} icon={<SquareDashedMousePointer strokeWidth={1.5} />}>
        <span className="min-w-0 truncate">{t('composer.context.sources', { n: contexts.length })}</span>
      </Chip>} />
      <DropdownMenu.Portal><DropdownMenu.Positioner side="top" width="sm"><DropdownMenu.Popup>
        {contexts.map(c => (
          <DropdownMenu.CheckboxItem key={c.source.id} checked={on(c)} closeOnClick={false} onCheckedChange={() => onToggle(c)}
            render={<button type="button" />} nativeButton className={cn(optionClass)} title={`${c.source.name}: ${contextLabel(c)}`}>
            <OptionContent checked={on(c)} checkSlot
              description={c.items.length === 1 ? c.items[0]!.label : t('composer.context.count', { n: c.items.length })}>{c.source.name}</OptionContent>
          </DropdownMenu.CheckboxItem>
        ))}
      </DropdownMenu.Popup></DropdownMenu.Positioner></DropdownMenu.Portal>
    </DropdownMenu.Root>
  );
}

// The editor's live selection in the toolbar: `name (12-19)`, dimmed while left out. The name truncates first so the range stays readable
function SelectionChip({ selection, cwd, on, narrow, onToggle }: { selection: EditorSelection; cwd: string; on: boolean; narrow: LiveNarrow; onToggle: () => void }) {
  const draft = selectionDraft(selection, cwd);
  const range = lineRangeLabel(draft.startLine, draft.endLine);
  const hint = on ? t('composer.selection.include') : t('composer.selection.excluded');
  return (
    // Splits the free width with the right-hand controls (both basis 0, this one capped at its content) and folds to its icon
    // on a narrow toolbar, so a long file name never squeezes the model name
    <Chip caret={false} narrow={narrow} aria-pressed={on} onClick={onToggle}
      className={cn('min-w-0 max-w-fit flex-1', !on && 'opacity-50 hover:opacity-80')}
      title={`${draft.name} ${range}\n${hint}`} aria-label={`${draft.name} ${range}. ${hint}`}
      icon={<TextSelect strokeWidth={1.5} />}>
      <span className="flex min-w-0 gap-1">
        <span className="min-w-0 truncate">{draft.name.split('/').pop()}</span>
        <span className="shrink-0 text-fg-3">{range}</span>
      </span>
    </Chip>
  );
}
