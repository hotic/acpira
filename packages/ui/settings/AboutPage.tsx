import type { ReactNode } from 'react';
import { CodeXml, MessagesSquare, RefreshCw, Tag } from 'lucide-react';
import type { UpdateState } from '@acpira/shared/protocol';
import { formatBytes } from '@acpira/shared/storage';
import { getLocale, t } from '../i18n';
import { Button } from '../ui/Button';
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

// Desktop only: About Acpira, at the rail's foot. The first card is the app: its version as the head, where its updates
// stand (HostMsg updateState, rust/crates/acpira-desktop/src/update/) as the second line with the one next step as the
// head's action, then the automatic download switch. An install that cannot replace itself (a dmg, App Translocation,
// a folder it cannot write, a development build) says why and links to the release page instead. Getting started replays
// the interface tour. The last card links out to the repository: help, release notes, source
export function AboutPage({ state, on, onTour }: { state: UpdateState | undefined; on: UpdateHandlers; onTour?: () => void }) {
  return <>
    {state && <VersionCard state={state} on={on} />}
    {onTour && <TourSection onStart={onTour} />}
    <Section title="GitHub" desc={REPO_URL.replace(/^https:\/\//, '')}>
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
  if (state.phase === 'ready') action = <Button variant="primary" onClick={on.install}>{t('settings.update.restart')}</Button>;
  else if (manual && (state.version || !canCheck)) action = <SectionAction onClick={() => on.openExternal(state.releaseUrl)}>{t('settings.update.openRelease')}</SectionAction>;
  else if (state.phase === 'available' || (state.phase === 'failed' && state.version)) action = <Button variant="primary" onClick={() => on.check(true)}>{t('settings.update.download')}</Button>;
  // While a download or the install runs the status line is the whole story; a check in flight keeps its action, waiting
  else if (state.phase === 'checking' || !busy) action = <SectionAction icon={<RefreshCw className={busy ? 'animate-spin live-spin' : undefined} />} disabled={busy} onClick={() => on.check()}>{t('settings.update.check')}</SectionAction>;
  const pct = state.total ? Math.min(100, Math.round(((state.done ?? 0) / state.total) * 100)) : 0;
  // An install that never checks has no status of its own: its reason says it all
  const status = manual && state.phase === 'idle' ? undefined : statusLine(state);
  const note = 'text-3 text-fg-3 [overflow-wrap:anywhere]';
  return (
    <Section title={t('settings.update.version', { version: state.current })} desc={status && <Shimmer active={busy}>{status}</Shimmer>} action={action}>
      {(manual || (state.phase === 'failed' && state.error) || state.phase === 'downloading') && (
        <div className="flex flex-col gap-1 py-(--setting-row-pad)">
          {manual && <span className={note}>{t(`settings.update.manual.${manual}` as const)}</span>}
          {state.phase === 'failed' && state.error && <span className={note}>{state.error}</span>}
          {state.phase === 'downloading' && (
            <span role="progressbar" aria-valuenow={pct} aria-valuemin={0} aria-valuemax={100} aria-label={status}
              className="block h-(--quota-track) overflow-hidden rounded-full bg-active">
              <span className="block h-full rounded-full bg-fg-3 transition-[width] duration-(--quota-fill-duration) ease-out" style={{ width: `${pct}%` }} />
            </span>
          )}
        </div>
      )}
      <Field label={t('settings.update.auto')} desc={t('settings.update.auto.desc')}>
        <Switch checked={state.auto} disabled={!!manual} onChange={on.setAuto} label={t('settings.update.auto')} />
      </Field>
    </Section>
  );
}

function statusLine(s: UpdateState): string {
  const version = s.version ?? '';
  switch (s.phase) {
    case 'checking': return t('settings.update.checking');
    case 'upToDate': return s.checkedAt ? t('settings.update.upToDate', { time: checkedLabel(s.checkedAt) }) : t('settings.update.upToDateNow');
    case 'available': return t('settings.update.available', { version });
    case 'downloading': {
      const locale = getLocale();
      return t('settings.update.downloading', { version, done: formatBytes(s.done ?? 0, locale), total: formatBytes(s.total ?? 0, locale) });
    }
    case 'staging': return t('settings.update.staging', { version });
    case 'ready': return t('settings.update.ready', { version });
    case 'applying': return t('settings.update.applying', { version });
    case 'failed': return t('settings.update.failed');
    default: return t('settings.update.idle');
  }
}

// Today's checks by the clock, older ones with their date
function checkedLabel(ms: number): string {
  const at = new Date(ms);
  const today = new Date().toDateString() === at.toDateString();
  const opts: Intl.DateTimeFormatOptions = today ? { hour: '2-digit', minute: '2-digit' } : { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' };
  return new Intl.DateTimeFormat(getLocale(), opts).format(at);
}
