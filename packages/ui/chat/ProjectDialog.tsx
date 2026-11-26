import { useState } from 'react';
import { Folder, FolderPlus, Server, X } from 'lucide-react';
import type { CategoryOp, ProjectMeta } from '@acpira/shared/transcript';
import { parseRemoteFolder } from '@acpira/shared/remote';
import { Button, IconButton } from '../ui/Button';
import { cn } from '../ui/cn';
import { Collapsible } from '../ui/Collapsible';
import { DropdownMenu } from '../ui/DropdownMenu';
import { OptionContent } from '../ui/Panel';
import { usePresence } from '../ui/usePresence';
import { t } from '../i18n';
import { blockList, DangerAction, DialogBody, FormField, inputBox, SectionAction, SettingsDialog } from '../chrome/controls';
import { shortPath } from '../chrome/paths';
import { useConnections } from '../remote/api';
import { RemoteFolderDialog } from '../remote/RemoteFolderDialog';
import { RemoteFolderIcon } from '../remote/RemoteFolderIcon';
import { projectName } from './SessionList';
import { Submenu } from './SessionMenu';

// A host's connection dot, the session column's colours
const HOST_DOT: Partial<Record<string, string>> = { connected: 'bg-ok', error: 'bg-warn', connecting: 'bg-fg-3 remote-breathe' };

