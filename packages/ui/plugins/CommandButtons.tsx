import { memo, useRef, useState, type ReactNode } from 'react';
import {
  Bot, Bug, Check, Eye, FileText, Flame, GitBranch, GitCommitHorizontal, GitPullRequest, Heart, Languages, ListChecks, LoaderCircle,
  MessageSquare, Package, Play, RefreshCw, Rocket, Search, Sparkles, SquareTerminal, Star, TestTube, WandSparkles, Zap, type LucideIcon,
} from 'lucide-react';
import type { CommandMenuItem, SlashCommand } from '@acpira/shared/transcript';
import { Chip, ChipTag } from '../ui/Button';
import { DropdownMenu, optionClass } from '../ui/DropdownMenu';
import { OptionContent } from '../ui/Panel';
import { cn } from '../ui/cn';
import { t } from '../i18n';
import { callPluginCommand } from '../pluginSlots';
import { vscodeApi } from '../vscodeApi';
import { useArrival } from './Pieces';

// Plugin commands with a `button` (docs/dev/plugins.md Commands), as native chips in the composer's toolbar, so they
// work in every host. A click runs the command with the draft as its arguments: sent like `/name <draft>`, or with
// `draft` its text replaces the draft, one edit the field's undo takes back. A command that takes no draft (`input:
// "none"`) sends `/name` alone and leaves the draft be. One applied from a card flies in from it.
// The plugin picks the glyph and the look; a press sends a ring of the brand colour out of the button (motion.css).
// A command with a `menu` is a menu chip instead, drawn like the mode chip: the engine keeps the session's picks

// The manifest's BUTTON_ICONS, one glyph each (test/pluginButtonIcons.test.ts keeps the two lists equal)
export const BUTTON_ICONS: Record<string, LucideIcon> = {
  bolt: Zap, wand: WandSparkles, sparkles: Sparkles, 'git-commit': GitCommitHorizontal, 'git-branch': GitBranch, 'git-pull-request': GitPullRequest,
  check: Check, play: Play, rocket: Rocket, bug: Bug, test: TestTube, search: Search, eye: Eye, file: FileText, list: ListChecks,
  message: MessageSquare, languages: Languages, refresh: RefreshCw, terminal: SquareTerminal, package: Package, star: Star, heart: Heart,
  flame: Flame, bot: Bot,
};

const LOOK = {
  brand: 'bg-[color-mix(in_srgb,var(--brand)_16%,transparent)] text-fg-1 hover:bg-[color-mix(in_srgb,var(--brand)_26%,transparent)]',
  solid: 'bg-chip text-fg-1 hover:bg-chip-hover',
  outline: 'text-fg-1 shadow-[inset_0_0_0_1px_var(--line-strong)] hover:bg-active',
  icon: 'bg-chip text-fg-1 hover:bg-chip-hover w-ctl-sm justify-center px-0',
};

export const CommandButtons = memo(function CommandButtons({ commands, sessionId, draft, disabled, onSend, onSendBare, onDraft, onRewriting, onNotice, onOpenChange }: {
  commands?: SlashCommand[];
  sessionId?: string;
  draft: string;
  disabled?: boolean;
  // In the draft's place, with its attachments; `onSendBare` leaves both in the field
  onSend: (text: string) => void;
  onSendBare: (text: string) => void;
  // `from`: the draft the command was given, so a result for a draft edited since is dropped
  onDraft: (text: string, from: string) => void;
  // A draft command is working on the draft: the field shows it (composer-field[data-rewriting])
  onRewriting: (busy: boolean) => void;
  onNotice: (text: string) => void;
  // A menu chip's menu opened or closed, as the mode chip reports it
  onOpenChange?: (open: boolean) => void;
}) {
  const buttons = commands?.filter(c => c.plugin && c.button) ?? [];
  if (!buttons.length) return null;
  const args = draft.trim();
  const run = (c: SlashCommand) => async () => {
    if (!c.takesDraft && !c.draft) { onSendBare(`/${c.name}`); return; }
    if (!c.draft) { onSend(args ? `/${c.name} ${args}` : `/${c.name}`); return; }
    if (!sessionId) return;
    onRewriting(true);
    try {
      const text = await callPluginCommand(sessionId, c.name, args);
      onRewriting(false);
      if (text !== undefined) onDraft(text, args);
    } catch (e) {
      onRewriting(false);
      onNotice(t('plugin.actionFailed', { name: c.button!, error: e instanceof Error ? e.message : String(e) }));
    }
  };
  const plugins = [...new Set(buttons.map(c => c.plugin!))];
  return (
    <div data-plugin-slot="composer.toolbar" className="flex shrink-0 items-center gap-1">
      {plugins.map(plugin => (
        // Keyed by its buttons too: a plugin applied again with changed buttons lands again
        <PluginButtons key={`${plugin}|${buttons.filter(c => c.plugin === plugin).map(c => c.name).join(',')}`} plugin={plugin} sessionId={sessionId}>
          {buttons.filter(c => c.plugin === plugin).map(c => c.menu?.length
            ? <CommandMenu key={c.name} command={c} sessionId={sessionId} disabled={disabled} onOpenChange={onOpenChange} />
            : <CommandButton key={c.name} command={c} disabled={disabled} run={run(c)} />)}
        </PluginButtons>
      ))}
    </div>
  );
});

// One plugin's buttons arrive together: the card's lit block lands on the group, which shows once it is down
function PluginButtons({ plugin, sessionId, children }: { plugin: string; sessionId?: string; children: ReactNode }) {
  const box = useRef<HTMLSpanElement>(null);
  const landed = useArrival(plugin, 'composer.toolbar', box, sessionId);
  return (
    <span ref={box} data-plugin-buttons={plugin} className="inline-flex shrink-0 items-center gap-1 rounded-sm transition-opacity duration-200" style={{ opacity: landed ? 1 : 0 }}>
      {children}
    </span>
  );
}

