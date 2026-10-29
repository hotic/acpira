import { useEffect, useLayoutEffect, useState } from 'react';
import { Check, ChevronDown, Folder, FolderPlus, MessageSquare, Search } from 'lucide-react';
import type { AgentId, AgentInfo } from '@acpira/shared/transcript';
import { launchable } from '@acpira/shared/agentOrder';
import { parseRemoteFolder } from '@acpira/shared/remote';
import { RemoteFolderIcon } from '../remote/RemoteFolderIcon';
import { cn } from '../ui/cn';
import { Popover } from '../ui/Popover';
import { t } from '../i18n';
import { shortcutLabel } from '../desktop';
import { AgentMark } from './AgentMark';
import { projectName } from './SessionList';

// The desktop app's start page: what an empty session shows around the composer, which stays where a conversation keeps
// it. Above it the agent picker (desktop.css folds it); the project is a dropdown on the sheet header (ProjectPick) and
// importing an agent's own session lives on the column's Projects label. Picking an agent or a project starts the empty
// session over there (Shell). No greeting: the composer is the page

// Mounted folded, opened on the second frame so the fold plays, and kept for the close transition after `open` drops
const CLOSE_MS = 340;
export function usePresence(open: boolean): { mounted: boolean; shown: boolean } {
  const [mounted, setMounted] = useState(open);
  const [shown, setShown] = useState(false);
  if (open && !mounted) setMounted(true);
  useLayoutEffect(() => {
    if (!open) { setShown(false); return; }
    let frame = requestAnimationFrame(() => { frame = requestAnimationFrame(() => setShown(true)); });
    return () => cancelAnimationFrame(frame);
  }, [open]);
  useEffect(() => {
    if (open || !mounted) return;
    const timer = setTimeout(() => setMounted(false), CLOSE_MS);
    return () => clearTimeout(timer);
  }, [open, mounted]);
  return { mounted, shown };
}

// The fold above the composer: a segmented picker of the agents, in the app's shape language (a rounded-lg frame,
// rounded-md segments). The picked segment's name opens on its own grid track while the last one's closes, both on one
// curve, and the fill cross-fades with them, so the segments slide aside instead of jumping. A pick shows at once; the
// session it starts follows. `only`: the agents that can work in this project (a folder on an SSH host under local run
// takes the built-in agent and Claude Code alone); the rest stay in place, greyed out, saying why
export function StartPicker({ open, agents, current, only, onPick }: { open: boolean; agents: AgentInfo[]; current: AgentId; only?: AgentId[]; onPick: (agent: AgentId) => void }) {
  const [want, setWant] = useState<AgentId>();
  // A pick slides the segments under a still pointer: whichever lands beneath it stays unlit until the pointer moves
  const [settling, setSettling] = useState(false);
  useEffect(() => { if (want === current) setWant(undefined); }, [want, current]);
  const pick = want ?? current;
  const offered = launchable(agents);
  return (
    <div data-start-top data-closed={!open || undefined}>
      <div>
        <div data-picker className="flex">
          <div role="radiogroup" aria-label={t('common.agent')} className="flex min-w-0 items-center gap-0.5 rounded-lg border border-line p-1"
            onPointerMove={() => { if (settling) setSettling(false); }}>
            {offered.map(a => {
              const sel = a.id === pick;
              const outside = !!only && !only.includes(a.id);
              const off = a.available === false || outside;
              const why = outside ? t('start.localRunOnly') : t('agent.notInstalled');
              return (
                <button key={a.id} type="button" role="radio" aria-checked={sel} disabled={off} title={off ? `${a.name}${t('common.metaSep')}${why}` : a.name}
                  onClick={() => { if (!sel) { setSettling(true); setWant(a.id); onPick(a.id); } }}
                  className={cn('inline-flex h-ctl-sm shrink-0 items-center rounded-md px-2 transition-[background-color,color] duration-300 ease-(--start-ease) disabled:opacity-40',
                    sel ? 'bg-active text-fg-strong' : cn('text-fg-3', !settling && 'enabled:hover:bg-hover enabled:hover:text-fg-1'))}>
                  <AgentMark id={a.id} name={a.name} />
                  <span className={cn('grid transition-[grid-template-columns,opacity] duration-300 ease-(--start-ease)', sel ? 'grid-cols-[1fr] opacity-100' : 'grid-cols-[0fr] opacity-0')}>
                    <span className="min-w-0 overflow-hidden whitespace-nowrap"><span className="block pr-0.5 pl-1.5 text-2">{a.name}</span></span>
                  </span>
                </button>
              );
            })}
          </div>
        </div>
      </div>
    </div>
  );
}