// A project's name and folders (desktop). The first folder is the primary one: sessions run there (their ACP `cwd`) and
// agents get the others as `additionalDirectories`. Nothing reaches the engine until Save, which sends one `editProject`;
// "remove from list" takes the whole project off, as the header menu's item does. Folders may be on this machine or on
// an SSH host, in one project: with hosts connected, "add folder" asks which (the session column's add menu's shape) and
// a host opens its folder picker over this dialog
export function ProjectDialog({ cwd, meta, home, onPickFolder, onSave, onRemove, onClose }: {
  cwd: string;
  meta?: ProjectMeta;
  home?: string;
  onPickFolder?: () => Promise<string | undefined>;
  onSave: (op: Extract<CategoryOp, { op: 'editProject' }>) => void;
  onRemove: () => void;
  onClose: () => void;
}) {
  const [name, setName] = useState(meta?.name ?? '');
  const [folders, setFolders] = useState<string[]>(() => [cwd, ...(meta?.extra ?? []).filter(f => f !== cwd)]);
  const env = { home: home ?? '', cwd: '' };
  const append = (dir: string) => setFolders(list => list.includes(dir) ? list : [...list, dir]);
  const addLocal = () => { void onPickFolder?.().then(dir => { if (dir) append(dir); }); };
  // The SSH hosts a folder can come from (only the desktop app and the harness send connections), and the one being picked on
  const hosts = useConnections() ?? [];
  const [picking, setPicking] = useState<string>();
  const makePrimary = (f: string) => setFolders(list => [f, ...list.filter(x => x !== f)]);
  const remove = (f: string) => setFolders(list => list.filter(x => x !== f));
  const primary = folders[0];
  // Rows fold in and out as folders are added and removed, so the dialog's bottom edge glides (pinTop)
  const rows = usePresence(folders, f => f);
  const save = () => {
    if (!primary) return;
    const trimmed = name.trim();
    onSave({ op: 'editProject', cwd, primary, extra: folders.slice(1), ...(trimmed ? { name: trimmed } : {}) });
    onClose();
  };
  const reveal = 'text-fg-3 opacity-0 group-hover/row:opacity-100 focus-visible:opacity-100';
  return (
    <SettingsDialog pinTop icon={<Folder />} title={t('session.project.editTitle')} onClose={onClose}
      footer={<>
        <DangerAction onClick={() => { onRemove(); onClose(); }}>{t('session.project.remove')}</DangerAction>
        <span className="flex-1" />
        <Button onClick={onClose}>{t('common.cancel')}</Button>
        <Button variant="primary" disabled={!primary} onClick={save}>{t('common.save')}</Button>
      </>}>
      <DialogBody>
        <FormField label={t('session.project.name')}>
          <input autoFocus value={name} aria-label={t('session.project.name')} placeholder={primary ? projectName(parseRemoteFolder(primary)?.path ?? primary) : undefined}
            onChange={e => setName(e.target.value)} onKeyDown={e => { if (e.key === 'Enter' && !e.nativeEvent.isComposing) save(); }}
            className={cn(inputBox, 'h-ctl w-full bg-hover')} />
        </FormField>
        {/* The folders as a compact list at the row pitch of the session column: name, then its path in the secondary
            role; the actions show on hover. The primary folder leads */}
        <div className="flex min-w-0 flex-col gap-1">
          <div className="flex h-ctl-sm items-center gap-2">
            {/* Same label inset as the FormField above; the action's box ends on the list's edge, not past it */}
            <span className="flex-1 px-(--form-label-inset) text-3 text-fg-3" title={t('session.project.foldersDesc')}>{t('session.project.folders')}</span>
            {hosts.length
              ? <DropdownMenu.Root>
                <DropdownMenu.Trigger render={<SectionAction className="mr-0 data-[popup-open]:bg-hover data-[popup-open]:text-fg-1" icon={<FolderPlus />}>{t('session.project.addFolder')}</SectionAction>} />
                <DropdownMenu.Portal><DropdownMenu.Positioner align="end" width="sm"><DropdownMenu.Popup>
                  {onPickFolder && <DropdownMenu.Item onClick={addLocal}><OptionContent icon={<Folder strokeWidth={1.5} />}>{t('session.project.localFolder')}</OptionContent></DropdownMenu.Item>}
                  <Submenu side="inline-end" icon={<RemoteFolderIcon />} label={t('session.project.remote')}>{hosts.map(c => (
                    <DropdownMenu.Item key={c.alias} title={c.address} onClick={() => setPicking(c.alias)}>
                      <OptionContent icon={<Server strokeWidth={1.5} />}>{c.alias}</OptionContent>
                    </DropdownMenu.Item>
                  ))}</Submenu>
                </DropdownMenu.Popup></DropdownMenu.Positioner></DropdownMenu.Portal>
              </DropdownMenu.Root>
              : <SectionAction className="mr-0" icon={<FolderPlus />} disabled={!onPickFolder} onClick={addLocal}>{t('session.project.addFolder')}</SectionAction>}
          </div>
          <div className={blockList}>
            {rows.map(({ key: f, open }) => {
              const host = parseRemoteFolder(f);
              const path = host?.path ?? f;
              return <Collapsible.Section key={f} open={open}>
                <div className="group/row flex h-row min-w-0 items-center gap-2 pr-1 pl-3 text-2">
                  {host ? <RemoteFolderIcon className="text-fg-3" /> : <Folder className="size-icon shrink-0 text-fg-3" strokeWidth={1.5} />}
                  <span className="max-w-1/2 shrink-0 truncate text-fg-1">{projectName(path)}</span>
                  <span className="min-w-0 flex-1 truncate text-3 text-fg-3" title={host ? f : path}>{host ? path : shortPath(path, env)}</span>
                  {/* The row's tail, one cell: a host folder rests on its host's name and connection dot (the session
                      column's project header), which gives way to the actions on hover or focus */}
                  <span className="grid shrink-0 items-center [grid-template-areas:'x'] *:[grid-area:x] [&:has(:focus-visible)>[data-host-tail]]:opacity-0">
                    {host && <span data-host-tail className="flex items-center justify-end gap-1.5 pr-2 text-3 text-fg-3 transition-opacity group-hover/row:opacity-0">
                      <span className="max-w-host truncate">{host.alias}</span>
                      <span className={cn('size-1.5 shrink-0 rounded-full', HOST_DOT[hosts.find(c => c.alias === host.alias)?.state ?? 'off'] ?? 'bg-fg-3/40')} />
                    </span>}
                    <span className="flex items-center justify-self-end">
                      {/* The first folder is the primary one by its place; the others offer to take it. Both actions keep a
                          --ctl-sm hit area but inset their hover fill by --icon-fill-inset, so it clears the separator above */}
                      {f !== primary && <button type="button" onClick={() => makePrimary(f)}
                        className={cn('group/primary flex h-ctl-sm shrink-0 py-(--icon-fill-inset) text-3 hover:text-fg-1', reveal)}>
                        <span className="flex items-center rounded-md px-2 transition-colors group-hover/primary:bg-hover group-focus-visible/primary:bg-hover">{t('session.project.makePrimary')}</span>
                      </button>}
                      {/* The last folder stays: a project needs one to run in */}
                      <IconButton size="sm" disabled={folders.length < 2} title={t('common.remove')} aria-label={t('common.remove')} onClick={() => remove(f)}
                        className={cn(reveal, 'shrink-0 bg-clip-content p-(--icon-fill-inset) disabled:invisible')}><X strokeWidth={1.5} /></IconButton>
                    </span>
                  </span>
                </div>
              </Collapsible.Section>;
            })}
          </div>
        </div>
      </DialogBody>
      {picking && <RemoteFolderDialog alias={picking} addLabel={t('session.project.addFolder')} onClose={() => setPicking(undefined)}
        onAdd={folder => { setPicking(undefined); append(folder); }} />}
    </SettingsDialog>
  );
}
