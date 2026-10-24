import { useEffect, useRef, useState, type ReactNode } from 'react';
import { ArrowLeft, ArrowRight, ChevronRight, CircleAlert, Columns3, FolderOpen, FolderPlus, Info, Languages, LifeBuoy, Palette, PanelLeft, Settings, Settings2, SunMoon } from 'lucide-react';
import { DESKTOP_LAYOUTS, THEMES, type DesktopLayout, type ThemeSetting } from '@acpira/shared/settings';
import { LANGUAGES, type Language } from '@acpira/shared/i18n';
import { t } from '../i18n';
import { Button, IconButton } from '../ui/Button';
import { DropdownMenu } from '../ui/DropdownMenu';
import { OptionContent } from '../ui/Panel';
import { Toast } from '../ui/Toast';
import { dropShellToast, shortcutLabel, useShellToasts } from '../desktop';
import type { SettingsPage } from '../settings/Nav';

// The desktop app's own frame around the shared Shell (desktop.css): the column's title row, its gear menu, and the
// sheet's state when no project is open

const HELP_URL = 'https://github.com/hotic/acpira/issues';
// An error stays up longer than a notice: it is often the only word that a login or an install did not start
const ERROR_TOAST_TTL = 15_000;

// The shell's toasts (packages/ui/desktop.ts) in the stack of whichever page is up; `open` opens the file a toast names
export function ShellToastItems({ open }: { open?: (path: string) => void }) {
  const toasts = useShellToasts();
  return <>{toasts.map(s => {
    const file = s.open;
    return (
      <Toast key={s.key} text={s.text} ttl={s.level === 'error' ? ERROR_TOAST_TTL : undefined}
        icon={s.level === 'error' ? <CircleAlert className="size-icon shrink-0 text-danger" strokeWidth={1.5} /> : <Info className="size-icon shrink-0 text-fg-3" strokeWidth={1.5} />}
        action={file && open ? { label: file.label, onClick: () => { open(file.path); dropShellToast(s.key); } } : undefined}
        onClose={() => dropShellToast(s.key)} />
    );
  })}</>;
}

// The settings page's stack for them, over the foot of its sheet (the chat keeps them in its own stack above the composer)
export function ShellToasts({ open }: { open?: (path: string) => void }) {
  const toasts = useShellToasts();
  if (!toasts.length) return null;
  return (
    <div className="pointer-events-none absolute inset-x-0 bottom-gap z-10 flex flex-col items-center gap-1 px-page">
      <ShellToastItems open={open} />
    </div>
  );
}

const STEP = 'disabled:pointer-events-none disabled:opacity-40';

// The window's title row, pinned at its top-left whatever the column does (desktop.css): after the traffic lights (macOS),
// back / forward, then the column's toggle. Back and forward stay put across pages: the settings page shows the same row
// without the toggle, its back returning to the chat. The column and the settings rail leave a strip under it (TitleSpace),
// and with no column beside the sheet its header starts after the row (--title-w). It moves the window like a title bar.
// A narrow window (the column a page over it) keeps the lights and the toggle only, so the header has the room; back and
// forward stay on ⌘[ / ⌘] there. The toggle reads as pressed while that page is open
export function TitleRow({ onBack, onForward, column }: {
  onBack?: () => void;
  onForward?: () => void;
  // The column's toggle: hide / show a docked column, open / close the list page of a narrow window
  column?: { open: boolean; drawer: boolean; onToggle: () => void };
}) {
  const label = column && t(column.open ? 'desktop.hideSidebar' : 'desktop.showSidebar');
  return (
    <div data-title-row data-drag-region className="absolute top-0 left-0 z-20 flex h-hdr items-center gap-0.5">
      {!column?.drawer && <>
        <IconButton title={`${t('desktop.back')} ${shortcutLabel('[')}`} aria-label={t('desktop.back')} disabled={!onBack} onClick={onBack} className={STEP}><ArrowLeft strokeWidth={1.5} /></IconButton>
        <IconButton title={`${t('desktop.forward')} ${shortcutLabel(']')}`} aria-label={t('desktop.forward')} disabled={!onForward} onClick={onForward} className={STEP}><ArrowRight strokeWidth={1.5} /></IconButton>
      </>}
      {column && (
        <IconButton data-session-toggle={column.drawer || undefined} aria-expanded={column.drawer ? column.open : undefined} title={label} aria-label={label} onClick={column.onToggle}
          className="aria-expanded:bg-active aria-expanded:text-fg-1">
          <PanelLeft strokeWidth={1.5} />
        </IconButton>
      )}
    </div>
  );
}