// The start page's project, on the sheet header after the title: the name opens the projects in the column's order with
// their paths, filtered as you type, the current one ticked; "open another folder" asks the system's folder dialog and
// starts there. The list keeps one height, so filtering never moves the popover. It replaced a bar under the composer
// (2026-10-23): a second, smaller dropdown under the composer's own read as clutter for what is only a path. Where the
// engine makes chats (`chats`), "no project" leads the list: a chat in a folder of its own; on one (`chat`) it is ticked
const PICK = 'inline-flex h-ctl-sm min-w-0 items-center gap-1 rounded-md px-1.5 -mx-1.5 text-2 text-fg-3 transition-colors hover:bg-hover hover:text-fg-1 data-[popup-open]:bg-hover data-[popup-open]:text-fg-1';
export function ProjectPick({ cwd, projects, home, onPick, onOpenOther, chat, chats, onChat }: { cwd: string; projects: string[]; home?: string; onPick: (cwd: string) => void; onOpenOther?: () => void; chat?: boolean; chats?: string; onChat?: () => void }) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState('');
  const q = query.trim().toLowerCase();
  const shown = projects.filter(p => !q || p.toLowerCase().includes(q));
  // A folder on an SSH host reads as the host, then its path there
  const short = (p: string) => {
    const remote = parseRemoteFolder(p);
    if (remote) return `${remote.alias}  ${remote.path}`;
    return home && (p === home || p.startsWith(`${home}/`) || p.startsWith(`${home}\\`)) ? `~${p.slice(home.length)}` : p;
  };
  const close = () => { setOpen(false); setQuery(''); };
  return (
    <Popover.Root open={open} onOpenChange={o => { setOpen(o); if (!o) setQuery(''); }}>
      <Popover.Trigger render={<button type="button" title={cwd} aria-label={t('start.project')} className={PICK}>
        {/* The name fades in when the project changes */}
        <span key={cwd} className="swap-in min-w-0 truncate">{chat ? t('start.noProject') : projectName(cwd)}</span>
        <ChevronDown className={cn('size-3 shrink-0 text-fg-3 transition-transform duration-200', open && 'rotate-180')} strokeWidth={1.5} />
      </button>} />
      <Popover.Portal><Popover.Positioner side="bottom" align="start" width="lg"><Popover.Popup aria-label={t('start.project')}>
        <label className="flex h-ctl items-center gap-2 px-2 text-fg-3">
          <Search className="size-icon shrink-0" strokeWidth={1.5} />
          <input autoFocus value={query} onChange={e => setQuery(e.target.value)} placeholder={t('start.searchProjects')} aria-label={t('start.searchProjects')}
            className="min-w-0 flex-1 bg-transparent text-2 text-fg-1 outline-none placeholder:text-fg-3" />
        </label>
        <div className="my-1 h-px shrink-0 bg-line" />
        {/* Outside the filtered list, so it is always there and the list keeps its height */}
        {chats && (
          <button type="button" role="option" aria-selected={!!chat} title={chats} onClick={() => { close(); if (!chat) onChat?.(); }}
            className="flex shrink-0 items-center gap-gap rounded-md px-2 py-1 text-left transition-colors hover:bg-hover focus-visible:bg-hover">
            <MessageSquare className="size-icon shrink-0 text-fg-3" strokeWidth={1.5} />
            <span className="flex min-w-0 flex-1 flex-col">
              <span className="truncate text-2 text-fg-1">{t('start.noProject')}</span>
              <span className="truncate text-3 text-fg-3">{short(chats)}</span>
            </span>
            {chat && <Check className="size-icon shrink-0 text-fg-1" strokeWidth={1.75} />}
          </button>
        )}
        <div className="scroll-thin flex h-pop flex-col overflow-y-auto" role="listbox" aria-label={t('start.project')}>
          {shown.map(p => (
            <button key={p} type="button" role="option" aria-selected={p === cwd} title={p} onClick={() => { close(); if (p !== cwd) onPick(p); }}
              className="flex shrink-0 items-center gap-gap rounded-md px-2 py-1 text-left transition-colors hover:bg-hover focus-visible:bg-hover">
              {parseRemoteFolder(p) ? <RemoteFolderIcon className="text-fg-3" /> : <Folder className="size-icon shrink-0 text-fg-3" strokeWidth={1.5} />}
              <span className="flex min-w-0 flex-1 flex-col">
                <span className="truncate text-2 text-fg-1">{projectName(p)}</span>
                <span className="truncate text-3 text-fg-3">{short(p)}</span>
              </span>
              {p === cwd && <Check className="size-icon shrink-0 text-fg-1" strokeWidth={1.75} />}
            </button>
          ))}
          {!shown.length && <div className="flex min-h-row items-center px-2 text-3 text-fg-3">{t('start.noProjectMatch')}</div>}
        </div>
        {onOpenOther && <>
          <div className="my-1 h-px shrink-0 bg-line" />
          <button type="button" onClick={() => { close(); onOpenOther(); }}
            className="flex min-h-row shrink-0 items-center gap-gap rounded-md px-2 text-left text-2 text-fg-1 transition-colors hover:bg-hover focus-visible:bg-hover">
            <FolderPlus className="size-icon shrink-0 text-fg-3" strokeWidth={1.5} />
            <span className="min-w-0 flex-1 truncate">{t('start.openFolder')}</span>
            <span className="shrink-0 text-3 text-fg-3">{shortcutLabel('O')}</span>
          </button>
        </>}
      </Popover.Popup></Popover.Positioner></Popover.Portal>
    </Popover.Root>
  );
}
