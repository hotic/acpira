import { app, Menu, type MenuItemConstructorOptions } from 'electron';
import type { MsgKey, Params } from '@shared/i18n';
import { DESKTOP_LAYOUTS, type DesktopLayout } from '@shared/settings';

// The menu bar: the extension's editor-free commands plus the page's own shortcuts. On macOS a menu item's key equivalent
// takes the key before the page sees it, so ⌘N / ⌘K / ⌘O / ⌘, arrive here and go to the page as shellCommand, which runs
// what its own shortcut would. The standard role items carry labels in the app's language too: macOS adds items of its
// own (Quit and Keep Windows, tiling) in the system's language, and Electron's English defaults stood out among them

export interface MenuActions {
  newSession: () => void;
  newWindow: () => void;
  openFolder: () => void;
  search: () => void;
  settings: () => void;
  browser: () => void;
  toolPanel: () => void;
  layout: (layout: DesktopLayout) => void;
  connectChatgpt: () => void;
  showLog: () => void;
  help: () => void;
}

// `view`: the window layout the page shows (acpira.desktopLayout), ticked in View › Layout; the menu is built again when it
// changes, since ⌥⌘B folds the session column instead of the tool panel in the files layout
export function buildMenu(t: (key: MsgKey, params?: Params) => string, a: MenuActions, view: { layout: DesktopLayout }, platform = process.platform): Menu {
  const mac = platform === 'darwin';
  const named = { app: app.name };
  const sep = { type: 'separator' } as const;
  const settings: MenuItemConstructorOptions = { label: t('session.settings'), accelerator: 'CmdOrCtrl+,', click: a.settings };
  const quit: MenuItemConstructorOptions = { role: 'quit', label: t('desktop.menu.quit', named) };
  const template: MenuItemConstructorOptions[] = [
    ...(mac ? [{
      label: app.name,
      submenu: [
        { role: 'about', label: t('desktop.menu.about', named) }, sep, settings, sep,
        { role: 'services', label: t('desktop.menu.services') }, sep,
        { role: 'hide', label: t('desktop.menu.hide', named) }, { role: 'hideOthers', label: t('desktop.menu.hideOthers') }, { role: 'unhide', label: t('desktop.menu.showAll') }, sep,
        quit,
      ],
    } satisfies MenuItemConstructorOptions] : []),
    {
      label: t('desktop.menu.file'),
      submenu: [
        { label: t('session.new'), accelerator: 'CmdOrCtrl+N', click: a.newSession },
        { label: t('desktop.menu.newWindow'), accelerator: 'CmdOrCtrl+Shift+N', click: a.newWindow },
        { label: t('desktop.empty.open'), accelerator: 'CmdOrCtrl+O', click: a.openFolder },
        sep,
        { label: t('desktop.menu.connectChatgpt'), click: a.connectChatgpt },
        sep,
        { role: 'close', label: t('desktop.menu.closeWindow') },
        ...(mac ? [] : [sep, settings, sep, quit]),
      ],
    },
    // Without the Edit roles macOS sends no ⌘C / ⌘V / ⌘Z to the page
    {
      label: t('desktop.menu.edit'),
      submenu: [
        { role: 'undo', label: t('desktop.menu.undo') }, { role: 'redo', label: t('desktop.menu.redo') }, sep,
        { role: 'cut', label: t('desktop.menu.cut') }, { role: 'copy', label: t('desktop.menu.copy') }, { role: 'paste', label: t('desktop.menu.paste') },
        ...(mac ? [{ role: 'pasteAndMatchStyle', label: t('desktop.menu.pasteAndMatchStyle') } as const] : []),
        { role: 'delete', label: t('desktop.menu.delete') }, { role: 'selectAll', label: t('desktop.menu.selectAll') },
        ...(mac ? [sep, {
          label: t('desktop.menu.speech'),
          submenu: [{ role: 'startSpeaking', label: t('desktop.menu.startSpeaking') }, { role: 'stopSpeaking', label: t('desktop.menu.stopSpeaking') }],
        } satisfies MenuItemConstructorOptions] : []),
      ],
    },
    {
      label: t('desktop.menu.view'),
      submenu: [
        { label: t('session.search'), accelerator: 'CmdOrCtrl+K', click: a.search },
        { label: view.layout === 'files' ? t('desktop.sessionList') : t('panel.label'), accelerator: 'Alt+CmdOrCtrl+B', click: a.toolPanel },
        { label: t('desktop.menu.browser'), click: a.browser },
        sep,
        {
          label: t('settings.desktopLayout'),
          submenu: DESKTOP_LAYOUTS.map((l): MenuItemConstructorOptions => ({ label: t(`settings.desktopLayout.${l}`), type: 'radio', checked: l === view.layout, click: () => a.layout(l) })),
        },
        sep,
        { role: 'reload', label: t('desktop.menu.reload') }, { role: 'toggleDevTools', label: t('desktop.menu.devTools') }, sep,
        { role: 'resetZoom', label: t('desktop.menu.actualSize') }, { role: 'zoomIn', label: t('desktop.menu.zoomIn') }, { role: 'zoomOut', label: t('desktop.menu.zoomOut') }, sep,
        { role: 'togglefullscreen', label: t('desktop.menu.fullScreen') },
      ],
    },
    // role 'window' makes it the system's Window menu on macOS, which lists the open windows after these
    {
      label: t('desktop.menu.window'),
      role: 'window',
      submenu: mac
        ? [{ role: 'minimize', label: t('desktop.menu.minimize') }, { role: 'zoom', label: t('desktop.menu.zoom') }, sep, { role: 'front', label: t('desktop.menu.front') }]
        : [{ role: 'minimize', label: t('desktop.menu.minimize') }, { role: 'close', label: t('desktop.menu.closeWindow') }],
    },
    {
      label: t('desktop.menu.help'),
      role: 'help',
      submenu: [
        { label: t('desktop.help'), click: a.help },
        { label: t('desktop.menu.showLog'), click: a.showLog },
      ],
    },
  ];
  return Menu.buildFromTemplate(template);
}
