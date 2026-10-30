import { useMemo } from 'react';
import { DEFAULT_SETTINGS, type SettingsView } from '@acpira/shared/settings';
import type { Locale } from '@acpira/shared/i18n';
import type { SessionControls } from '@acpira/shared/transcript';
import type { Appearance } from '../appearance';
import { Shell, type ShellHandlers } from '../chat/Shell';
import { lookFromSettings, type Theme } from '../look';
import { sampleSession } from './sample';

const noop = () => {};
const HANDLERS: ShellHandlers = {
  send: noop, searchFiles: () => Promise.resolve([]), stop: noop, permission: noop, setMode: noop, setConfig: noop,
  selectSession: noop, newSession: noop, renameSession: noop, deleteSession: async () => false, restoreSession: noop, pinSession: noop,
  selectAccount: noop, addAccount: noop, removeAccount: noop, compact: noop, login: noop, retry: noop, retryTurn: noop,
};

// What the composer's corners show (the tour's composer step names them): the modes and a model, as the agent names them
const CONTROLS: SessionControls = {
  modes: [
    { id: 'default', name: 'Default' },
    { id: 'acceptEdits', name: 'Accept Edits' },
    { id: 'plan', name: 'Plan Mode' },
  ],
  modeId: 'default',
  options: [{ id: 'model', name: 'Model', category: 'model', options: [{ id: 'opus', name: 'Opus' }, { id: 'sonnet', name: 'Sonnet' }], value: 'opus' }],
};

export interface SampleShellProps {
  appearance: Appearance;
  theme: Theme;
  // The user's settings once the engine sent them (a replay); the defaults before (the first run)
  settings?: SettingsView;
  host: 'sidebar' | 'editor';
  desktop: boolean;
  // The copy is built in this locale (the sample's titles and turns)
  locale: Locale;
}

// The real shell on the sample session (sample.ts), under the tour. Inert: nothing in it takes focus, a click or a key, so
// the tour's keys and buttons are the only ones live. Its session column is docked wherever there is room
export function SampleShell({ appearance, theme, settings, host, desktop, locale }: SampleShellProps) {
  const sample = useMemo(() => sampleSession(), [locale]); // eslint-disable-line react-hooks/exhaustive-deps
  const look = useMemo(() => lookFromSettings({ ...(settings ?? DEFAULT_SETTINGS), sessionListPosition: 'left', desktopLayout: 'chat' }), [settings]);
  const first = sample.sessions[0]!;
  return (
    <div inert className="h-full" data-sample-shell>
      <Shell sample appearance={appearance} theme={theme} look={look} host={host} desktop={desktop}
        agent={sample.agents[0]!} agents={sample.agents} title={first.title} status="ready" turns={sample.turns} running
        queued={sample.queued} controls={CONTROLS} sessions={sample.sessions} categories={sample.categories} activeSessionId={first.id}
        cwd={sample.cwd} workspace={sample.cwd} sessionScope="all" home={sample.home} on={HANDLERS} onOpenSettings={noop} />
    </div>
  );
}
