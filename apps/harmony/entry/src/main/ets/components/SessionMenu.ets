import { SessionSummary } from '../wire/Wire';
import { AppModel } from '../store/AppModel';
import { ExportFormat, SessionCopy } from '../store/SessionLists';
import { folderLabel, folderSubtitle, isUnread, Places } from '../store/SessionLogic';
import { t } from '../store/L10n';
import { MenuBack, MenuCard, MenuGap, MenuRow } from './Menu';
import { confirmDestructive, openTextPrompt, PromptArgs } from './Dialogs';

// The session's "…" menu (apps/ios Design/NativeChrome.swift NativeSessionMenu, its actions Chat/ChatScreen.swift
// HeaderActions; apps/android ui/SessionMenu.kt): the account and the session's changes, pin, unread and rename, copy /
// move / export in submenus, the chat's own pages, archive and delete. A list row's long press shows it too, without the
// chat's own entries and without delete, which the row's swipe offers.

/** The chat's own entries of its "…" menu; undefined ones are left out (a list row passes none) */
export class ChatEntries {
  account: (() => void) | undefined = undefined;
  /** The files the session changed (panel.changes) */
  changes: (() => void) | undefined = undefined;
  /** The session's subagent graph */
  subagents: (() => void) | undefined = undefined;
  terminal: (() => void) | undefined = undefined;
  settings: (() => void) | undefined = undefined;
  /** Delete, asked first; the list's swipe deletes without */
  delete: boolean = false;
}

const COPY: string = 'copy';
const EXPORT: string = 'export';
const MOVE: string = 'move';
/** A machine's folders under the move menu: this prefix and the host's alias ("" this computer) */
const FOLDERS: string = 'move:';

@ComponentV2
export struct SessionMenu {
  @Require @Param model: AppModel;
  @Param sessionId: string = '';
  @Param chat: ChatEntries | undefined = undefined;
  /** The menu closes (a pick, or a dialog it raised) */
  @Event onClose: () => void = () => {};
  /** After an archive (the chat leaves for the list) */
  @Event onArchived: () => void = () => {};
  /** Draw a level by default (screenshots: the Previewer cannot tap) */
  @Param start: string = '';
  @Local level: string = '';

  aboutToAppear(): void {
    this.level = this.start;
  }

  pick(action: () => void): void {
    this.onClose();
    action();
  }

  summary(): SessionSummary | undefined {
    return this.model.summaries.find((s: SessionSummary) => s.id === this.sessionId);
  }

  cwd(): string {
    return this.model.heldView(this.sessionId)?.cwd ?? this.summary()?.cwd ?? '';
  }

  /** The folders the session can move to: its own first unless it is a chat, then the known projects */
  places(): Places {
    const cwd = this.cwd();
    const own: string[] = this.model.isChat(cwd) ? [] : [cwd];
    return Places.of(own.concat(this.model.lists.projects()));
  }

  rename(): void {
    const ui = this.getUIContext();
    const lists = this.model.lists;
    const id = this.sessionId;
    const args = new PromptArgs();
    args.title = t('common.rename');
    args.initial = lists.titleOf(id);
    args.placeholder = t('session.titleAria');
    args.onSave = (text: string) => lists.rename(id, text);
    this.pick(() => openTextPrompt(ui, args));
  }

  addFolder(): void {
    const ui = this.getUIContext();
    const model = this.model;
    const id = this.sessionId;
    const args = new PromptArgs();
    args.title = t('mobile.projects.typePath');
    args.hint = t('mobile.projects.pathHint');
    args.placeholder = t('mobile.projects.path');
    args.save = t('mobile.add');
    args.onSave = (text: string) => {
      model.addProject(text);
      model.lists.move(id, text.trim());
    };
    this.pick(() => openTextPrompt(ui, args));
  }

  remove(): void {
    const ui = this.getUIContext();
    const lists = this.model.lists;
    const id = this.sessionId;
    const title = new Map<string, string>([['title', lists.titleOf(id)]]);
    this.pick(() => confirmDestructive(ui, t('mobile.deleteConfirm', title), t('common.delete'), () => lists.delete(id)));
  }

  build() {
    MenuCard() {
      if (this.level === '') {
        this.top()
      } else if (this.level === COPY) {
        this.copyLevel()
      } else if (this.level === EXPORT) {
        this.exportLevel()
      } else if (this.level === MOVE) {
        this.moveLevel()
      } else {
        this.foldersLevel(this.level.substring(FOLDERS.length))
      }
    }
  }

