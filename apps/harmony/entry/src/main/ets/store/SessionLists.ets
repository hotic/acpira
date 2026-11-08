import { pasteboard } from '@kit.BasicServicesKit';
import { picker, fileIo } from '@kit.CoreFileKit';
import { common } from '@kit.AbilityKit';
import { ListHostMsg, SessionCategories, SessionHit, SessionSection, SessionSummary, SessionView } from '../wire/Wire';
import { WebviewMsg } from '../wire/EngineClient';
import { t } from './L10n';
import { exportFileName, exportJson, exportMarkdown, exportText, projectName, searchSnippet, searchTerms } from './SessionLogic';
import { PrefKey, prefs } from './Prefs';

// The session list's state and the session actions its rows, its menus and the chat's "…" menu post (apps/ios
// AppModel's list, projects and session actions; apps/android store/SessionLists.kt): the grouping prefs, the desktop's
// sections and removed folders, the content search's hits, open projects and the toast's undo. Without an engine the
// actions apply to the demo's list in place.

/** A toast's button: Undo after a delete, an archive or a project taken off the list */
export class ToastAction {
  label: string = '';
  /** The toast it belongs to: a later toast never shows it */
  toast: string = '';
  run: () => void = () => {};
}

/** What the session menu's Copy puts on the clipboard */
export enum SessionCopy {
  Link,
  Markdown,
  Cwd
}

/** The engine's exports, as `sessionMarkdown`'s format names them */
export enum ExportFormat {
  Markdown,
  Json,
  Text
}

/** What the lists need from the app model (store/AppModel.ets implements it) */
export interface ListHost {
  summaries: SessionSummary[];
  projectOrder: string[];
  addedProjects: string[];
  toast: string | undefined;
  readonly live: boolean;
  postLive(message: WebviewMsg): boolean;
  agentName(id: string): string;
  now(): number;
  isChat(cwd: string): boolean;
  /** The view the phone holds for a session: the open chat, or the demo's */
  heldView(id: string): SessionView | undefined;
  /** Every view the demo can search */
  demoViews(): SessionView[];
  appContext(): common.Context | undefined;
  addProject(cwd: string): void;
  /** The chat page on `id` leaves (a deleted or archived session) */
  closeIf(id: string): void;
}

const SSH: string = 'ssh://';
const SESSION_LINK: string = 'acpira://session/';
/** How long an export waits for the engine's answer */
const ASK_TIMEOUT_MS: number = 15000;

class Ask {
  seq: number = 0;
  done: (markdown: string | undefined) => void = () => {};
}

@ObservedV2
export class SessionLists {
  /** time | project | agent | none, kept on the phone */
  @Trace view: string = 'time';
  /** top | project */
  @Trace pinPlace: string = 'top';
  /** The desktop app's sections, each holding whole projects in its own order */
  @Trace sections: SessionSection[] = [];
  /** Folders taken off the list, shared with every project shell on the machine */
  @Trace removedProjects: string[] = [];
  /** The history search's content hits (id → snippet), for the query they answered */
  @Trace hits: Map<string, string> | undefined = undefined;
  @Trace hitsQuery: string = '';
  /** Summaries a content search brought that the list does not hold */
  @Trace hitSummaries: SessionSummary[] = [];
  /** Projects folded open in the by-project view; undefined until one is toggled (the demo opens the latest one) */
  @Trace opened: string[] | undefined = undefined;
  /** The current toast's button */
  @Trace toastAction: ToastAction | undefined = undefined;

  private host: ListHost;
  private trash: Map<string, SessionSummary> = new Map<string, SessionSummary>();
  private searchSeq: number = 0;
  private searchQuery: string = '';
  private askSeq: number = 0;
  private asks: Ask[] = [];

  constructor(host: ListHost) {
    this.host = host;
  }

  /** The stored grouping and pin placement */
  restore(): void {
    this.view = prefs.string(PrefKey.listView, 'time');
    this.pinPlace = prefs.string(PrefKey.pinPlace, 'top');
  }

  pickView(next: string): void {
    this.view = next;
    prefs.set(PrefKey.listView, next);
  }

  pickPinPlace(next: string): void {
    this.pinPlace = next;
    prefs.set(PrefKey.pinPlace, next);
  }

  openProjects(): string[] {
    if (this.opened !== undefined) {
      return this.opened;
    }
    const first = this.host.summaries.length > 0 ? this.host.summaries[0].cwd : undefined;
    return this.host.live || first === undefined ? [] : [first];
  }

  toggleOpen(cwd: string): void {
    const now = this.openProjects();
    this.opened = now.includes(cwd) ? now.filter((c: string) => c !== cwd) : now.concat([cwd]);
  }

  // Toasts

