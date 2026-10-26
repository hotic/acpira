import type { ReactNode } from 'react';
import { Check, CircleAlert, Download, LoaderCircle, SquareTerminal } from 'lucide-react';
import type { MsgKey } from '@acpira/shared/i18n';
import type { ConnectFailure, ConnectionView, ConnectStep } from '@acpira/shared/remote';
import { cn } from '../ui/cn';
import { Button } from '../ui/Button';
import { Shimmer } from '../ui/Shimmer';
import { t } from '../i18n';
import { connectionOp } from './api';

// A connection attempt in one block of fixed height (--connect-status), on a host's page and in the remote folder picker:
// while connecting, the steps (done, under way with its newest detail, still to come); after a failure, what failed,
// what to do about it, the engine's own words and the action that settles it. Both layouts sit in the same grid cell,
// so a retry flipping between them (a network failure retries on its own) never changes the block's height. Icons are
// --icon squares on the text's line, the same column as the section title's state dot

const STEP_NAME: Record<ConnectStep, MsgKey> = {
  ssh: 'connections.step.ssh', probe: 'connections.step.probe', fetch: 'connections.step.fetch', upload: 'connections.step.upload', start: 'connections.step.start',
};
const STEP_FAILED: Record<ConnectStep, MsgKey> = {
  ssh: 'connections.failed.ssh', probe: 'connections.failed.probe', fetch: 'connections.failed.fetch', upload: 'connections.failed.upload', start: 'connections.failed.start',
};
// What to do about a failure, in words; `other` has only the engine's own message
const FIX: Partial<Record<ConnectFailure, MsgKey>> = {
  hostKey: 'connections.fix.hostKey', keyLocked: 'connections.fix.keyLocked', denied: 'connections.fix.denied', network: 'connections.fix.network',
  machineChanged: 'connections.fix.machineChanged', unsupported: 'connections.fix.unsupported', noSsh: 'connections.fix.noSsh',
};
// The steps shown in order; the download takes the upload's place while it runs (it comes right before it)
const SLOTS: ConnectStep[] = ['ssh', 'probe', 'upload', 'start'];
const SLOT_OF: Record<ConnectStep, number> = { ssh: 0, probe: 1, fetch: 2, upload: 2, start: 3 };

/** Whether the block has anything to show: an attempt under way or a failure with its reason */
export function hasConnectStatus(view: ConnectionView): boolean {
  return view.state === 'connecting' || (view.state === 'error' && !!view.error);
}

/** Connect again: a switched-off host comes back on through add, which also connects */
export function reconnect(view: ConnectionView, extra?: { acceptNewMachine?: boolean; fetchBuild?: boolean }) {
  connectionOp(view.disabled ? { op: 'add', alias: view.alias, mode: view.mode } : { op: 'connect', alias: view.alias, ...extra });
}

/**
 * `retry`: offer Retry beside a failure (the folder picker; a host's page has Connect in its section head).
 * `onOpenConfig`: an authentication refusal offers ~/.ssh/config
 */
export function ConnectStatus({ view, retry, onOpenConfig, className }: { view: ConnectionView; retry?: boolean; onOpenConfig?: () => void; className?: string }) {
  const error = view.state === 'error' ? view.error : undefined;
  return (
    <div className={cn('grid h-(--connect-status) min-w-0', className)}>
      <Steps view={view} className={cn('[grid-area:1/1]', error && 'invisible')} />
      <div className={cn('[grid-area:1/1] min-w-0', !error && 'invisible')} aria-hidden={!error || undefined}>
        {error && <Failure view={view} retry={retry} onOpenConfig={onOpenConfig} />}
      </div>
    </div>
  );
}

function Steps({ view, className }: { view: ConnectionView; className?: string }) {
  const step = view.step ?? 'ssh';
  const at = SLOT_OF[step];
  return (
    <ol className={cn('m-0 flex min-w-0 list-none flex-col justify-between p-0', className)}>
      {SLOTS.map((slot, i) => {
        const name = i === at ? STEP_NAME[step] : STEP_NAME[slot];
        const icon = i < at ? <Check className="size-icon text-fg-3" strokeWidth={1.75} />
          : i === at ? <LoaderCircle className="size-icon animate-spin live-spin text-fg-2" strokeWidth={1.75} />
            : <span className="size-(--dot) rounded-full bg-fg-3 opacity-50" />;
        return <li key={slot} className="flex h-(--text-2-lh) min-w-0 items-center gap-2 text-2">
          <span className="flex size-icon shrink-0 items-center justify-center">{icon}</span>
          {i === at ? <Shimmer className="shrink-0 text-fg-1">{t(name)}</Shimmer>
            : <span className={cn('shrink-0', i < at ? 'text-fg-2' : 'text-fg-3')}>{t(name)}</span>}
          {i === at && view.detail && <span className="min-w-0 truncate font-mono text-mono text-fg-3" title={view.detail}>{view.detail}</span>}
        </li>;
      })}
    </ol>
  );
}

function Failure({ view, retry, onOpenConfig }: { view: ConnectionView; retry?: boolean; onOpenConfig?: () => void }) {
  const error = view.error!;
  const { alias } = view;
  const fix = FIX[error.kind];
  const title = error.kind === 'machineChanged' ? t('connections.failed.machine') : t(STEP_FAILED[error.step]);
  const again = retry && <Button onClick={() => reconnect(view)}>{t('common.retry')}</Button>;
  const actions: ReactNode = error.kind === 'hostKey' || error.kind === 'keyLocked'
    ? <>{again}<Button variant="primary" onClick={() => connectionOp({ op: 'openTerminal', alias })}><SquareTerminal className="size-icon" strokeWidth={1.5} />{t('connections.openTerminal')}</Button></>
    : error.kind === 'machineChanged'
      ? <><Button onClick={() => connectionOp({ op: 'disconnect', alias })}>{t('connections.cancel')}</Button>
        <Button variant="primary" onClick={() => reconnect(view, { acceptNewMachine: true })}>{t('connections.acceptMachine')}</Button></>
      : error.kind === 'unsupported'
        ? <Button variant="primary" onClick={() => reconnect(view, { fetchBuild: true })}><Download className="size-icon" strokeWidth={1.5} />{t('connections.fetchBuild')}</Button>
        : error.kind === 'denied' && onOpenConfig
          ? <>{again}<Button onClick={onOpenConfig}>{t('connections.editConfigShort')}</Button></>
          : again;
  // Two columns: the icon, then the title with its action and, under it, the fix and the engine's words (each at most
  // two lines, the rest in its tooltip)
  return (
    <div className="grid min-w-0 grid-cols-[var(--icon)_minmax(0,1fr)] items-center gap-x-2 gap-y-(--stack-gap)">
      <CircleAlert className="size-icon text-danger" strokeWidth={1.75} />
      <div className="flex h-ctl min-w-0 items-center gap-2">
        <span className="min-w-0 flex-1 truncate text-2 text-danger">{title}</span>
        {actions && <span className="flex shrink-0 items-center gap-2">{actions}</span>}
      </div>
      {fix && <p className="col-start-2 m-0 line-clamp-2 text-2 text-fg-2 [overflow-wrap:anywhere]" title={t(fix)}>{t(fix)}</p>}
      {error.message && <p className="col-start-2 m-0 line-clamp-2 font-mono text-mono text-fg-3 [overflow-wrap:anywhere]" title={error.message}>{error.message}</p>}
    </div>
  );
}
