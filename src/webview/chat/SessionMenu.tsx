import { useState, type ReactElement } from 'react';
import { ContextMenu } from '@base-ui/react/context-menu';
import { ChevronRight, Copy, FileDown, FileJson, FileText, Folder, FolderInput, Inbox, LetterText, Link, Pencil, Pin, PinOff, SquareArrowOutUpRight, Trash2, Undo2 } from 'lucide-react';
import type { ExportFormat } from '@shared/protocol';
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
  onExport?: (format: ExportFormat) => void;
  // "Copy": the session's link, its transcript as Markdown, its working directory (only when the summary knows one)
  onCopy?: (what: SessionCopyKind) => void;
  onDelete: () => void;
  // "Move to category": the categories of the session's own project, the pick (null = take it out) and "new category…".
  // Offered only with onFile, and never for a pinned session (pinned ones are locked to the top)
  categories?: SessionCategory[];
  onFile?: (category: string | null) => void;
  onNewCategory?: () => void;
}

export type SessionCopyKind = 'link' | 'markdown' | 'cwd';

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

// The session menu's rows (Cursor's shape): session actions, copy and export, delete. Shared by the "…" dropdown of the list row and the
// header and by the row's right-click menu; Base UI's context menu is built from the same Menu parts, so the rows are one set.
// An item only renders when its handler exists; a separator only between two non-empty groups
function sessionMenuItems({ session, onOpenInEditor, onRename, onPin, onMove, projects = [], onMoveTo, onExport, onCopy, onDelete, categories = [], onFile, onNewCategory }: SessionMenuItemsProps): ReactElement[] {
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
  // Copy leads the middle group; link and folder are written inside the click, the Markdown is fetched (Shell copySession)
  if (onCopy) {
    middle.push(<Submenu key="copy" icon={<Copy strokeWidth={1.5} />} label={t('session.copy')}>{[
      <DropdownMenu.Item key="link" onClick={() => onCopy('link')}><OptionContent icon={<Link strokeWidth={1.5} />}>{t('session.copy.link')}</OptionContent></DropdownMenu.Item>,
      <DropdownMenu.Item key="markdown" onClick={() => onCopy('markdown')}><OptionContent icon={<FileText strokeWidth={1.5} />}>{t('session.copy.markdown')}</OptionContent></DropdownMenu.Item>,
      ...session.cwd ? [<DropdownMenu.Item key="cwd" title={session.cwd} onClick={() => onCopy('cwd')}><OptionContent icon={<Folder strokeWidth={1.5} />}>{t('session.copy.cwd')}</OptionContent></DropdownMenu.Item>] : [],
    ]}</Submenu>);
  }
  // Export: one submenu like Copy; plain text keeps only what was said (prompts and replies, no tools or thinking)
  if (onExport) {
    middle.push(<Submenu key="export" icon={<FileDown strokeWidth={1.5} />} label={t('session.export')}>{[
      <DropdownMenu.Item key="md" onClick={() => onExport('markdown')}><OptionContent icon={<FileText strokeWidth={1.5} />}>{t('session.export.markdown')}</OptionContent></DropdownMenu.Item>,
      <DropdownMenu.Item key="json" onClick={() => onExport('json')}><OptionContent icon={<FileJson strokeWidth={1.5} />}>{t('session.export.json')}</OptionContent></DropdownMenu.Item>,
      <DropdownMenu.Item key="text" title={t('session.export.textHint')} onClick={() => onExport('text')}><OptionContent icon={<LetterText strokeWidth={1.5} />}>{t('session.export.text')}</OptionContent></DropdownMenu.Item>,
    ]}</Submenu>);
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
  const [open, setOpen] = useState(false);
  return (
    <DropdownMenu.Root open={open} onOpenChange={next => { setOpen(next); onOpenChange?.(next); }}>
      <DropdownMenu.Trigger render={trigger} />
      <DropdownMenu.Portal><DropdownMenu.Positioner side="bottom" align={align} width="md"><DropdownMenu.Popup>
        {open && sessionMenuItems(items)}
      </DropdownMenu.Popup></DropdownMenu.Positioner></DropdownMenu.Portal>
    </DropdownMenu.Root>
  );
}

// The same menu on right-click (or long press) of a list row, replacing the webview's native Cut / Copy / Paste. The row itself
// becomes the trigger (no wrapper element), and the menu opens at the pointer. Like a native context menu it is as wide as its
// rows (at least the small popup width) and flips to the other side of the pointer when it does not fit. At the fixed popup width
// a narrow session column left no room, so the align shift pinned the menu to the same spot wherever the row was clicked.
// Shifting stays as the fallback when neither side fits.
// `editing`: the row is being renamed. The menu stays mounted but disabled (the input keeps the native menu), so the row
// element is not remounted. Closing never returns focus: a context menu has no trigger button to return to, and returning
// focus to the row (or to a row right-clicked earlier) either stole focus from the fresh rename input or left the row's
// focus-within action cluster showing without a hover fill
const contextMenuCollision = { side: 'flip', align: 'flip', fallbackAxisSide: 'none' } as const;
export function SessionContextMenu({ children, onOpenChange, editing, ...items }: SessionMenuItemsProps & { children: ReactElement<Record<string, unknown>>; onOpenChange?: (open: boolean) => void; editing?: boolean }) {
  const [open, setOpen] = useState(false);
  return (
    <ContextMenu.Root disabled={editing} onOpenChange={next => { setOpen(next); onOpenChange?.(next); }}>
      <ContextMenu.Trigger render={children} />
      {/* Portaled, but React events still bubble up the component tree: a menu click must not reach the row and select it */}
      <DropdownMenu.Portal><DropdownMenu.Positioner className="w-max min-w-pop-sm" collisionAvoidance={contextMenuCollision}><DropdownMenu.Popup finalFocus={false} onClick={e => e.stopPropagation()} onKeyDown={e => e.stopPropagation()}>
        {open && sessionMenuItems(items)}
      </DropdownMenu.Popup></DropdownMenu.Positioner></DropdownMenu.Portal>
    </ContextMenu.Root>
  );
}