  /** A toast, with a button that keeps it longer */
  toast(text: string, label?: string, run?: () => void): void {
    if (label !== undefined && run !== undefined) {
      const action = new ToastAction();
      action.label = label;
      action.toast = text;
      action.run = run;
      this.toastAction = action;
    } else {
      this.toastAction = undefined;
    }
    // A repeat of the same text still shows again
    this.host.toast = undefined;
    this.host.toast = text;
  }

  /** The toast's button was tapped: its action runs and the toast goes */
  runToastAction(): void {
    const action = this.toastAction;
    if (action === undefined) {
      return;
    }
    this.toastAction = undefined;
    this.host.toast = undefined;
    action.run();
  }

  // The engine's messages

  follow(categories: SessionCategories): void {
    this.removedProjects = categories.removedProjects ?? [];
    this.sections = categories.sections ?? [];
  }

  receive(message: ListHostMsg): void {
    switch (message.type) {
      case 'sessionHits': {
        // A reply to an older query is dropped rather than mixed in
        if (message.seq !== this.searchSeq) {
          return;
        }
        const found = new Map<string, string>();
        const extra: SessionSummary[] = [];
        (message.hits ?? []).forEach((h: SessionHit) => {
          found.set(h.id, h.snippet);
          if (h.summary !== undefined) {
            extra.push(h.summary);
          }
        });
        this.hitsQuery = this.searchQuery;
        this.hitSummaries = extra;
        this.hits = found;
        break;
      }
      case 'sessionMarkdown': {
        const at = this.asks.findIndex((a: Ask) => a.seq === message.seq);
        if (at >= 0) {
          const ask = this.asks[at];
          this.asks.splice(at, 1);
          ask.done(message.markdown);
        }
        break;
      }
      default:
        break;
    }
  }

  // Projects

  /** Every known project for the folder menus, the latest first: added by hand, used by sessions, registered */
  projects(): string[] {
    const removed = this.removedProjects;
    const used = this.host.summaries.slice().sort((a: SessionSummary, b: SessionSummary) => b.updatedAt.localeCompare(a.updatedAt))
      .map((s: SessionSummary) => s.cwd);
    return distinct(this.host.addedProjects.concat(used).concat(this.host.projectOrder)
      .filter((c: string) => c.length > 0 && !this.host.isChat(c) && !removed.includes(c)));
  }

  /** The by-project view's projects (sessionTree.ts buildSessionTree): the user's order, then folders sessions ran in */
  listedProjects(): string[] {
    const registered = this.host.projectOrder.concat(this.host.addedProjects);
    const removed = this.removedProjects;
    const used = this.host.summaries.slice().sort((a: SessionSummary, b: SessionSummary) => b.updatedAt.localeCompare(a.updatedAt))
      .map((s: SessionSummary) => s.cwd).filter((c: string) => !c.startsWith(SSH) || registered.includes(c));
    return distinct(registered.concat(used).filter((c: string) => c.length > 0 && !this.host.isChat(c) && !removed.includes(c)));
  }

  /** The project leads the by-project view: registered if it was not, then moved before the first one */
  moveProjectToTop(cwd: string): void {
    const listed = this.listedProjects();
    if (listed.length > 0 && listed[0] === cwd) {
      return;
    }
    if (this.host.live) {
      if (!this.host.projectOrder.includes(cwd)) {
        this.host.postLive({ type: 'categoryOp', op: 'addProject', cwd: cwd });
      }
      this.host.postLive({ type: 'categoryOp', op: 'reorderProject', cwd: cwd, before: this.host.projectOrder.find((c: string) => c !== cwd) });
      return;
    }
    this.host.projectOrder = [cwd].concat(listed.filter((c: string) => c !== cwd));
  }

  /** Takes a folder off the list (its sessions stay and a search still finds them); the toast puts it back */
  removeProject(cwd: string): void {
    if (!this.host.postLive({ type: 'categoryOp', op: 'removeProject', cwd: cwd })) {
      this.host.projectOrder = this.host.projectOrder.filter((c: string) => c !== cwd);
      if (!this.removedProjects.includes(cwd)) {
        this.removedProjects = this.removedProjects.concat([cwd]);
      }
    }
    const name = new Map<string, string>([['name', projectName(cwd)]]);
    this.toast(t('mobile.project.removed', name), t('common.undo'), () => {
      this.host.addProject(cwd);
      if (!this.host.live) {
        this.removedProjects = this.removedProjects.filter((c: string) => c !== cwd);
      }
    });
  }

  /** Files a folder into one of the desktop's sections at its end, or takes it out (`section` undefined) */
  setProjectSection(cwd: string, section: string | undefined): void {
    if (this.host.postLive({ type: 'categoryOp', op: 'setProjectSection', cwd: cwd, section: section })) {
      return;
    }
    this.sections = this.sections.map((s: SessionSection) => {
      const rest = s.projects.filter((c: string) => c !== cwd);
      const next: SessionSection = { id: s.id, name: s.name, projects: s.id === section ? rest.concat([cwd]) : rest };
      return next;
    });
  }