  @Builder
  top() {
    if (this.chat?.account !== undefined || this.chat?.changes !== undefined) {
      if (this.chat?.account !== undefined) {
        MenuRow({ text: t('common.account'), icon: 'userRound', onTap: () => this.pick(this.chat?.account ?? (() => {})) })
      }
      if (this.chat?.changes !== undefined) {
        MenuRow({ text: t('panel.changes'), icon: 'filePen', onTap: () => this.pick(this.chat?.changes ?? (() => {})) })
      }
      MenuGap()
    }
    MenuRow({
      text: t(this.summary()?.pinned === true ? 'common.unpin' : 'common.pin'),
      icon: this.summary()?.pinned === true ? 'pinOff' : 'pin',
      tag: 'menu.pin',
      onTap: () => this.pick(() => this.model.lists.pin(this.sessionId))
    })
    MenuRow({
      text: t(this.isUnreadNow() ? 'session.markRead' : 'session.markUnread'),
      icon: this.isUnreadNow() ? 'mailCheck' : 'mail',
      tag: 'menu.unread',
      onTap: () => {
        const unread = this.isUnreadNow();
        this.pick(() => this.model.lists.setUnread(this.sessionId, !unread));
      }
    })
    MenuRow({ text: t('common.rename'), icon: 'pencil', tag: 'menu.rename', onTap: () => this.rename() })
    MenuRow({ text: t('session.copy'), icon: 'chevronRight', tag: 'menu.copy', onTap: () => { this.level = COPY; } })
    MenuRow({ text: t('session.moveToProject'), icon: 'chevronRight', tag: 'menu.move', onTap: () => { this.level = MOVE; } })
    MenuRow({ text: t('session.export'), icon: 'chevronRight', tag: 'menu.export', onTap: () => { this.level = EXPORT; } })
    if (this.chat?.subagents !== undefined || this.chat?.terminal !== undefined || this.chat?.settings !== undefined) {
      MenuGap()
      if (this.chat?.subagents !== undefined) {
        MenuRow({ text: t('subagents.graph'), icon: 'workflow', onTap: () => this.pick(this.chat?.subagents ?? (() => {})) })
      }
      if (this.chat?.terminal !== undefined) {
        MenuRow({ text: t('panel.terminal'), icon: 'squareTerminal', onTap: () => this.pick(this.chat?.terminal ?? (() => {})) })
      }
      if (this.chat?.settings !== undefined) {
        MenuRow({ text: t('session.settings'), icon: 'settings', onTap: () => this.pick(this.chat?.settings ?? (() => {})) })
      }
    }
    // Archive keeps the session, delete is the one with no way back
    MenuGap()
    MenuRow({
      text: t(this.summary()?.archived === true ? 'session.unarchive' : 'session.archive'),
      icon: this.summary()?.archived === true ? 'archiveRestore' : 'archive',
      tag: 'menu.archive',
      onTap: () => {
        const archived = this.summary()?.archived !== true;
        this.pick(() => {
          this.model.lists.setArchived(this.sessionId, archived);
          if (archived) {
            this.onArchived();
          }
        });
      }
    })
    if (this.chat?.delete === true) {
      MenuRow({ text: t('session.deleteForever'), icon: 'trash2', danger: true, tag: 'menu.delete', onTap: () => this.remove() })
    }
  }

  isUnreadNow(): boolean {
    const s = this.summary();
    return s !== undefined && isUnread(s);
  }

  @Builder
  copyLevel() {
    MenuBack({ title: t('session.copy'), onTap: () => { this.level = ''; } })
    MenuGap()
    MenuRow({ text: t('session.copy.link'), icon: 'link', tag: 'menu.copy.link', onTap: () => this.pick(() => this.model.lists.copy(this.sessionId, SessionCopy.Link)) })
    MenuRow({
      text: t('session.copy.markdown'), icon: 'fileText', tag: 'menu.copy.markdown',
      onTap: () => this.pick(() => this.model.lists.copy(this.sessionId, SessionCopy.Markdown))
    })
    if (this.cwd().length > 0) {
      MenuRow({ text: t('session.copy.cwd'), icon: 'folder', tag: 'menu.copy.cwd', onTap: () => this.pick(() => this.model.lists.copy(this.sessionId, SessionCopy.Cwd)) })
    }
  }

  @Builder
  exportLevel() {
    MenuBack({ title: t('session.export'), onTap: () => { this.level = ''; } })
    MenuGap()
    MenuRow({
      text: t('session.export.markdown'), icon: 'fileText', tag: 'menu.export.markdown',
      onTap: () => this.pick(() => this.model.lists.export(this.sessionId, ExportFormat.Markdown))
    })
    // Built from the view the phone holds, which a list row's session may not have
    if (this.model.lists.holdsView(this.sessionId)) {
      MenuRow({ text: t('session.export.json'), icon: 'braces', tag: 'menu.export.json', onTap: () => this.pick(() => this.model.lists.export(this.sessionId, ExportFormat.Json)) })
    }
    MenuRow({
      text: t('session.export.text'), icon: 'textAlignStart', tag: 'menu.export.text',
      onTap: () => this.pick(() => this.model.lists.export(this.sessionId, ExportFormat.Text))
    })
  }

  @Builder
  moveLevel() {
    MenuBack({ title: t('session.moveToProject'), onTap: () => { this.level = ''; } })
    MenuGap()
    if (this.places().local.length > 0) {
      MenuRow({ text: t('mobile.projects.local'), icon: 'chevronRight', tag: 'menu.move.local', onTap: () => { this.level = FOLDERS; } })
    }
    ForEach(this.places().hosts, (h: string) => {
      MenuRow({ text: h, subtitle: t('mobile.projects.remote'), icon: 'chevronRight', onTap: () => { this.level = FOLDERS + h; } })
    }, (h: string) => h)
    if (this.places().local.length > 0 || this.places().hosts.length > 0) {
      MenuGap()
    }
    MenuRow({ text: t('mobile.projects.add'), icon: 'folderPlus', tag: 'menu.move.add', onTap: () => this.addFolder() })
  }

  /** One machine's folders; the session's own is ticked, so picking it moves nothing */
  @Builder
  foldersLevel(host: string) {
    MenuBack({ title: host.length === 0 ? t('mobile.projects.local') : host, onTap: () => { this.level = MOVE; } })
    MenuGap()
    ForEach(host.length === 0 ? this.places().local : (this.places().byHost.get(host) ?? []), (p: string) => {
      MenuRow({
        text: folderLabel(p),
        subtitle: folderSubtitle(p),
        checked: !this.model.isChat(this.cwd()) && p === this.cwd(),
        icon: host.length === 0 ? 'folder' : 'globe',
        tag: `menu.move.to:${p}`,
        onTap: () => this.pick(() => {
          if (p !== this.cwd()) {
            this.model.lists.move(this.sessionId, p);
          }
        })
      })
    }, (p: string) => p)
  }
}
