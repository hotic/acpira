import { useEffect, useRef, useState, type ReactNode } from 'react';
import { ArrowLeft, ArrowRight, Check, ChevronRight, CircleAlert, Columns3, Copy, FolderOpen, FolderPlus, Info, Languages, LifeBuoy, Menu, Minus, Palette, PanelLeft, PanelRight, Settings, Settings2, Square, SunMoon, X } from 'lucide-react';
import { DESKTOP_LAYOUTS, PANEL_SCOPES, THEMES, type DesktopLayout, type PanelScope, type ThemeSetting } from '@acpira/shared/settings';
import { LANGUAGES, type Language } from '@acpira/shared/i18n';
import { t } from '../i18n';
import { Button, IconButton } from '../ui/Button';
import { DropdownMenu } from '../ui/DropdownMenu';
import { OptionContent } from '../ui/Panel';
import { Toast } from '../ui/Toast';
import type { DesktopMenuEntry, WindowControl } from '@acpira/shared/protocol';
import { dropShellToast, pageCaption, postMenuCommand, postUpdate, postWindowControl, shortcutLabel, useDesktopMenu, useShellToasts, useWindowMaximized, windowsOS } from '../desktop';
import { vscodeApi } from '../vscodeApi';
import type { SettingsPage } from '../settings/Nav';

// The desktop app's own frame around the shared Shell (desktop.css): the column's title row, its gear menu, and the
// sheet's state when no project is open

const HELP_URL = 'https://github.com/hotic/acpira/issues';
// An error stays up longer than a notice: it is often the only word that a login or an install did not start
const ERROR_TOAST_TTL = 15_000;