  copyPath(cwd: string): void {
    this.clip(cwd);
    this.toast(t('session.project.copied'));
  }

  // Session actions

  pin(id: string): void {
    const s = this.summary(id);
    const pinned = s?.pinned !== true;
    if (!this.host.postLive({ type: 'pinSession', id: id, pinned: pinned })) {
      this.patch(id, (x: SessionSummary) => {
        x.pinned = pinned ? true : undefined;
      });
    }
    this.toast(t(pinned ? 'mobile.pinned' : 'mobile.unpinned'));
  }

  /** Archived sessions leave the list (its view menu's "archived" shows them); the toast's undo brings one back */
  setArchived(id: string, archived: boolean): void {
    if (!this.host.postLive({ type: 'archiveSession', id: id, archived: archived })) {
      this.patch(id, (x: SessionSummary) => {
        x.archived = archived ? true : undefined;
      });
    }
    if (!archived) {
      return;
    }
    this.host.closeIf(id);
    const title = new Map<string, string>([['title', this.titleOf(id)]]);
    this.toast(t('session.archived', title), t('common.undo'), () => this.setArchived(id, false));
  }

  setUnread(id: string, unread: boolean): void {
    if (this.host.postLive({ type: 'setSessionUnread', id: id, unread: unread })) {
      return;
    }
    this.patch(id, (s: SessionSummary) => {
      if (s.state === 'background') {
        s.unread = unread ? true : undefined;
      } else if (unread && s.state === undefined) {
        s.state = 'unread';
      } else if (!unread && s.state === 'unread') {
        s.state = undefined;
      }
    });
  }

  rename(id: string, title: string): void {
    const trimmed = title.trim();
    if (trimmed.length === 0) {
      return;
    }
    if (this.host.postLive({ type: 'renameSession', id: id, title: trimmed })) {
      return;
    }
    this.patch(id, (s: SessionSummary) => {
      s.title = trimmed;
    });
  }

  /** Deletes at once, as the desktop does; the toast's undo restores the session */
  delete(id: string): void {
    const summary = this.summary(id);
    const title = new Map<string, string>([['title', this.titleOf(id)]]);
    if (!this.host.postLive({ type: 'deleteSession', id: id }) && summary !== undefined) {
      this.trash.set(id, summary);
    }
    this.host.summaries = this.host.summaries.filter((s: SessionSummary) => s.id !== id);
    this.host.closeIf(id);
    this.toast(t('session.deleted', title), t('common.undo'), () => this.restoreSession(id));
  }

  private restoreSession(id: string): void {
    if (this.host.postLive({ type: 'restoreSession', id: id })) {
      return;
    }
    const s = this.trash.get(id);
    if (s !== undefined) {
      this.trash.delete(id);
      this.host.summaries = this.host.summaries.concat([s]);
    }
  }

  /** Every unread mark goes, for every engine and window on the machine */
  markAllRead(): void {
    if (this.host.postLive({ type: 'markAllRead' })) {
      return;
    }
    this.host.summaries = this.host.summaries.map((s: SessionSummary) => {
      const x = copySummary(s);
      x.unread = undefined;
      if (x.state === 'unread') {
        x.state = undefined;
      }
      return x;
    });
  }

  /** Moves a session to another project folder: the agent works there from its next open on */
  move(id: string, cwd: string): void {
    if (!this.host.postLive({ type: 'moveSession', id: id, cwd: cwd })) {
      this.patch(id, (s: SessionSummary) => {
        s.cwd = cwd;
      });
    }
    const words = new Map<string, string>([['title', this.titleOf(id)], ['project', projectName(cwd)]]);
    this.toast(t('session.movedTo', words));
  }

  /** The history search over saved conversations: the engine answers `sessionHits`, the demo searches its views */
  search(query: string): void {
    const terms = searchTerms(query);
    this.searchSeq++;
    if (terms.length === 0) {
      this.searchQuery = '';
      this.hits = undefined;
      this.hitSummaries = [];
      return;
    }
    const q = terms.join(' ');
    this.searchQuery = q;
    if (this.host.postLive({ type: 'searchSessions', query: q, seq: this.searchSeq })) {
      return;
    }
    const found = new Map<string, string>();
    this.host.demoViews().forEach((v: SessionView) => {
      const snippet = searchSnippet(v, terms);
      if (snippet !== undefined) {
        found.set(v.id, snippet);
      }
    });
    this.hitsQuery = q;
    this.hitSummaries = [];
    this.hits = found;
  }

