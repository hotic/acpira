import { useCallback, useRef, useState, type ClipboardEvent, type DragEvent, type KeyboardEvent, type ReactNode } from 'react';
import { X } from 'lucide-react';
import type { Draft, SessionControls, SlashCommand, Turn, Usage } from '@shared/transcript';
import type { FileHit } from '@shared/protocol';
import type { HiddenMap } from '@shared/settings';
import { composerControls } from '@shared/composerControls';
import { commandName, namedCommand } from '@shared/slashCommands';
import { useAppearance } from '../appearance';
import { getLocale, t } from '../i18n';
import { cn } from '../ui/cn';
import { Chip, IconButton } from '../ui/Button';
import { DropdownMenu } from '../ui/DropdownMenu';
import { OptionContent } from '../ui/Panel';
import { WorkingBeam } from '../effects/WorkingBeam';
import { SendButton } from '../effects/SendButton';
import { DraftChips } from './Attachments';
import { collectDrafts, hasPayload } from './drafts';
import { MentionList, mentionAt, useMentionHits } from './Mention';
import { SlashList, commandAt, commandHint, useSlashHits } from './Slash';
import { modeIcon } from './modeIcons';
import { ModelControl, OptionControl, ReasoningControl } from './ModelPicker';
import { ContextRing } from './ContextUsage';
import { useComposerDraft } from './useComposerDraft';
import { PromptInput } from './PromptInput';

export interface ComposerProps {
  running: boolean;
  // Text + send locked for login / error / read-only. Starting still accepts a first prompt into the queue
  disabled?: boolean;
  // Mode / model chips stay inert until session/new has real controls
  controlsLocked?: boolean;
  theme: 'dark' | 'light';
  controls: SessionControls;
  turns: Turn[];
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
}

