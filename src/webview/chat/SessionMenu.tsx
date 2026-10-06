import type { ReactElement } from 'react';
import { ChevronRight, FileJson, FileText, FolderInput, Inbox, Pencil, Pin, PinOff, SquareArrowOutUpRight, Trash2, Undo2 } from 'lucide-react';
import type { SessionCategory, SessionSummary } from '@shared/transcript';
import { t } from '../i18n';
import { DropdownMenu } from '../ui/DropdownMenu';
import { OptionContent } from '../ui/Panel';
import { CategoryAddIcon } from './CategoryIcon';

export interface SessionMenuProps {
  session: SessionSummary;
  // The button that opens the menu (an IconButton); aria-label belongs on it
  trigger: ReactElement;
  align?: 'start' | 'end';
  onOpenChange?: (open: boolean) => void;
  // Header only: open the session in an editor tab
  onOpenInEditor?: () => void;
  // Row only: starts the inline RenameInput
  onRename?: () => void;
  onPin: () => void;
  // "Move to this project", only when the row offers it
  onMove?: () => void;
  onExport?: (format: 'markdown' | 'json') => void;
  onDelete: () => void;
  // "Move to category": the categories of the session's own project, the pick (null = take it out) and "new category…".
  // Offered only with onFile, and never for a pinned session (pinned ones are locked to the top)
  categories?: SessionCategory[];
  onFile?: (category: string | null) => void;
  onNewCategory?: () => void;
}

// The session "…" menu (Cursor's shape): session actions, export, delete — shared by the list row and the header.
// An item only renders when its handler exists; a separator only between two non-empty groups
export function SessionMenu({ session, trigger, align = 'start', onOpenChange, onOpenInEditor, onRename, onPin, onMove, onExport, onDelete, categories = [], onFile, onNewCategory }: SessionMenuProps) {
  const top: ReactElement[] = [];
  if (onOpenInEditor) top.push(<DropdownMenu.Item key="editor" onClick={onOpenInEditor}><OptionContent icon={<SquareArrowOutUpRight strokeWidth={1.5} />}>{t('session.openInEditor')}</OptionContent></DropdownMenu.Item>);
  if (onRename) top.push(<DropdownMenu.Item key="rename" onClick={onRename}><OptionContent icon={<Pencil strokeWidth={1.5} />}>{t('common.rename')}</OptionContent></DropdownMenu.Item>);
  top.push(<DropdownMenu.Item key="pin" onClick={onPin}><OptionContent icon={session.pinned ? <PinOff strokeWidth={1.5} /> : <Pin strokeWidth={1.5} />}>{session.pinned ? t('common.unpin') : t('common.pin')}</OptionContent></DropdownMenu.Item>);
  if (onFile && !session.pinned) {
    const filed = categories.some(c => c.id === session.category);
    top.push(<DropdownMenu.SubmenuRoot key="file">
      <DropdownMenu.SubmenuTrigger>
        <OptionContent icon={<Inbox strokeWidth={1.5} />}>{t('session.category.moveTo')}</OptionContent>
        <ChevronRight className="size-3 shrink-0 text-fg-3" strokeWidth={1.5} />
      </DropdownMenu.SubmenuTrigger>
      <DropdownMenu.Portal><DropdownMenu.Positioner side="inline-start" align="start" width="md"><DropdownMenu.Popup>
        {categories.map(c => (
          <DropdownMenu.Item key={c.id} onClick={() => onFile(c.id)}>
            <OptionContent icon={<Inbox strokeWidth={1.5} />} checked={c.id === session.category} checkSlot={filed}>{c.name}</OptionContent>
          </DropdownMenu.Item>
        ))}
        {categories.length > 0 && <DropdownMenu.Separator className="my-1 h-px bg-line" />}
        {onNewCategory && <DropdownMenu.Item onClick={onNewCategory}><OptionContent icon={<CategoryAddIcon />}>{t('session.category.newEllipsis')}</OptionContent></DropdownMenu.Item>}
        {filed && <DropdownMenu.Item onClick={() => onFile(null)}><OptionContent icon={<Undo2 strokeWidth={1.5} />}>{t('session.category.remove')}</OptionContent></DropdownMenu.Item>}
      </DropdownMenu.Popup></DropdownMenu.Positioner></DropdownMenu.Portal>
    </DropdownMenu.SubmenuRoot>);
  }
  if (onMove) top.push(<DropdownMenu.Item key="move" onClick={onMove}><OptionContent icon={<FolderInput strokeWidth={1.5} />}>{t('session.move')}</OptionContent></DropdownMenu.Item>);
  const middle: ReactElement[] = [];
  if (onExport) {
    middle.push(<DropdownMenu.Item key="md" onClick={() => onExport('markdown')}><OptionContent icon={<FileText strokeWidth={1.5} />}>{t('session.export.markdown')}</OptionContent></DropdownMenu.Item>);
    middle.push(<DropdownMenu.Item key="json" onClick={() => onExport('json')}><OptionContent icon={<FileJson strokeWidth={1.5} />}>{t('session.export.json')}</OptionContent></DropdownMenu.Item>);
  }
  const separator = <DropdownMenu.Separator className="my-1 h-px bg-line" />;
  return (
    <DropdownMenu.Root onOpenChange={onOpenChange}>
      <DropdownMenu.Trigger render={trigger} />
      <DropdownMenu.Portal><DropdownMenu.Positioner side="bottom" align={align} width="md"><DropdownMenu.Popup>
        {top}
        {top.length > 0 && middle.length > 0 && separator}
        {middle}
        <DropdownMenu.Separator className="my-1 h-px bg-line" />
        <DropdownMenu.Item className="text-danger" onClick={onDelete}><OptionContent icon={<Trash2 strokeWidth={1.5} />}>{t('common.delete')}</OptionContent></DropdownMenu.Item>
      </DropdownMenu.Popup></DropdownMenu.Positioner></DropdownMenu.Portal>
    </DropdownMenu.Root>
  );
}
