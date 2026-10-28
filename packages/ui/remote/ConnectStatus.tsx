import { useRef, type ReactNode } from 'react';
import { Check, Download, LoaderCircle, SquareTerminal } from 'lucide-react';
import type { MsgKey } from '@acpira/shared/i18n';
import type { ConnectFailure, ConnectionView, ConnectStep } from '@acpira/shared/remote';
import { cn } from '../ui/cn';
import { Button } from '../ui/Button';
import { Collapsible } from '../ui/Collapsible';
import { Shimmer } from '../ui/Shimmer';
import { t } from '../i18n';
import { connectionOp } from './api';

// A connection attempt in one block, on a host's page and in the remote folder picker: while connecting, the steps
// (done, under way with its newest detail, still to come); after a failure, one setting row saying what failed and what
// to do about it, with the action that settles it. Each layout keeps its own height and the two fold into each other
// (one closing as the other opens, ui/Collapsible), so a retry flipping between them (a network failure retries on its
// own) eases the block between heights instead of jumping or reserving the taller one as blank space. A host at rest shows the steps at rest (all to come when off, all done when connected): a host's page folds
// the block away once an attempt is over, and that is what it shows while folding. Icons are --icon squares on the
// text's line, the same column as the section title's state dot; a step's icon fades in as it changes

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
  // The failure keeps its last words while it fades out, so a retry never blanks it mid-fade
  const shown = useLast(error && view);
  // Stacked, not overlaid: the closing one gives its height to the opening one, so their texts never cross
  return (
    <div className={cn('flex min-w-0 flex-col', className)}>
      <Collapsible.Section open={!error}><Steps view={view} /></Collapsible.Section>
      <Collapsible.Section open={!!error}>{shown && <Failure view={shown} retry={retry} onOpenConfig={onOpenConfig} />}</Collapsible.Section>
    </div>
  );
}

/** Whether the block has anything to show: an attempt under way or a failure with its reason */
export function hasConnectStatus(view: ConnectionView): boolean {
  return view.state === 'connecting' || (view.state === 'error' && !!view.error);
}

// The last defined value, held after it goes away
function useLast<T>(value: T | undefined | false | ''): T | undefined {
  const last = useRef<T>(undefined);
  if (value) last.current = value;
  return last.current;
}

function Steps({ view }: { view: ConnectionView }) {
  const step = view.step ?? 'ssh';
  // At rest no step runs: an off host has every step to come, a connected one every step done
  const at = view.state === 'off' ? -1 : view.state === 'connected' ? SLOTS.length : SLOT_OF[step];
  return (
    <ol className="m-0 flex min-w-0 list-none flex-col gap-1 p-0">
      {SLOTS.map((slot, i) => {
        const name = i === at ? STEP_NAME[step] : STEP_NAME[slot];
        const kind = i < at ? 'done' : i === at ? 'live' : 'wait';
        const icon = kind === 'done' ? <Check className="size-icon text-fg-3" strokeWidth={1.75} />
          : kind === 'live' ? <LoaderCircle className="size-icon animate-spin live-spin text-fg-2" strokeWidth={1.75} />
            : <span className="size-(--dot) rounded-full bg-fg-3 opacity-50" />;
        return <li key={slot} className="flex h-(--text-2-lh) min-w-0 items-center gap-2 text-2">
          {/* Keyed by kind: a step moving on remounts the slot, which fades in (on the slot, since the spinner's own
              animation would override a fade on the icon) */}
          <span key={kind} className="fade-in flex size-icon shrink-0 items-center justify-center">{icon}</span>
          {i === at ? <Shimmer className="shrink-0 text-fg-1">{t(name)}</Shimmer>
            : <span className={cn('shrink-0 transition-colors duration-(--dur-open)', i < at ? 'text-fg-2' : 'text-fg-3')}>{t(name)}</span>}
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
  // The fix in words, or the engine's own words when there is none (they otherwise sit in the line's tooltip)
  const words = fix ? t(fix) : error.message;
  // One setting row in the page's three text roles (title fg-1, the line under it fg-3, actions on the right), wrapping
  // like a card head: the actions drop under the text once the text column would go below its minimum, so a narrow
  // column never squeezes the fix into an ellipsis
  return (
    <div className="flex min-w-0 flex-wrap items-center gap-gap">
      <div className="flex min-w-0 flex-[1_1_var(--setting-header-copy)] flex-col gap-y-(--stack-gap)">
        <span className="truncate text-2 text-fg-1">{title}</span>
        {words && <p className="m-0 line-clamp-3 text-3 text-fg-3 [overflow-wrap:anywhere]" title={error.message || undefined}>{words}</p>}
      </div>
      {actions && <span className="flex shrink-0 items-center gap-2">{actions}</span>}
    </div>
  );
}
