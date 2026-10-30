import { memo, useRef, useState, type ReactNode } from 'react';
import {
  Bot, Bug, Check, Eye, FileText, Flame, GitBranch, GitCommitHorizontal, GitPullRequest, Heart, Languages, ListChecks, LoaderCircle,
  MessageSquare, Package, Play, RefreshCw, Rocket, Search, Sparkles, SquareTerminal, Star, TestTube, WandSparkles, Zap, type LucideIcon,
} from 'lucide-react';
import type { SlashCommand } from '@acpira/shared/transcript';
import { Chip } from '../ui/Button';
import { cn } from '../ui/cn';
import { t } from '../i18n';
import { callPluginCommand } from '../pluginSlots';
import { useArrival } from './Pieces';

// Plugin commands with a `button` (docs/dev/plugins.md Commands), as native chips in the composer's toolbar, so they
// work in every host. A click runs the command with the draft as its arguments: sent like `/name <draft>`, or with
// `draft` its text replaces the draft, one edit the field's undo takes back. A command that takes no draft (`input:
// "none"`) sends `/name` alone and leaves the draft be. One applied from a card flies in from it.
// The plugin picks the glyph and the look; a press sends a ring of the brand colour out of the button (motion.css)

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

export const CommandButtons = memo(function CommandButtons({ commands, sessionId, draft, disabled, onSend, onSendBare, onDraft, onRewriting, onNotice }: {
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
          {buttons.filter(c => c.plugin === plugin).map(c => <CommandButton key={c.name} command={c} disabled={disabled} run={run(c)} />)}
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
    <span ref={box} className="inline-flex shrink-0 items-center gap-1 rounded-sm transition-opacity duration-200" style={{ opacity: landed ? 1 : 0 }}>
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