// The shell's toasts (packages/ui/desktop.ts) in the stack of whichever page is up; `open` opens the file a toast names, a
// toast's own `action` posts its command back to the shell (restart to update)
export function ShellToastItems({ open }: { open?: (path: string) => void }) {
  const toasts = useShellToasts();
  return <>{toasts.map(s => {
    const file = s.open;
    const command = s.action;
    const action = command
      ? { label: command.label, onClick: () => { postUpdate({ type: command.command }); dropShellToast(s.key); } }
      : file && open ? { label: file.label, onClick: () => { open(file.path); dropShellToast(s.key); } } : undefined;
    return (
      <Toast key={s.key} text={s.text} ttl={s.level === 'error' || command ? ERROR_TOAST_TTL : undefined}
        icon={s.level === 'error' ? <CircleAlert className="size-icon shrink-0 text-danger" strokeWidth={1.5} /> : <Info className="size-icon shrink-0 text-fg-3" strokeWidth={1.5} />}
        action={action}
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
  return <>
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
      {pageCaption && <AppMenus />}
    </div>
    {pageCaption && <WindowCaption />}
  </>;
}

// The menu bar of a window without a global one (Windows, Linux): one button after the column's toggle, the menus as its
// submenus, as VS Code's compact menu (the column is too narrow for four labels beside back, forward and the toggle).
// The shell sends the tree (desktopMenu) and runs what is picked (menuCommand); the Edit items act on the field that had
// the focus before the menu opened. An item's shortcut works while the menu is closed too, except the page's own, which
// its keys already run
const EDIT: Record<string, string> = { 'edit.undo': 'undo', 'edit.redo': 'redo', 'edit.cut': 'cut', 'edit.copy': 'copy', 'edit.delete': 'delete', 'edit.selectAll': 'selectAll' };
function AppMenus() {
  const menus = useDesktopMenu();
  const focused = useRef<Element | null>(null);
  useMenuKeys(menus);
  if (!menus.length) return null;
  const run = (id: string) => {
    const field = focused.current;
    if (!id.startsWith('edit.')) { postMenuCommand(id); return; }
    if (field instanceof HTMLElement) field.focus();
    if (id === 'edit.paste') {
      void vscodeApi().readClipboard?.().then(text => { if (text) document.execCommand('insertText', false, text); });
      return;
    }
    const command = EDIT[id];
    if (command) document.execCommand(command);
  };
  const entries = (items: DesktopMenuEntry[]) => items.map((e, i) => {
    if (e.kind === 'separator') return <DropdownMenu.Separator key={i} className="my-1 h-px bg-line" />;
    if (e.kind === 'submenu') return (
      <DropdownMenu.SubmenuRoot key={i}>
        <DropdownMenu.SubmenuTrigger>
          <OptionContent>{e.label}</OptionContent>
          <ChevronRight className="size-3 shrink-0 text-fg-3" strokeWidth={1.5} />
        </DropdownMenu.SubmenuTrigger>
        <DropdownMenu.Portal><DropdownMenu.Positioner side="inline-end" align="start" width="md"><DropdownMenu.Popup>
          {entries(e.items)}
        </DropdownMenu.Popup></DropdownMenu.Positioner></DropdownMenu.Portal>
      </DropdownMenu.SubmenuRoot>
    );
    return (
      <DropdownMenu.Item key={i} onClick={() => run(e.id)}>
        <OptionContent icon={e.checked === undefined ? undefined : e.checked ? <Check strokeWidth={1.5} /> : <span />}>{e.label}</OptionContent>
        {e.key && <span className="shrink-0 text-3 text-fg-3">{e.key}</span>}
      </DropdownMenu.Item>
    );
  });
  return (
    <DropdownMenu.Root modal={false} onOpenChange={open => { if (open) focused.current = document.activeElement; }}>
      <DropdownMenu.Trigger render={<IconButton data-app-menus title={t('desktop.menu.bar')} aria-label={t('desktop.menu.bar')} className="data-[popup-open]:bg-hover data-[popup-open]:text-fg-1"><Menu strokeWidth={1.5} /></IconButton>} />
      <DropdownMenu.Portal><DropdownMenu.Positioner side="bottom" align="start" width="sm"><DropdownMenu.Popup>
        {menus.map((m, i) => (
          <DropdownMenu.SubmenuRoot key={i}>
            <DropdownMenu.SubmenuTrigger>
              <OptionContent>{m.label}</OptionContent>
              <ChevronRight className="size-3 shrink-0 text-fg-3" strokeWidth={1.5} />
            </DropdownMenu.SubmenuTrigger>
            <DropdownMenu.Portal><DropdownMenu.Positioner side="inline-end" align="start" width="md"><DropdownMenu.Popup>
              {entries(m.items)}
            </DropdownMenu.Popup></DropdownMenu.Positioner></DropdownMenu.Portal>
          </DropdownMenu.SubmenuRoot>
        ))}
      </DropdownMenu.Popup></DropdownMenu.Positioner></DropdownMenu.Portal>
    </DropdownMenu.Root>
  );
}

// "Ctrl+Shift+N" as the key event it names
function matches(key: string, e: KeyboardEvent): boolean {
  const parts = key.split('+');
  const name = parts.pop()!;
  const mods = new Set(parts);
  if (e.ctrlKey !== mods.has('Ctrl') || e.altKey !== mods.has('Alt') || e.shiftKey !== mods.has('Shift') || e.metaKey) return false;
  return name.length === 1 ? e.key.toUpperCase() === name.toUpperCase() || e.code === `Digit${name}` : e.key === name;
}

// The shortcuts of the shell's own items (new window, reload, zoom …): the menu bar took them before the page on macOS,
// here the page hands them to the shell
function useMenuKeys(menus: { items: DesktopMenuEntry[] }[]) {
  useEffect(() => {
    const keyed: { id: string; key: string }[] = [];
    const walk = (items: DesktopMenuEntry[]) => {
      for (const e of items) {
        if (e.kind === 'submenu') walk(e.items);
        else if (e.kind === 'item' && e.key && !e.page) keyed.push({ id: e.id, key: e.key });
      }
    };
    for (const m of menus) walk(m.items);
    if (!keyed.length) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.isComposing || e.defaultPrevented) return;
      const hit = keyed.find(k => matches(k.key, e));
      if (!hit) return;
      e.preventDefault();
      postMenuCommand(hit.id);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [menus]);
}

// The window's minimize, maximize / restore and close, drawn as Windows 11 draws them (desktop.css .win-caption): the
// window has no system title bar, so these stand in the top-right corner where its own buttons would, in the system's
// glyph font. Linux has no such font: the same buttons carry lucide's glyphs. A drag of the title row and its double
// click stay the system's (init.js)
const CAPTION_GLYPH = { minimize: '\uE921', maximize: '\uE922', restore: '\uE923', close: '\uE8BB' };
const CAPTION_ICON = { minimize: Minus, maximize: Square, restore: Copy, close: X };
function WindowCaption() {
  const maximized = useWindowMaximized();
  const button = (op: WindowControl, label: string, glyph: keyof typeof CAPTION_GLYPH) => {
    const Icon = CAPTION_ICON[glyph];
    return (
      <button type="button" data-op={op} title={label} aria-label={label} onClick={() => postWindowControl(op)}>
        {windowsOS ? CAPTION_GLYPH[glyph] : <Icon strokeWidth={1.75} />}
      </button>
    );
  };
  return (
    <div className="win-caption" data-window-caption>
      {button('minimize', t('desktop.menu.minimize'), 'minimize')}
      {maximized
        ? button('maximize', t('desktop.window.restore'), 'restore')
        : button('maximize', t('desktop.window.maximize'), 'maximize')}
      {button('close', t('desktop.menu.closeWindow'), 'close')}
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
  panelScope?: PanelScope;
  panelFiles?: boolean;
  anyPanelModule?: boolean;
  set: <K extends 'theme' | 'language' | 'desktopLayout' | 'panelScope'>(
    key: K,
    value: K extends 'theme' ? ThemeSetting : K extends 'language' ? Language : K extends 'desktopLayout' ? DesktopLayout : PanelScope,
  ) => void;
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
            {quick.panelFiles !== false && (
              <QuickChoice icon={<Columns3 strokeWidth={1.5} />} label={t('settings.desktopLayout')} value={quick.layout} options={DESKTOP_LAYOUTS}
                labelOf={v => t(`settings.desktopLayout.${v}`)} onPick={v => quick.set('desktopLayout', v)} />
            )}
            {quick.anyPanelModule !== false && (
              <QuickChoice icon={<PanelRight strokeWidth={1.5} />} label={t('settings.panelScope')} value={quick.panelScope ?? 'project'} options={PANEL_SCOPES}
                labelOf={v => t(`settings.panelScope.${v}`)} onPick={v => quick.set('panelScope', v)} />
            )}
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