// The strip a column or the settings rail leaves under the pinned title row; it moves the window too
export function TitleSpace() {
  return <div data-title-space data-drag-region className="h-hdr shrink-0" />;
}

// The gear at the column's foot: settings, the theme and language as quick switches, and help in one menu, instead of a
// full-width settings row. Built like a row's
// lead (its padding, then the lead slot), so the gear stands on the same line as the rows' icons at every density. Its glyph
// is a size up from a row's (--foot-icon) and its hover fill square, the weight Codex and VS Code give their foot gear: at
// the rows' size it read as a stray mark under the list (reported 2026-10-23)
const GEAR = 'flex aspect-square items-center rounded-md px-2 text-fg-2 transition-colors hover:bg-hover hover:text-fg-1 focus-visible:bg-hover data-[popup-open]:bg-hover data-[popup-open]:text-fg-1';
// The settings worth a quick switch without opening the page: the theme, the interface language and the window's layout
export interface QuickSettings {
  theme: ThemeSetting;
  language: Language;
  layout: DesktopLayout;
  set: <K extends 'theme' | 'language' | 'desktopLayout'>(key: K, value: K extends 'theme' ? ThemeSetting : K extends 'language' ? Language : DesktopLayout) => void;
}

// One quick setting: a row naming its current value that opens the choices beside the menu, the current one ticked
// A pick leaves both menus open, the way switching a model does: the tick moves and the page changes behind them
function QuickChoice<V extends string>({ icon, label, value, options, labelOf, onPick }: {
  icon: ReactNode; label: string; value: V; options: readonly V[]; labelOf: (v: V) => string; onPick: (v: V) => void;
}) {
  return (
    <DropdownMenu.SubmenuRoot>
      <DropdownMenu.SubmenuTrigger>
        <OptionContent icon={icon}>{label}</OptionContent>
        <span className="shrink-0 truncate text-3 text-fg-3">{labelOf(value)}</span>
        <ChevronRight className="size-3 shrink-0 text-fg-3" strokeWidth={1.5} />
      </DropdownMenu.SubmenuTrigger>
      <DropdownMenu.Portal><DropdownMenu.Positioner side="inline-end" align="end" width="sm"><DropdownMenu.Popup>
        <div className="scroll-thin flex max-h-pop flex-col overflow-y-auto">
          {options.map(v => <DropdownMenu.Item key={v} closeOnClick={false} onClick={() => onPick(v)}><OptionContent checked={v === value} checkSlot>{labelOf(v)}</OptionContent></DropdownMenu.Item>)}
        </div>
      </DropdownMenu.Popup></DropdownMenu.Positioner></DropdownMenu.Portal>
    </DropdownMenu.SubmenuRoot>
  );
}