// Composer has three layers: attachment chips (when any), the input area, and a toolbar row below.
// One layout for all ACP agents: mode on the left; context, model with reasoning, and send on the right.
// Attachments come from pasting / dropping (images, OS files, Explorer items) or from an @ mention that searches the workspace
export function Composer(p: ComposerProps) {
  const { composer } = useAppearance();
  const { text, setText, drafts, setDrafts } = useComposerDraft(p.draftKey, p.edit?.text);
  const flush = !p.edit && composer === 'flush';
  // The beam lights up while the composer is focused (focus-within semantics), not while it's working.
  // Overlays portal to the shell root, so opening a menu blurs the composer; "a menu is open" therefore also counts as focused
  const [focused, setFocused] = useState(false);
  const [openMenus, setOpenMenus] = useState(0);
  const onOpenChange = useCallback((open: boolean) => setOpenMenus(n => n + (open ? 1 : -1)), []);
  const beamActive = focused || openMenus > 0;
  const mode = p.controls.modes.find(m => m.id === p.controls.modeId);
  const ModeIcon = mode ? modeIcon(mode) : undefined;
  const dim = p.running || p.disabled;
  const { models, reasoning, other } = composerControls(p.controls.options);
  const canCompact = !!p.commands?.some(c => c.name === 'compact');
  // Files are read asynchronously after a paste / drop; sending is held until every read has landed, so a message never leaves without its attachments
  const [reading, setReading] = useState(0);
  const [sending, setSending] = useState(false);
  const canSend = !p.disabled && !sending && reading === 0 && (text.trim().length > 0 || drafts.length > 0 || !!p.edit?.hasAttachments);
  const send = async () => {
    if (!canSend) return;
    setSending(true);
    try {
      await p.onSend(text, drafts);
      setText('');
      setDrafts([]);
      setDismissed(undefined);
      setSlashDismissed(false);
    } catch (e) {
      p.onNotice(e instanceof Error ? e.message : String(e));
    } finally {
      setSending(false);
    }
  };

  // Drop zone: the whole field lights up while something attachable hovers over it. Enter / leave are counted rather than trusting relatedTarget,
  // which is null for drags coming from outside the window
  const dragDepth = useRef(0);
  const [dragging, setDragging] = useState(false);
  const fieldRef = useRef<HTMLDivElement>(null);
  const addFrom = async (dt: DataTransfer) => {
    setReading(n => n + 1);
    try {
      const { drafts: more, refused } = await collectDrafts(dt, p.cwd);
      if (more.length) setDrafts(d => [...d, ...more.filter(m => m.kind !== 'file' || !d.some(x => x.kind === 'file' && x.uri === m.uri))]);
      if (refused.length) p.onNotice(refused.join(t('common.listSep')));
    } catch (e) {
      p.onNotice(t('composer.attachFailed', { error: e instanceof Error ? e.message : String(e) }));
    } finally {
      setReading(n => n - 1);
    }
  };
  const takes = (dt: DataTransfer | null) => !p.disabled && hasPayload(dt);
  const onDragEnter = (e: DragEvent) => { if (!takes(e.dataTransfer)) return; e.preventDefault(); dragDepth.current++; setDragging(true); };
  const onDragOver = (e: DragEvent) => { if (takes(e.dataTransfer)) e.preventDefault(); };
  const onDragLeave = (e: DragEvent) => { if (!takes(e.dataTransfer)) return; if (--dragDepth.current <= 0) { dragDepth.current = 0; setDragging(false); } };
  // Plain text drags are left to the textarea's native handling; only attachable payloads are taken over
  const onDrop = (e: DragEvent) => {
    dragDepth.current = 0;
    setDragging(false);
    if (!takes(e.dataTransfer)) return;
    e.preventDefault();
    void addFrom(e.dataTransfer);
  };
  // Likewise plain text pastes fall through; only pastes carrying files are taken over
  const onPaste = (e: ClipboardEvent) => {
    if (!hasPayload(e.clipboardData) || !e.clipboardData.files.length) return;
    e.preventDefault();
    void addFrom(e.clipboardData);
  };

  // @ mention: the span under the caret drives the file list (only with a collapsed selection); Esc parks it for that @ until the caret leaves or the message is sent
  const textarea = useRef<HTMLTextAreaElement>(null);
  const [caret, setCaret] = useState(0);
  const [collapsed, setCollapsed] = useState(true);
  const [dismissed, setDismissed] = useState<number>();
  const span = collapsed ? mentionAt(text, caret) : undefined;
  const mentionOpen = !!span && span.start !== dismissed && !p.disabled;
  const { hits, ready, active, setActive, move } = useMentionHits(mentionOpen ? span.query : undefined, p.onSearchFiles);
  const pick = (hit: FileHit) => {
    if (!span) return;
    const next = text.slice(0, span.start) + text.slice(caret);
    setText(next);
    setCaret(span.start);
    setDrafts(d => (d.some(x => x.kind === 'file' && x.uri === hit.uri) ? d : [...d, { kind: 'file', uri: hit.uri, name: hit.path }]));
    requestAnimationFrame(() => textarea.current?.setSelectionRange(span.start, span.start));
  };
  // / command: a leading slash with the caret in its token lists the commands the agent advertised for this session. Picking only completes
  // the text (`/name `) — the command goes out through the ordinary send path and the agent runs it; the list is discovery, never a whitelist,
  // so with no match the slash stays plain text. Mutually exclusive with @ (a text starting with `/` has no @ at its start)
  const [slashDismissed, setSlashDismissed] = useState(false);
  const slashSpan = collapsed && !slashDismissed && !p.disabled ? commandAt(text, caret) : undefined;
  const slash = useSlashHits(p.commands, slashSpan?.query);
  const slashOpen = !!slashSpan && slash.hits.length > 0;
  const pickCommand = (c: SlashCommand) => {
    // Whatever follows the caret inside the token goes too; text after the token is kept as the arguments
    const tail = text.slice(caret).replace(/^\S*/, '').trimStart();
    const head = `/${c.name} `;
    setText(head + tail);
    setCaret(head.length);
    requestAnimationFrame(() => textarea.current?.setSelectionRange(head.length, head.length));
  };
  // The input hint of the command the text names, while its arguments are still empty (the open list already shows it in the row)
  const hint = !slashOpen && p.commands ? commandHint(p.commands, text, getLocale()) : undefined;
  const command = namedCommand(p.commands ?? [], text);
  const unknownCommand = !slashOpen && !command && commandName(text);
  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.nativeEvent.isComposing) return;
    if (slashOpen) {
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') { e.preventDefault(); slash.move(e.key === 'ArrowDown' ? 1 : -1); return; }
      if (e.key === 'Escape') { e.preventDefault(); setSlashDismissed(true); return; }
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
    if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); send(); }
  };
  const syncCaret = (el: HTMLTextAreaElement) => {
    setCaret(el.selectionStart);
    setCollapsed(el.selectionStart === el.selectionEnd);
    if (dismissed !== undefined && !mentionAt(el.value, el.selectionStart)) setDismissed(undefined);
    if (slashDismissed && !commandAt(el.value, el.selectionStart)) setSlashDismissed(false);
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
      <DraftChips drafts={drafts} before={p.edit?.hasAttachments ? p.edit.attachments : undefined} onRemove={i => setDrafts(d => d.filter((_, j) => j !== i))} />
      <PromptInput
        command={command?.name}
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
        placeholder={p.disabled ? t('composer.notReady') : p.running ? t('composer.placeholder.queue') : t('composer.placeholder')}
        className={cn(
          // Match message text and attachment insets in every composer, including inline editors.
          'min-w-0 resize-none bg-transparent px-pad pt-2.5 pb-1 text-1 outline-none transition-colors',
          'max-h-[calc(8*var(--text-1-lh))] placeholder:text-fg-3',
          // Queued / follow-up text while a turn runs stays at full strength; only an unready session dims the field
          p.disabled ? 'text-fg-3/60 placeholder:text-fg-3/60' : 'text-fg-strong',
        )}
      />
      {/* The hint sits under the text like a second, faint line: the agent's own wording for what to type after the command */}
      {hint && <div className="truncate px-pad pb-1 font-mono text-mono text-fg-3">{hint}</div>}
      {unknownCommand && <div className="px-pad pb-1 text-3 text-fg-3">{t('composer.commandUnknown')}</div>}
      {slashOpen && <SlashList anchor={fieldRef} hits={slash.hits} active={slash.active} onHover={slash.setActive} onPick={pickCommand} />}
      {mentionOpen && <MentionList anchor={fieldRef} hits={hits} active={active} empty={span!.query.length > 0} onHover={setActive} onPick={pick} />}
      {/* The row is a container: below the sm tier (a 380 sidebar leaves ~324 here) the mode chip collapses to icon + caret so the option chips keep their room —
          the same move Cursor makes in a narrow sidebar; the editor panel is wide enough for the names */}
      <div className="@container flex min-w-0 items-center gap-1 px-2 pt-1 pb-2">
        {/* The row is always --ctl tall: the context ring is the one --ctl control among --ctl-sm chips, and it comes and goes with
            the agent's usage knowledge (Grok reports none until the first model round, none again right after a model switch) —
            without the floor the whole composer jumped by the difference every time */}
        <fieldset disabled={p.disabled || p.controlsLocked || sending} className="m-0 flex min-h-ctl min-w-0 flex-1 items-center gap-1 border-0 p-0">
          <div className="flex shrink-0 items-center gap-1">
            {/* Mode is the one solid chip and never truncates; single-line rows with a glyph each, the description rides along as a tooltip */}
            {p.controls.modes.length > 0 && (
              <DropdownMenu.Root onOpenLifecycle={onOpenChange}>
                <DropdownMenu.Trigger render={<Chip variant="solid" className="ml-0.5 shrink-0" narrow="icon"
                  title={mode ? [mode.name, mode.description].filter(Boolean).join(' · ') : t('composer.mode')}
                  icon={ModeIcon && <ModeIcon strokeWidth={1.75} />}>
                  {mode?.name ?? t('composer.mode')}
                </Chip>} />
                <DropdownMenu.Portal><DropdownMenu.Positioner side="top" width="sm"><DropdownMenu.Popup>
                  <DropdownMenu.RadioGroup value={p.controls.modeId} className="scroll-thin flex max-h-pop flex-col overflow-y-auto">
                    {p.controls.modes.map(m => {
                      const Icon = modeIcon(m);
                      return <DropdownMenu.RadioItem key={m.id} value={m.id} title={m.description} onClick={() => p.onSetMode(m.id)}>
                        <OptionContent icon={<Icon strokeWidth={1.75} />} checked={m.id === p.controls.modeId} checkSlot={!!mode}>{m.name}</OptionContent>
                      </DropdownMenu.RadioItem>;
                    })}
                  </DropdownMenu.RadioGroup>
                </DropdownMenu.Popup></DropdownMenu.Positioner></DropdownMenu.Portal>
              </DropdownMenu.Root>
            )}
          </div>
          <div className="min-w-0 flex-1" />
          {other.map(c => (
            <OptionControl key={c.id} end control={c} hidden={p.hidden?.[c.id]} onSelect={v => p.onSetConfig(c.id, v)} onOpenChange={onOpenChange} />
          ))}
          {p.usage && <ContextRing usage={p.usage} turns={p.turns} canCompact={canCompact && !dim} compactAt={p.compactAt} running={p.running} onCompact={p.onCompact} onOpenChange={onOpenChange} />}
          {models.map((c, i) => (
            <ModelControl key={c.id} control={c} hidden={p.hidden?.[c.id]} reasoning={i === 0 ? reasoning : undefined}
              onSetReasoning={p.onSetConfig} onSelect={v => p.onSetConfig(c.id, v)} onOpenChange={onOpenChange} />
          ))}
          {!models.length && reasoning.map(c => (
            <ReasoningControl key={c.id} control={c} onSelect={v => p.onSetConfig(c.id, v)} onOpenChange={onOpenChange} />
          ))}
        </fieldset>
        {p.edit && !p.edit.dismissOnOutside && <IconButton title={t('history.cancel')} aria-label={t('history.cancel')} onClick={p.edit.onCancel}><X /></IconButton>}
        <SendButton running={p.running} filled={canSend} theme={p.theme} onClick={p.running ? p.onStop : send} />
      </div>
    </div>
  );

  return (
    <div className={cn(p.edit ? 'min-w-0' : flush ? 'pt-0' : 'px-page pb-page pt-2')}>
      <WorkingBeam active={beamActive} theme={p.theme}>{field}</WorkingBeam>
    </div>
  );
}
