import type { ReactNode } from 'react';
import { CodeXml, Download, MessagesSquare, RefreshCw, RotateCcw, Tag } from 'lucide-react';
import type { UpdateState } from '@acpira/shared/protocol';
import { t } from '../i18n';
import { Shimmer } from '../ui/Shimmer';
import { Field, ItemRow, Section, SectionAction, Switch } from './controls';
import { TourSection } from './TourSection';

export interface UpdateHandlers {
  // download: fetch a newer version now even with automatic downloads off
  check: (download?: boolean) => void;
  install: () => void;
  setAuto: (on: boolean) => void;
  openExternal: (url: string) => void;
}

const REPO_URL = 'https://github.com/hotic/acpira';
const HELP_URL = `${REPO_URL}/issues`;
const RELEASES_URL = `${REPO_URL}/releases`;

// The phases with work under way: the status line shimmers and the check button waits
const BUSY = new Set<UpdateState['phase']>(['checking', 'downloading', 'staging', 'applying']);

// Desktop only: About, at the rail's foot. The first card is the app: its version as the head, where its updates
// stand (HostMsg updateState, rust/crates/acpira-desktop/src/update/) as the second line with the one next step as the
// head's action, then the automatic download switch. An install that cannot replace itself (a dmg, App Translocation,
// a folder it cannot write, a development build) says why and links to the release page instead. Getting started replays
// the interface tour. The last card links out to the repository: help, release notes, source
export function AboutPage({ state, on, onTour }: { state: UpdateState | undefined; on: UpdateHandlers; onTour?: () => void }) {
  return <>
    {state && <VersionCard state={state} on={on} />}
    {onTour && <TourSection onStart={onTour} />}
    <Section title={t('settings.nav.about')}>
      <ItemRow lead={<MessagesSquare />} title={t('desktop.help')} desc="Issues" onOpen={() => on.openExternal(HELP_URL)} />
      <ItemRow lead={<Tag />} title={t('settings.about.releaseNotes')} desc="Releases" onOpen={() => on.openExternal(RELEASES_URL)} />
      <ItemRow lead={<CodeXml />} title={t('settings.about.source')} desc={t('settings.about.sourceDesc')} onOpen={() => on.openExternal(REPO_URL)} />
    </Section>
  </>;
}

function VersionCard({ state, on }: { state: UpdateState; on: UpdateHandlers }) {
  const busy = BUSY.has(state.phase);
  const manual = state.manual;
  // A build without the release key never checks: there is nothing to start
  const canCheck = manual !== 'unconfigured' && manual !== 'notBundled';
  let action: ReactNode;
  // Every step is the same quiet head action, so the slot never swaps a filled button in and its words keep one edge
  if (state.phase === 'ready') action = <SectionAction icon={<RotateCcw />} onClick={on.install}>{t('settings.update.restart')}</SectionAction>;
  else if (manual && (state.version || !canCheck)) action = <SectionAction onClick={() => on.openExternal(state.releaseUrl)}>{t('settings.update.openRelease')}</SectionAction>;
  else if (state.phase === 'available' || (state.phase === 'failed' && state.version)) action = <SectionAction icon={<Download />} onClick={() => on.check(true)}>{t('settings.update.download')}</SectionAction>;
  // While a download or the install runs the status line is the whole story; a check in flight keeps its action, waiting
  else if (state.phase === 'checking' || !busy) action = <SectionAction icon={<RefreshCw className={busy ? 'animate-spin live-spin' : undefined} />} disabled={busy} onClick={() => on.check()}>{t('settings.update.check')}</SectionAction>;
  const reason = manual && t(`settings.update.manual.${manual}` as const);
  // A build that never checks has no status of its own: its reason is the status. Any other manual install keeps its
  // status here and gives the reason to the switch it greys out, so every state is one head line and one row
  const status = reason && !canCheck ? reason : statusLine(state);
  // Every passing state lives in the head (the status line and the action slot), so the rows below never move: the
  // download bar takes the action's place, a failure's reason follows "failed" on the same line, cut with the whole
  // text on hover
  if (state.phase === 'downloading') {
    const pct = state.total ? Math.min(100, Math.round(((state.done ?? 0) / state.total) * 100)) : 0;
    action = (
      <span className="flex h-ctl-sm w-(--update-bar-w) shrink-0 items-center">
        <span role="progressbar" aria-valuenow={pct} aria-valuemin={0} aria-valuemax={100} aria-label={status}
          className="block h-(--quota-track) w-full overflow-hidden rounded-full bg-active">
          <span className="block h-full rounded-full bg-fg-3 transition-[width] duration-(--quota-fill-duration) ease-out" style={{ width: `${pct}%` }} />
        </span>
      </span>
    );
  }
  const error = state.phase === 'failed' ? state.error : undefined;
  const desc = status && (error
    ? <span className="flex min-w-0 gap-2"><span className="shrink-0">{status}</span><span className="min-w-0 truncate text-fg-3" title={error}>{error}</span></span>
    : <Shimmer active={busy}>{status}</Shimmer>);
  return (
    <Section title={t('settings.update.version', { version: state.current })} desc={desc} action={action}>
      <Field label={t('settings.update.auto')} desc={reason && canCheck ? reason : t('settings.update.auto.desc')}>
        <Switch checked={state.auto} disabled={!!manual} onChange={on.setAuto} label={t('settings.update.auto')} />
      </Field>
    </Section>
  );
}

function statusLine(s: UpdateState): string {
  const version = s.version ?? '';
  switch (s.phase) {
    case 'checking': return t('settings.update.checking');
    // One fact per second line: no check time, no byte counts (the bar on the right shows progress)
    case 'upToDate': return t('settings.update.upToDateNow');
    case 'available': return t('settings.update.available', { version });
    case 'downloading': return t('settings.update.downloading', { version });
    case 'staging': return t('settings.update.staging', { version });
    case 'ready': return t('settings.update.ready', { version });
    case 'applying': return t('settings.update.applying', { version });
    case 'failed': return t('settings.update.failed');
    default: return t('settings.update.idle');
  }
}
