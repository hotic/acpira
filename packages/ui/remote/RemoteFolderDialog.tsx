import { useCallback, useEffect, useRef, useState } from 'react';
import { ChevronRight, Folder, GitBranch, House } from 'lucide-react';
import { formatRemoteFolder } from '@acpira/shared/remote';
import { cn } from '../ui/cn';
import { Button } from '../ui/Button';
import { Shimmer } from '../ui/Shimmer';
import { t } from '../i18n';
import { SettingsDialog } from '../settings/controls';
import { browseRemote, connectionOp, useConnections, type RemoteListing } from './api';
import { ConnectStatus } from './ConnectStatus';

// A folder on an SSH host to add as a project: breadcrumbs over a list of fixed height (it never resizes the dialog while
// a folder loads, fails or fills), starting in the host's home. A click on a folder goes into it; the footer adds the
// folder the list shows. Opened from a host's settings page (only while connected) and from the session column's add menu.
// Nothing is listed until the host is connected: a host that is off is connected on opening, and until it is up the list's
// place shows the attempt's steps or its failure (remote/ConnectStatus), with Retry
export function RemoteFolderDialog({ alias, onAdd, onClose }: { alias: string; onAdd: (folder: string) => void; onClose: () => void }) {
  const [home, setHome] = useState<string>();
  // The folder shown; undefined until the first listing named the home
  const [path, setPath] = useState<string>();
  const [listing, setListing] = useState<RemoteListing>();
  const [loading, setLoading] = useState(true);
  const seq = useRef(0);
  // The host's connection; a host the list does not know is left to the engine (browse connects it)
  const view = useConnections()?.find(c => c.alias === alias);
  const ready = !view || view.state === 'connected';
  const off = view?.state === 'off' && !view.disabled;
  useEffect(() => { if (off) connectionOp({ op: 'connect', alias }); }, [off, alias]);
  const pathRef = useRef(path);
  pathRef.current = path;
  const go = useCallback((to?: string) => {
    const mine = ++seq.current;
    setLoading(true);
    void browseRemote(alias, to).then(r => {
      if (mine !== seq.current) return;
      setLoading(false);
      setListing(r);
      if (r.error) return;
      setPath(r.path);
      if (to === undefined) setHome(r.path);
    });
  }, [alias]);
  // Listed once connected, and again where it was after a dropped connection comes back
  useEffect(() => { if (ready) go(pathRef.current); }, [ready, go]);
  // Folders only, hidden ones after the rest
  const dirs = (listing?.entries ?? []).filter(e => e.dir).sort((a, b) => Number(a.name.startsWith('.')) - Number(b.name.startsWith('.')));
  const error = listing?.error && (listing.error === 'timeout' ? t('connections.noAnswer') : listing.error);
  const folder = path !== undefined ? formatRemoteFolder(alias, path) : undefined;
  const join = (name: string) => `${path === '/' ? '' : path}/${name}`;
  return (
    <SettingsDialog title={t('connections.folderTitle', { host: alias })} onClose={onClose}
      footer={<>
        <span className="min-w-0 flex-1 truncate font-mono text-mono text-fg-3" title={path}>{path !== undefined ? shortRemote(path, home) : ''}</span>
        <Button variant="primary" disabled={!ready || loading || !!error || !folder} onClick={() => { if (folder) onAdd(folder); }}>{t('connections.addAsProject')}</Button>
      </>}>
      <div className="flex min-h-0 flex-col p-1">
        <Crumbs path={path} home={home} onGo={go} />
        <div className="my-1 h-px shrink-0 bg-line" />
        <div className="scroll-thin flex h-pop min-h-0 flex-col overflow-y-auto">
          {!ready && view && <ConnectStatus retry className="px-2 py-1"
            view={view.state === 'off' ? { ...view, state: 'connecting', step: 'ssh' } : view} />}
          {ready && loading && <div className="flex min-h-row shrink-0 items-center px-2 text-3 text-fg-3"><Shimmer active>{t('connections.loadingDir')}</Shimmer></div>}
          {ready && !loading && error && <div className="px-2 py-1 text-3 [overflow-wrap:anywhere]"><span className="text-danger">{error}</span></div>}
          {ready && !loading && !error && !dirs.length && <div className="flex min-h-row shrink-0 items-center px-2 text-3 text-fg-3">{t('connections.emptyDir')}</div>}
          {ready && !loading && !error && dirs.map(d => (
            <button key={d.name} type="button" title={join(d.name)} onClick={() => go(join(d.name))}
              className="group flex min-h-row shrink-0 items-center gap-gap rounded-md px-2 text-left text-2 text-fg-1 transition-colors hover:bg-hover focus-visible:bg-hover">
              <Folder className="size-icon shrink-0 text-fg-3" strokeWidth={1.5} />
              <span className="min-w-0 flex-1 truncate">{d.name}</span>
              {d.git && <GitBranch className="size-icon shrink-0 text-fg-3" strokeWidth={1.5} aria-label={t('connections.gitRepo')} />}
              <ChevronRight className="size-icon shrink-0 text-fg-3 opacity-0 group-hover:opacity-100 group-focus-visible:opacity-100" strokeWidth={1.5} />
            </button>
          ))}
        </div>
      </div>
    </SettingsDialog>
  );
}

// The way back up: the home (its name beside the house) or the root, then each folder down to the one shown. Earlier
// crumbs give way first when the row runs out of room, so the folder shown stays readable
function Crumbs({ path, home, onGo }: { path?: string; home?: string; onGo: (to: string) => void }) {
  const inHome = home !== undefined && path !== undefined && (path === home || path.startsWith(`${home}/`));
  const segments = path === undefined ? [] : (inHome ? path.slice(home!.length) : path).split('/').filter(Boolean);
  const crumb = 'inline-flex h-ctl-sm min-w-0 items-center gap-1 rounded-md px-1.5 transition-colors hover:bg-hover hover:text-fg-1 focus-visible:bg-hover focus-visible:text-fg-1';
  let at = inHome ? home! : '';
  return (
    <div className="flex h-ctl min-w-0 items-center gap-0.5 px-1 text-2 text-fg-2">
      {home !== undefined && (
        <button type="button" title={home} aria-label={home} onClick={() => onGo(home)} className={cn(crumb, 'shrink-1000', inHome && !segments.length && 'text-fg-1')}>
          <House className="size-icon shrink-0" strokeWidth={1.5} />
          {inHome && <span className="min-w-0 truncate">{home.split('/').filter(Boolean).pop() ?? home}</span>}
        </button>
      )}
      {!inHome && path !== undefined && <button type="button" onClick={() => onGo('/')} className={cn(crumb, 'shrink-1000', !segments.length && 'text-fg-1')}>/</button>}
      {segments.map((s, i) => {
        at = `${at}/${s}`;
        const to = at;
        const last = i === segments.length - 1;
        return <span key={to} className={cn('flex min-w-0 items-center gap-0.5', !last && 'shrink-1000')}>
          {(inHome || i > 0) && <ChevronRight className="size-3 shrink-0 text-fg-3" strokeWidth={1.5} />}
          <button type="button" title={to} onClick={() => onGo(to)} className={cn(crumb, last && 'text-fg-1')}><span className="min-w-0 truncate">{s}</span></button>
        </span>;
      })}
    </div>
  );
}

// A path of the host as the footer shows it: under the host's home with ~
function shortRemote(path: string, home?: string): string {
  if (!home || home === '/') return path;
  if (path === home) return '~';
  return path.startsWith(`${home}/`) ? `~${path.slice(home.length)}` : path;
}