  /** The session as the engine's Markdown (or text) export; the demo exports the view it holds */
  sessionMarkdown(id: string, format: ExportFormat, done: (text: string | undefined) => void): void {
    if (!this.host.live) {
      const view = this.host.heldView(id);
      if (view === undefined) {
        done(undefined);
        return;
      }
      const name = this.host.agentName(view.agent);
      done(format === ExportFormat.Text ? exportText(view, name) : exportMarkdown(view, name, this.host.now()));
      return;
    }
    const ask = new Ask();
    ask.seq = ++this.askSeq;
    let answered = false;
    ask.done = (markdown: string | undefined): void => {
      if (!answered) {
        answered = true;
        done(markdown);
      }
    };
    this.asks.push(ask);
    // Markdown asks without the field, as before it existed
    this.host.postLive({ type: 'sessionMarkdown', id: id, seq: ask.seq, format: format === ExportFormat.Text ? 'text' : undefined });
    setTimeout(() => {
      this.asks = this.asks.filter((a: Ask) => a !== ask);
      ask.done(undefined);
    }, ASK_TIMEOUT_MS);
  }

  copy(id: string, kind: SessionCopy): void {
    const finish = (text: string | undefined): void => {
      if (text === undefined || text.length === 0) {
        this.toast(t('session.copyFailed', new Map<string, string>([['detail', t('mobile.engineNeeded')]])));
        return;
      }
      this.clip(text);
      this.toast(t(kind === SessionCopy.Link ? 'session.copied.link' : kind === SessionCopy.Markdown ? 'session.copied.markdown' : 'session.copied.cwd'));
    };
    if (kind === SessionCopy.Link) {
      finish(SESSION_LINK + id);
    } else if (kind === SessionCopy.Cwd) {
      finish(this.host.heldView(id)?.cwd ?? this.summary(id)?.cwd);
    } else {
      this.sessionMarkdown(id, ExportFormat.Markdown, finish);
    }
  }

  /** The phone holds the session's view (a JSON export needs it) */
  holdsView(id: string): boolean {
    return this.host.heldView(id) !== undefined;
  }

  /** Writes the export where the system's save dialog says: Markdown and text are the engine's, JSON the held view */
  export(id: string, format: ExportFormat): void {
    const write = (body: string | undefined): void => {
      if (body === undefined || body.length === 0) {
        this.toast(t('mobile.engineNeeded'));
        return;
      }
      const ext = format === ExportFormat.Json ? 'json' : format === ExportFormat.Text ? 'txt' : 'md';
      this.save(`${exportFileName(this.titleOf(id))}.${ext}`, body);
    };
    if (format === ExportFormat.Json) {
      const view = this.host.heldView(id);
      write(view === undefined ? undefined : exportJson(view));
    } else {
      this.sessionMarkdown(id, format, write);
    }
  }

  private save(name: string, body: string): void {
    const context = this.host.appContext();
    if (context === undefined) {
      this.toast(t('mobile.engineNeeded'));
      return;
    }
    try {
      const options = new picker.DocumentSaveOptions();
      options.newFileNames = [name];
      new picker.DocumentViewPicker(context).save(options).then((uris: string[]) => {
        if (uris.length === 0) {
          return;
        }
        const file = fileIo.openSync(uris[0], fileIo.OpenMode.READ_WRITE | fileIo.OpenMode.CREATE | fileIo.OpenMode.TRUNC);
        fileIo.writeSync(file.fd, body);
        fileIo.closeSync(file);
      }).catch((e: Error) => this.toast(e.message));
    } catch (e) {
      this.toast(`${e}`);
    }
  }

  private clip(text: string): void {
    try {
      pasteboard.getSystemPasteboard().setDataSync(pasteboard.createData(pasteboard.MIMETYPE_TEXT_PLAIN, text));
    } catch (e) {
      // No pasteboard (the Previewer)
    }
  }

  summary(id: string): SessionSummary | undefined {
    return this.host.summaries.find((s: SessionSummary) => s.id === id);
  }

  titleOf(id: string): string {
    return this.host.heldView(id)?.title ?? this.summary(id)?.title ?? '';
  }

  /** The demo's list with one session changed in place (a copy, so the rows render again) */
  private patch(id: string, change: (s: SessionSummary) => void): void {
    this.host.summaries = this.host.summaries.map((s: SessionSummary) => {
      if (s.id !== id) {
        return s;
      }
      const x = copySummary(s);
      change(x);
      return x;
    });
  }
}

export function copySummary(s: SessionSummary): SessionSummary {
  const x: SessionSummary = {
    id: s.id, title: s.title, agent: s.agent, cwd: s.cwd, updatedAt: s.updatedAt, pinned: s.pinned,
    archived: s.archived, state: s.state, unread: s.unread
  };
  return x;
}

function distinct(list: string[]): string[] {
  const out: string[] = [];
  list.forEach((s: string) => {
    if (!out.includes(s)) {
      out.push(s);
    }
  });
  return out;
}