function CommandButton({ command: c, disabled, run }: { command: SlashCommand; disabled?: boolean; run: () => Promise<void> }) {
  // A draft command waits on the plugin's process (a model call may take seconds): one run at a time, shown on the chip
  const [busy, setBusy] = useState(false);
  // Each press mounts a fresh ring, so a quick second press rings again
  const [presses, setPresses] = useState(0);
  const look = c.look && c.look in LOOK ? c.look : 'brand';
  const Icon = (c.icon && BUTTON_ICONS[c.icon]) || (c.draft ? WandSparkles : Zap);
  const glyph = busy ? <LoaderCircle className="animate-spin" strokeWidth={1.75} /> : <Icon strokeWidth={1.75} className={look === 'brand' ? 'text-(--brand)' : look === 'icon' ? 'text-fg-1' : undefined} />;
  return (
    <span className="relative inline-flex shrink-0 rounded-sm">
      <Chip caret={false} disabled={disabled || busy} title={c.description || c.button}
        aria-label={c.button} aria-busy={busy || undefined} className={cn('command-press', LOOK[look])} icon={glyph}
        narrow={look === 'icon' ? undefined : 'icon'}
        onClick={() => { setPresses(n => n + 1); setBusy(true); void run().finally(() => setBusy(false)); }}>
        {/* The icon look is the glyph alone, centred in a square: no label slot to push it aside */}
        {look === 'icon' ? null : c.button}
      </Chip>
      {presses > 0 && <span key={presses} aria-hidden="true" className="command-ring" />}
    </span>
  );
}

// A command button with a `menu`: the chip names the command and carries the first group's pick as a tag, and its menu
// opens upwards like the mode chip's. A radio pick closes the menu as a mode pick does; a toggle keeps it open
function CommandMenu({ command: c, sessionId, disabled, onOpenChange }: { command: SlashCommand; sessionId?: string; disabled?: boolean; onOpenChange?: (open: boolean) => void }) {
  const look = c.look && c.look in LOOK ? c.look : 'brand';
  const Icon = (c.icon && BUTTON_ICONS[c.icon]) || Zap;
  const pick = (item: string) => { if (sessionId) vscodeApi().postMessage({ type: 'pluginMenu', sessionId, name: c.name, item }); };
  const items = c.menu ?? [];
  const wide = items.some(i => i.description);
  return (
    <DropdownMenu.Root onOpenLifecycle={onOpenChange}>
      <DropdownMenu.Trigger disabled={disabled || !sessionId} render={<Chip className={cn('shrink-0', LOOK[look])} narrow={look === 'icon' ? undefined : 'icon'}
        title={[c.description || c.button, c.badge].filter(Boolean).join('\n')} aria-label={c.button}
        icon={<Icon strokeWidth={1.75} className={look === 'brand' ? 'text-(--brand)' : look === 'icon' ? 'text-fg-1' : undefined} />}
        tags={c.badge ? [<ChipTag key="badge">{c.badge}</ChipTag>] : undefined} tagsKey={c.badge ?? ''}>
        {look === 'icon' ? null : c.button}
      </Chip>} />
      <DropdownMenu.Portal><DropdownMenu.Positioner side="top" width={wide ? 'md' : 'sm'}><DropdownMenu.Popup>
        <div className="scroll-thin flex max-h-pop flex-col overflow-y-auto">
          {menuRuns(items).map((run, i) => <MenuRun key={i} run={run} onPick={pick} />)}
        </div>
      </DropdownMenu.Popup></DropdownMenu.Positioner></DropdownMenu.Portal>
    </DropdownMenu.Root>
  );
}

// Consecutive radio rows of one group form one radio group; every other row stands alone
function menuRuns(items: CommandMenuItem[]): CommandMenuItem[][] {
  const runs: CommandMenuItem[][] = [];
  for (const item of items) {
    const last = runs[runs.length - 1];
    if (item.kind === 'radio' && last?.[0]?.kind === 'radio' && last[0].group === item.group) last.push(item);
    else runs.push([item]);
  }
  return runs;
}

function MenuRun({ run, onPick }: { run: CommandMenuItem[]; onPick: (id: string) => void }) {
  const first = run[0]!;
  // A check slot on every row keeps labels aligned whether a row is checked or not
  const content = (i: CommandMenuItem) => <OptionContent checked={!!i.checked} checkSlot description={i.description}>{i.label}</OptionContent>;
  // A row with a description is two lines: a little air keeps neighbours apart
  const roomy = (i: CommandMenuItem) => (i.description ? 'py-1' : undefined);
  switch (first.kind) {
    case 'separator':
      return <DropdownMenu.Separator className="my-1 h-px shrink-0 bg-line" />;
    case 'radio':
      return (
        <DropdownMenu.RadioGroup value={run.find(i => i.checked)?.id}>
          {run.map(i => <DropdownMenu.RadioItem key={i.id} value={i.id} className={roomy(i)} onClick={() => onPick(i.id)}>{content(i)}</DropdownMenu.RadioItem>)}
        </DropdownMenu.RadioGroup>
      );
    case 'toggle':
      return (
        <DropdownMenu.CheckboxItem checked={!!first.checked} closeOnClick={false} onCheckedChange={() => onPick(first.id)}
          render={<button type="button" />} nativeButton className={cn(optionClass, roomy(first))}>
          {content(first)}
        </DropdownMenu.CheckboxItem>
      );
    default:
      return <DropdownMenu.Item className={roomy(first)} onClick={() => onPick(first.id)}><OptionContent checkSlot description={first.description}>{first.label}</OptionContent></DropdownMenu.Item>;
  }
}