export function AppMenu({ onOpenSettings, onOpenExternal, quick }: { onOpenSettings: (page?: SettingsPage) => void; onOpenExternal?: (url: string) => void; quick?: QuickSettings }) {
  return (
    <div className="flex shrink-0 items-center p-gap-half">
      <DropdownMenu.Root>
        <DropdownMenu.Trigger render={<button type="button" title={t('desktop.appMenu')} aria-label={t('desktop.appMenu')} className={GEAR}>
          <span className="flex size-lead items-center justify-center"><Settings className="size-(--foot-icon)" strokeWidth={1.5} /></span>
        </button>} />
        {/* No collision padding: the gear sits closer to the window's edge than the shell's pad, which shifted the menu off it */}
        <DropdownMenu.Portal><DropdownMenu.Positioner side="top" align="start" width="md" collisionPadding={0}><DropdownMenu.Popup>
          <DropdownMenu.Item onClick={() => onOpenSettings()}>
            <OptionContent icon={<Settings2 strokeWidth={1.5} />}>{t('session.settings')}</OptionContent>
            <span className="shrink-0 text-3 text-fg-3">{shortcutLabel(',')}</span>
          </DropdownMenu.Item>
          <DropdownMenu.Item onClick={() => onOpenSettings({ kind: 'appearance' })}>
            <OptionContent icon={<Palette strokeWidth={1.5} />}>{t('settings.nav.appearance')}</OptionContent>
          </DropdownMenu.Item>
          {quick && <>
            <DropdownMenu.Separator className="my-1 h-px bg-line" />
            {/* `auto` follows the system here: nativeTheme for the theme, app.getLocale for the language */}
            <QuickChoice icon={<SunMoon strokeWidth={1.5} />} label={t('settings.theme')} value={quick.theme} options={THEMES}
              labelOf={v => t(v === 'auto' ? 'settings.theme.system' : `settings.theme.${v}` as const)} onPick={v => quick.set('theme', v)} />
            <QuickChoice icon={<Languages strokeWidth={1.5} />} label={t('settings.language')} value={quick.language} options={LANGUAGES}
              labelOf={v => t(v === 'auto' ? 'settings.language.system' : `settings.language.${v}` as const)} onPick={v => quick.set('language', v)} />
            <QuickChoice icon={<Columns3 strokeWidth={1.5} />} label={t('settings.desktopLayout')} value={quick.layout} options={DESKTOP_LAYOUTS}
              labelOf={v => t(`settings.desktopLayout.${v}`)} onPick={v => quick.set('desktopLayout', v)} />
          </>}
          {onOpenExternal && <>
            <DropdownMenu.Separator className="my-1 h-px bg-line" />
            <DropdownMenu.Item onClick={() => onOpenExternal(HELP_URL)}>
              <OptionContent icon={<LifeBuoy strokeWidth={1.5} />}>{t('desktop.help')}</OptionContent>
            </DropdownMenu.Item>
          </>}
        </DropdownMenu.Popup></DropdownMenu.Positioner></DropdownMenu.Portal>
      </DropdownMenu.Root>
    </div>
  );
}

// No project yet (the engine answered noSession): a session needs a folder to run in, so the sheet asks for one
export function NoProject({ onOpen }: { onOpen?: () => void }) {
  return (
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-pad px-page text-center">
      <FolderOpen className="size-8 text-fg-3" strokeWidth={1.25} />
      <div className="flex max-w-pop-xl flex-col gap-1">
        <div className="text-1 font-medium text-fg-strong">{t('desktop.empty.title')}</div>
        <div className="text-2 text-fg-3">{t('desktop.empty.text')}</div>
      </div>
      {onOpen && (
        <div className="flex flex-col items-center gap-gap">
          <Button variant="primary" onClick={onOpen}><FolderPlus className="size-icon" strokeWidth={1.75} />{t('desktop.empty.open')}</Button>
          <span className="text-3 text-fg-3">{t('desktop.empty.drop')}</span>
        </div>
      )}
    </div>
  );
}

// Back / forward through the sessions the window showed, as a browser keeps its pages: going somewhere new drops the
// forward half. A session deleted since is stepped over
export function useSessionHistory(active: string | undefined, alive: (id: string) => boolean, select: (id: string) => void) {
  const [trail, setTrail] = useState<{ stack: string[]; at: number }>({ stack: [], at: -1 });
  // The step under way: the view lands on it without starting a new branch
  const travelling = useRef<number | undefined>(undefined);
  useEffect(() => {
    if (!active) return;
    const step = travelling.current;
    travelling.current = undefined;
    setTrail(s => {
      if (s.stack[s.at] === active) return s;
      if (step !== undefined && s.stack[step] === active) return { stack: s.stack, at: step };
      const stack = [...s.stack.slice(0, s.at + 1), active];
      return { stack, at: stack.length - 1 };
    });
  }, [active]);
  const go = (dir: -1 | 1) => {
    for (let i = trail.at + dir; i >= 0 && i < trail.stack.length; i += dir) {
      const id = trail.stack[i]!;
      if (id === active || !alive(id)) continue;
      return () => { travelling.current = i; select(id); };
    }
    return undefined;
  };
  return { back: go(-1), forward: go(1) };
}
