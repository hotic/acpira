import type { ReactElement } from 'react';
import { ContextMenu } from '@base-ui/react/context-menu';
import { ChevronRight, FileJson, FileText, Folder, FolderInput, Inbox, Pencil, Pin, PinOff, SquareArrowOutUpRight, Trash2, Undo2 } from 'lucide-react';
import type { SessionCategory, SessionSummary } from '@shared/transcript';
import { t } from '../i18n';
import { DropdownMenu } from '../ui/DropdownMenu';
import { OptionContent } from '../ui/Panel';
import { CategoryAddIcon } from './CategoryIcon';

// A project a session can be moved into: its folder, the folder's display name, and whether it is the window's own
export interface MoveProject {
  cwd: string;
  name: string;
  current: boolean;
  // The parent folder, set only when another listed project shares the name, so the two rows can be told apart
  hint?: string;
}

export interface SessionMenuItemsProps {
  session: SessionSummary;
  // Header only: open the session in an editor tab
  onOpenInEditor?: () => void;
  // Row only: starts the inline RenameInput
  onRename?: () => void;
  onPin: () => void;
  // "Move to this project", only when the row offers it and there is no "move to project" submenu (which lists this project)
  onMove?: () => void;
  // "Move to project": every other project the list knows (the window's first), the pick re-homes the session there
  projects?: MoveProject[];
  onMoveTo?: (cwd: string) => void;
  onExport?: (format: 'markdown' | 'json') => void;
  onDelete: () => void;
  // "Move to category": the categories of the session's own project, the pick (null = take it out) and "new category…".
  // Offered only with onFile, and never for a pinned session (pinned ones are locked to the top)
  categories?: SessionCategory[];
  onFile?: (category: string | null) => void;
  onNewCategory?: () => void;
}

export interface SessionMenuProps extends SessionMenuItemsProps {
  // The button that opens the menu (an IconButton); aria-label belongs on it
  trigger: ReactElement;
  align?: 'start' | 'end';
  onOpenChange?: (open: boolean) => void;
}

const separator = (key: string) => <DropdownMenu.Separator key={key} className="my-1 h-px bg-line" />;

// A nested menu row: the trigger with its chevron, the popup opening beside the parent menu
function Submenu({ icon, label, children }: { icon: ReactElement; label: string; children: ReactElement[] }) {
  return (
    <DropdownMenu.SubmenuRoot>
      <DropdownMenu.SubmenuTrigger>
        <OptionContent icon={icon}>{label}</OptionContent>
        <ChevronRight className="size-3 shrink-0 text-fg-3" strokeWidth={1.5} />
      </DropdownMenu.SubmenuTrigger>
      <DropdownMenu.Portal><DropdownMenu.Positioner side="inline-start" align="start" width="md"><DropdownMenu.Popup>
        {children}
      </DropdownMenu.Popup></DropdownMenu.Positioner></DropdownMenu.Portal>
    </DropdownMenu.SubmenuRoot>
  );
}

