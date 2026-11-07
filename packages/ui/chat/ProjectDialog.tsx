import { useState } from 'react';
import { Folder, FolderPlus, X } from 'lucide-react';
import type { CategoryOp, ProjectMeta } from '@acpira/shared/transcript';
import { isRemoteFolder, parseRemoteFolder } from '@acpira/shared/remote';
import { Button, IconButton } from '../ui/Button';
import { cn } from '../ui/cn';
import { Collapsible } from '../ui/Collapsible';
import { usePresence } from '../ui/usePresence';
import { t } from '../i18n';
import { blockList, DangerAction, DialogBody, FormField, inputBox, SectionAction, SettingsDialog } from '../settings/controls';
import { shortPath } from '../settings/paths';
import { projectName } from './SessionList';

// A project's name and folders (desktop). The first folder is the primary one: sessions run there (their ACP `cwd`) and
// agents get the others as `additionalDirectories`. Nothing reaches the engine until Save, which sends one `editProject`;
// "remove from list" takes the whole project off, as the header menu's item does. A project on an SSH host keeps one
// folder: the engine sends further folders to local sessions only
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
  const remote = isRemoteFolder(cwd);
  const env = { home: home ?? '', cwd: '' };
  const add = () => {
    void onPickFolder?.().then(dir => { if (dir) setFolders(list => list.includes(dir) ? list : [...list, dir]); });
  };
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
            <span className="flex-1 text-3 text-fg-3" title={t('session.project.foldersDesc')}>{t('session.project.folders')}</span>
            <SectionAction icon={<FolderPlus />} disabled={remote || !onPickFolder} title={remote ? t('session.project.remoteOneFolder') : undefined} onClick={add}>{t('session.project.addFolder')}</SectionAction>
          </div>
          <div className={blockList}>
            {rows.map(({ key: f, open }) => {
              const path = parseRemoteFolder(f)?.path ?? f;
              return <Collapsible.Section key={f} open={open}>
                <div className="group/row flex h-row min-w-0 items-center gap-2 pr-1 pl-3 text-2">
                  <Folder className="size-icon shrink-0 text-fg-3" strokeWidth={1.5} />
                  <span className="max-w-1/2 shrink-0 truncate text-fg-1">{projectName(path)}</span>
                  <span className="min-w-0 flex-1 truncate text-3 text-fg-3" title={path}>{shortPath(path, env)}</span>
                  {/* The first folder is the primary one by its place; the others offer to take it. Both actions keep a
                      --ctl-sm hit area but inset their hover fill by --icon-fill-inset, so it clears the separator above */}
                  {f !== primary && <button type="button" onClick={() => makePrimary(f)}
                    className={cn('group/primary flex h-ctl-sm shrink-0 py-(--icon-fill-inset) text-3 hover:text-fg-1', reveal)}>
                    <span className="flex items-center rounded-md px-2 transition-colors group-hover/primary:bg-hover group-focus-visible/primary:bg-hover">{t('session.project.makePrimary')}</span>
                  </button>}
                  {/* The last folder stays: a project needs one to run in */}
                  <IconButton size="sm" disabled={folders.length < 2} title={t('common.remove')} aria-label={t('common.remove')} onClick={() => remove(f)}
                    className={cn(reveal, 'shrink-0 bg-clip-content p-(--icon-fill-inset) disabled:invisible')}><X strokeWidth={1.5} /></IconButton>
                </div>
              </Collapsible.Section>;
            })}
          </div>
        </div>
      </DialogBody>
    </SettingsDialog>
  );
}