// The session menu's rows (Cursor's shape): session actions, export, delete. Shared by the "…" dropdown of the list row and the
// header and by the row's right-click menu; Base UI's context menu is built from the same Menu parts, so the rows are one set.
// An item only renders when its handler exists; a separator only between two non-empty groups
function sessionMenuItems({ session, onOpenInEditor, onRename, onPin, onMove, projects = [], onMoveTo, onExport, onDelete, categories = [], onFile, onNewCategory }: SessionMenuItemsProps): ReactElement[] {
  const top: ReactElement[] = [];
  if (onOpenInEditor) top.push(<DropdownMenu.Item key="editor" onClick={onOpenInEditor}><OptionContent icon={<SquareArrowOutUpRight strokeWidth={1.5} />}>{t('session.openInEditor')}</OptionContent></DropdownMenu.Item>);
  if (onRename) top.push(<DropdownMenu.Item key="rename" onClick={onRename}><OptionContent icon={<Pencil strokeWidth={1.5} />}>{t('common.rename')}</OptionContent></DropdownMenu.Item>);
  top.push(<DropdownMenu.Item key="pin" onClick={onPin}><OptionContent icon={session.pinned ? <PinOff strokeWidth={1.5} /> : <Pin strokeWidth={1.5} />}>{session.pinned ? t('common.unpin') : t('common.pin')}</OptionContent></DropdownMenu.Item>);
  if (onFile && !session.pinned) {
    const filed = categories.some(c => c.id === session.category);
    top.push(<Submenu key="file" icon={<Inbox strokeWidth={1.5} />} label={t('session.category.moveTo')}>{[
      ...categories.map(c => (
        <DropdownMenu.Item key={c.id} onClick={() => onFile(c.id)}>
          <OptionContent icon={<Inbox strokeWidth={1.5} />} checked={c.id === session.category} checkSlot={filed}>{c.name}</OptionContent>
        </DropdownMenu.Item>
      )),
      ...categories.length > 0 ? [separator('sep')] : [],
      ...onNewCategory ? [<DropdownMenu.Item key="new" onClick={onNewCategory}><OptionContent icon={<CategoryAddIcon />}>{t('session.category.newEllipsis')}</OptionContent></DropdownMenu.Item>] : [],
      ...filed ? [<DropdownMenu.Item key="remove" onClick={() => onFile(null)}><OptionContent icon={<Undo2 strokeWidth={1.5} />}>{t('session.category.remove')}</OptionContent></DropdownMenu.Item>] : [],
    ]}</Submenu>);
  }
  // Each project is a one-line row like every other menu option: folder name, the "current" badge, and the parent folder only
  // when two projects share a name; the full path is the tooltip
  if (onMoveTo && projects.length) {
    top.push(<Submenu key="project" icon={<FolderInput strokeWidth={1.5} />} label={t('session.moveToProject')}>{projects.map(p => (
      <DropdownMenu.Item key={p.cwd} title={p.cwd} onClick={() => onMoveTo(p.cwd)}>
        <OptionContent icon={<Folder strokeWidth={1.5} />}>
          {p.name}
          {/* As faint as the project tag on a list row */}
          {p.hint && <span className="ml-gap text-fg-3/70">{p.hint}</span>}
        </OptionContent>
        {/* The same "current" badge as the project group header */}
        {p.current && <span className="shrink-0 rounded-sm bg-active px-1 text-3 text-fg-3">{t('session.project.current')}</span>}
      </DropdownMenu.Item>
    ))}</Submenu>);
  } else if (onMove) {
    top.push(<DropdownMenu.Item key="move" onClick={onMove}><OptionContent icon={<FolderInput strokeWidth={1.5} />}>{t('session.move')}</OptionContent></DropdownMenu.Item>);
  }
  const middle: ReactElement[] = [];
  if (onExport) {
    middle.push(<DropdownMenu.Item key="md" onClick={() => onExport('markdown')}><OptionContent icon={<FileText strokeWidth={1.5} />}>{t('session.export.markdown')}</OptionContent></DropdownMenu.Item>);
    middle.push(<DropdownMenu.Item key="json" onClick={() => onExport('json')}><OptionContent icon={<FileJson strokeWidth={1.5} />}>{t('session.export.json')}</OptionContent></DropdownMenu.Item>);
  }
  return [
    ...top,
    ...top.length > 0 && middle.length > 0 ? [separator('sep-top')] : [],
    ...middle,
    separator('sep-delete'),
    <DropdownMenu.Item key="delete" className="text-danger" onClick={onDelete}><OptionContent icon={<Trash2 strokeWidth={1.5} />}>{t('common.delete')}</OptionContent></DropdownMenu.Item>,
  ];
}

// The session "…" menu, opened from its trigger button
export function SessionMenu({ trigger, align = 'start', onOpenChange, ...items }: SessionMenuProps) {
  return (
    <DropdownMenu.Root onOpenChange={onOpenChange}>
      <DropdownMenu.Trigger render={trigger} />
      <DropdownMenu.Portal><DropdownMenu.Positioner side="bottom" align={align} width="md"><DropdownMenu.Popup>
        {sessionMenuItems(items)}
      </DropdownMenu.Popup></DropdownMenu.Positioner></DropdownMenu.Portal>
    </DropdownMenu.Root>
  );
}

// The same menu on right-click (or long press) of a list row, replacing the webview's native Cut / Copy / Paste. The row itself
// becomes the trigger (no wrapper element), and the menu opens at the pointer
export function SessionContextMenu({ children, onOpenChange, ...items }: SessionMenuItemsProps & { children: ReactElement<Record<string, unknown>>; onOpenChange?: (open: boolean) => void }) {
  return (
    <ContextMenu.Root onOpenChange={open => onOpenChange?.(open)}>
      <ContextMenu.Trigger render={children} />
      {/* Portaled, but React events still bubble up the component tree: a menu click must not reach the row and select it */}
      <DropdownMenu.Portal><DropdownMenu.Positioner width="md"><DropdownMenu.Popup onClick={e => e.stopPropagation()} onKeyDown={e => e.stopPropagation()}>
        {sessionMenuItems(items)}
      </DropdownMenu.Popup></DropdownMenu.Positioner></DropdownMenu.Portal>
    </ContextMenu.Root>
  );
}
