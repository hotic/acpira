import { SessionSection, SessionSummary } from '../wire/Wire';
import { ListMetrics, Metrics, Type } from '../design/Tokens';
import { AppModel, Connection } from '../store/AppModel';
import { currentLocale, t } from '../store/L10n';
import {
  isUnread, pressing, Pressing, projectName, recency, remoteAlias, searchMatches, searchParts, searchTerms, sessionTime,
  SessionListRows
} from '../store/SessionLogic';
import { AgentMark, Lucide } from '../components/Glyphs';
import { StateMark } from '../components/StateMark';
import { SessionMenu } from '../components/SessionMenu';
import { MenuCaption, MenuCard, MenuGap, MenuRow, menuOptions, MenuBack } from '../components/Menu';
import { connectionNote, ConnectionBanner, GlassButton } from '../components/Chrome';

// The session list (apps/ios Sessions/SessionListScreen.swift, apps/android ui/SessionListScreen.kt): every session,
// grouped as picked in the bar's filter menu. By time: the sessions pinned to the top, then day captions. By project:
// the same pinned run (or each project's pins leading it), then the desktop app's sections, each project as a header
// folding open in place, the latest few, the rest behind "N more", then the chats. By channel: pinned, then a caption
// per agent. Ungrouped: one run. The archived view lists the archived sessions alone. A search lists one run across
// every project, matching the conversations too (a content hit shows the text around its match under the title). A
// row's long press opens the session menu; a swipe bares Delete, whose toast can undo it. On a phone the search field
// and the new-session button ride the bottom; in a wide window's sidebar the search stands under the bar and the foot
// holds settings and the engine.

/** One line of the list: what it draws and the facts it draws from */
class ListEntry {
  kind: string = '';
  key: string = '';
  text: string = '';
  session: SessionSummary | undefined = undefined;
  place: string | undefined = undefined;
  time: string | undefined = undefined;
  snippet: string | undefined = undefined;
  divider: boolean = false;
  nested: boolean = false;
  cwd: string = '';
  open: boolean = false;
  first: boolean = false;
  sessions: SessionSummary[] = [];
  rest: SessionSummary[] = [];
}

/** A row's identity for ForEach: everything it draws, so a changed summary renders again */
function rowKey(s: SessionSummary): string {
  return `${s.id}:${s.title}:${s.state ?? ''}:${s.unread === true}:${s.updatedAt}:${s.pinned === true}:${s.cwd}`;
}

const CHATS_KEY: string = ' chats';

@ComponentV2
export struct SessionList {
  @Require @Param model: AppModel;
  /** The chat open beside the sidebar */
  @Param openId: string | undefined = undefined;
  /** Drawn as a wide window's sidebar: search on top, settings in the foot */
  @Param sidebar: boolean = false;
  /** A query typed on launch (screenshots of its hits; the Previewer cannot type) */
  @Param initialQuery: string = '';
  @Event onOpen: (id: string) => void = () => {};
  /** A new session, in `cwd` when given ('' a chat) */
  @Event onNew: (cwd: string | undefined) => void = () => {};
  @Event onSettings: () => void = () => {};
  @Event onHide: () => void = () => {};
  @Local query: string = '';
  @Local archivedOnly: boolean = false;
  @Local viewMenu: boolean = false;
  /** Per open project, how many sessions past its latest few are out */
  @Local moreShown: Map<string, number> = new Map<string, number>();
  private searchTimer: number = -1;

  aboutToAppear(): void {
    if (this.initialQuery.length > 0) {
      this.query = this.initialQuery;
      this.model.lists.search(this.initialQuery);
    }
  }

  /** The conversation search goes out once typing pauses */
  typed(value: string): void {
    this.query = value;
    clearTimeout(this.searchTimer);
    if (value.length === 0) {
      this.model.lists.search('');
      return;
    }
    this.searchTimer = setTimeout(() => this.model.lists.search(value), ListMetrics.searchDebounceMs);
  }

  /** Listed folders alone keep the filter too: from another view its menu is the way back to them */
  showsFilter(): boolean {
    return searchTerms(this.query).length === 0 &&
      !(this.model.summaries.length === 0 && this.model.lists.listedProjects().length === 0);
  }

  // The entries

  entries(): ListEntry[] {
    const model = this.model;
    const lists = model.lists;
    const now = model.now();
    const locale = currentLocale();
    const terms = searchTerms(this.query);
    const searching = terms.length > 0;
    // Content hits count for the query they answered only
    const snippets = lists.hits !== undefined && lists.hitsQuery === terms.join(' ') ? lists.hits : undefined;
    const listed = model.summaries.filter((s: SessionSummary) => (s.archived === true) === this.archivedOnly);
    const known = new Set<string>(model.summaries.map((s: SessionSummary) => s.id));
    const extra = snippets === undefined ? [] :
      lists.hitSummaries.filter((s: SessionSummary) => !known.has(s.id) && (s.archived === true) === this.archivedOnly);
    const pool = listed.concat(extra);
    const shown = !searching ? pool : pool.filter((s: SessionSummary) => snippets?.has(s.id) === true ||
      searchMatches([s.title, model.agentName(s.agent), model.placeName(s.cwd)], terms));
    // Pinned first, then the latest
    const ordered = shown.slice().sort((a: SessionSummary, b: SessionSummary) => {
      const pin = (b.pinned === true ? 1 : 0) - (a.pinned === true ? 1 : 0);
      return pin !== 0 ? pin : b.updatedAt.localeCompare(a.updatedAt);
    });
    const byProject = lists.view === 'project' && !searching && !this.archivedOnly;
    const pinsInProject = byProject && lists.pinPlace === 'project';
    const listedProjects = lists.listedProjects();
    const out: ListEntry[] = [];

    const row = (s: SessionSummary, key: string, place: string | undefined, time: string | undefined, divider: boolean,
      nested: boolean): void => {
      const e = new ListEntry();
      e.kind = 'row';
      e.key = `${key}:${rowKey(s)}:${divider}:${time ?? ''}`;
      e.session = s;
      e.place = place;
      e.time = time;
      e.divider = divider;
      e.nested = nested;
      e.snippet = snippets?.get(s.id);
      if (e.snippet !== undefined) {
        e.key += `:${e.snippet}`;
      }
      out.push(e);
    };
    const simple = (kind: string, key: string, text: string): ListEntry => {
      const e = new ListEntry();
      e.kind = kind;
      e.key = key;
      e.text = text;
      out.push(e);
      return e;
    };

    // By project, the chats' group still stands (its "new chat" row) when nothing else does
    if (ordered.length === 0 && !(byProject && (listedProjects.length > 0 || model.chatRoot !== undefined))) {
      simple('empty', `empty:${this.emptyNote(searching, snippets !== undefined)}`, this.emptyNote(searching, snippets !== undefined));
    }

    // The runs: one for a search or ungrouped, else pinned then the grouping's captions
    if (searching || lists.view === 'none') {
      ordered.forEach((s: SessionSummary, i: number) => {
        const next = i + 1 < ordered.length ? ordered[i + 1] : undefined;
        const pinEnd = s.pinned === true && next !== undefined && next.pinned !== true;
        row(s, 'all', model.placeName(s.cwd), sessionTime(s.updatedAt, now, locale), i < ordered.length - 1 && !pinEnd, false);
        if (pinEnd) {
          simple('pinRule', 'pinEnd:all', '');
        }
      });
    } else {
      const top = pinsInProject ? [] : ordered.filter((s: SessionSummary) => s.pinned === true);
      if (top.length > 0) {
        simple('caption', 'caption:pinned', t('session.group.pinned'));
        top.forEach((s: SessionSummary) => row(s, 'pinned', model.placeName(s.cwd), sessionTime(s.updatedAt, now, locale), false, false));
      }
      if (!byProject) {
        const rest = ordered.filter((s: SessionSummary) => s.pinned !== true);
        if (lists.view === 'agent') {
          const agents: string[] = [];
          rest.forEach((s: SessionSummary) => {
            if (!agents.includes(s.agent)) {
              agents.push(s.agent);
            }
          });
          agents.forEach((a: string) => {
            simple('caption', `caption:agent.${a}`, model.agentName(a));
            rest.filter((s: SessionSummary) => s.agent === a)
              .forEach((s: SessionSummary) => row(s, `agent.${a}`, model.placeName(s.cwd), sessionTime(s.updatedAt, now, locale), false, false));
          });
        } else {
          for (const key of ['today', 'yesterday', 'week', 'month', 'older']) {
            const bucket = rest.filter((s: SessionSummary) => recency(s.updatedAt, now) === key);
            if (bucket.length > 0) {
              simple('caption', `caption:${key}`, t(`session.when.${key}`));
              // A day caption already says when
              bucket.forEach((s: SessionSummary) => row(s, key, model.placeName(s.cwd), undefined, false, false));
            }
          }
        }
      }
    }

    if (byProject) {
      const project = (cwd: string): void => {
        const all = listed.filter((s: SessionSummary) => s.cwd === cwd);
        const pins = pinsInProject ? all.filter((s: SessionSummary) => s.pinned === true).sort(latest) : [];
        const sessions = all.filter((s: SessionSummary) => s.pinned !== true).sort(latest);
        const open = lists.openProjects().includes(cwd);
        const cut = sessions.length <= ListMetrics.perProject + 1 ? sessions.length : ListMetrics.perProject;
        const page = SessionListRows.of(sessions, cut, this.moreShown.get(cwd) ?? 0, this.openId);
        const head = new ListEntry();
        head.kind = 'project';
        head.cwd = cwd;
        head.open = open;
        head.first = listedProjects.length > 0 && listedProjects[0] === cwd;
        head.sessions = all;
        head.key = `project:${cwd}:${open}:${head.first}:${open ? '' : JSON.stringify(pressing(all) ?? '')}`;
        out.push(head);
        if (!open) {
          return;
        }
        // Inside its project a row is one line; no caption says when, so every row keeps its time
        pins.forEach((s: SessionSummary) => row(s, `p:${cwd}`, undefined, sessionTime(s.updatedAt, now, locale), false, true));
        if (pins.length > 0 && (page.rows.length > 0 || page.canFold)) {
          simple('pinRule', `pinEnd:${cwd}`, '');
        }
        page.rows.forEach((s: SessionSummary) => row(s, `p:${cwd}`, undefined, sessionTime(s.updatedAt, now, locale), false, true));
        if (page.canFold) {
          const more = simple('more', `more:${cwd}:${page.rest.map((s: SessionSummary) => rowKey(s)).join('|')}`, '');
          more.cwd = cwd;
          more.rest = page.rest;
          more.nested = true;
        }
      };
      const runs = lists.sections.map((s: SessionSection) => s.projects.filter((c: string) => listedProjects.includes(c)));
      if (runs.every((r: string[]) => r.length === 0)) {
        listedProjects.forEach((c: string) => project(c));
      } else {
        const filed = new Set<string>();
        lists.sections.forEach((section: SessionSection, i: number) => {
          if (runs[i].length === 0) {
            return;
          }
          simple('caption', `section:${section.id}`, section.name.length > 0 ? section.name : t('session.section.untitled'));
          runs[i].forEach((c: string) => {
            filed.add(c);
            project(c);
          });
        });
        const rest = listedProjects.filter((c: string) => !filed.has(c));
        if (rest.length > 0) {
          simple('caption', 'section:', t('session.group.projects'));
          rest.forEach((c: string) => project(c));
        }
      }
      // The chats under the projects, as the desktop list puts them
      if (model.chatRoot !== undefined) {
        const all = listed.filter((s: SessionSummary) => model.isChat(s.cwd));
        const pins = pinsInProject ? all.filter((s: SessionSummary) => s.pinned === true).sort(latest) : [];
        const sessions = all.filter((s: SessionSummary) => s.pinned !== true).sort(latest);
        const cut = sessions.length <= ListMetrics.perProject + 1 ? sessions.length : ListMetrics.perProject;
        const page = SessionListRows.of(sessions, cut, this.moreShown.get(CHATS_KEY) ?? 0, this.openId);
        simple('caption', 'caption:chats', t('session.group.chats'));
        pins.forEach((s: SessionSummary) => row(s, 'c', undefined, sessionTime(s.updatedAt, now, locale), false, false));
        if (pins.length > 0 && page.rows.length > 0) {
          simple('pinRule', 'pinEnd:chats', '');
        }
        page.rows.forEach((s: SessionSummary) => row(s, 'c', undefined, sessionTime(s.updatedAt, now, locale), false, false));
        if (page.canFold) {
          const more = simple('more', `more:chats:${page.rest.map((s: SessionSummary) => rowKey(s)).join('|')}`, '');
          more.cwd = CHATS_KEY;
          more.rest = page.rest;
        }
        if (all.length === 0) {
          simple('newChat', 'newChat', t('session.chat.new'));
        }
      }
    }
    return out;
  }

  /** What the list says with nothing to show: still searching, no match, or none yet (and where the engine stands) */
  emptyNote(searching: boolean, answered: boolean): string {
    if (searching) {
      return t(answered ? 'session.noMatch' : 'session.searching');
    }
    return connectionNote(this.model) ?? t('session.none');
  }

  more(cwd: string, rest: SessionSummary[]): void {
    const next = new Map<string, number>(this.moreShown);
    if (rest.length === 0) {
      next.delete(cwd);
    } else {
      next.set(cwd, (next.get(cwd) ?? 0) + ListMetrics.morePage);
    }
    this.moreShown = next;
  }

  // The page

  build() {
    Column() {
      this.bar()
      ConnectionBanner({ model: this.model, onRetry: () => this.model.reconnect() })
      if (this.sidebar) {
        Row() {
          this.searchField()
        }
        .padding({ left: Metrics.pad, right: Metrics.pad, bottom: Metrics.s2 })
      }
      if (this.model.connection === Connection.Connecting && this.model.summaries.length === 0) {
        // The engine's first list is on its way: its state takes the page, so the list giving way moves nothing
        Column({ space: Metrics.gap }) {
          Lucide({ name: 'laptop', tint: $r('app.color.fg2'), extent: Metrics.barCtl })
          Row({ space: Metrics.s2 }) {
            LoadingProgress().color($r('app.color.fg3')).width(Metrics.iconCtl).height(Metrics.iconCtl)
            Text(t('connections.state.connecting')).fontSize(Type.text2).fontColor($r('app.color.fg2'))
          }
        }
        .id('list.connecting')
        .justifyContent(FlexAlign.Center)
        .layoutWeight(1)
      } else {
        List() {
          ForEach(this.entries(), (e: ListEntry) => {
            if (e.kind === 'row' && e.session !== undefined) {
              ListItem() {
                SessionRowItem({
                  model: this.model,
                  session: e.session,
                  place: e.place,
                  time: e.time,
                  snippet: e.snippet,
                  terms: searchTerms(this.query),
                  divider: e.divider,
                  nested: e.nested,
                  active: e.session.id === this.openId,
                  onOpen: (id: string) => this.onOpen(id)
                })
              }
              .swipeAction({ end: { builder: () => { this.deleteAction(e.session?.id ?? ''); } }, edgeEffect: SwipeEdgeEffect.Spring })
            } else {
              ListItem() {
                this.entry(e)
              }
            }
          }, (e: ListEntry) => e.key)
        }
        .id('list.sessions')
        .scrollBar(BarState.Off)
        .edgeEffect(EdgeEffect.Spring)
        .contentEndOffset(Metrics.msgGap)
        .layoutWeight(1)
        .width('100%')
      }
      if (this.sidebar) {
        this.foot()
      } else {
        this.bottomBar()
      }
    }
    .backgroundColor($r('app.color.bg0'))
    .height('100%')
    .width('100%')
  }

  /**
   * The bar: the grouping filter at the start, the title centred, settings at the end; in the sidebar no title, and
   * the end hides the sidebar (Navigation's split has no toggle of its own once its title bar is off)
   */
  @Builder
  bar() {
    Stack() {
      // The sidebar shows no title, as iOS keeps it off there
      if (!this.sidebar) {
        Text(t('mobile.home'))
          .fontSize(Type.barTitle)
          .fontWeight(FontWeight.Bold)
          .fontColor($r('app.color.fg_strong'))
      }
      Row() {
        if (this.showsFilter()) {
          GlassButton({ icon: 'listFilter', tag: 'list.view', onTap: () => { this.viewMenu = true; } })
            .bindMenu(this.viewMenu, this.viewMenuCard(), menuOptions(() => { this.viewMenu = false; }, Placement.BottomLeft))
        }
        Blank()
        if (this.sidebar) {
          GlassButton({ icon: 'panelLeft', tag: 'split.hide', onTap: () => this.onHide() })
        } else {
          GlassButton({ icon: 'settings', tag: 'list.settings', onTap: () => this.onSettings() })
        }
      }
      .width('100%')
      .padding({ left: Metrics.pad, right: Metrics.pad })
    }
    .height(Metrics.barCtl + Metrics.gap)
    .width('100%')
  }

  /** The list's grouping filter: the grouping ticked, where pins sit by project, archived, mark all read */
  @Builder
  viewMenuCard() {
    MenuCard() {
      MenuCaption({ title: t('session.view.group') })
      MenuRow({ text: t('mobile.view.time'), checked: this.model.lists.view === 'time', icon: 'clock', tag: 'view.time', onTap: () => this.pickView('time') })
      MenuRow({ text: t('mobile.view.project'), checked: this.model.lists.view === 'project', icon: 'folder', tag: 'view.project', onTap: () => this.pickView('project') })
      MenuRow({ text: t('mobile.view.agent'), checked: this.model.lists.view === 'agent', icon: 'bot', tag: 'view.agent', onTap: () => this.pickView('agent') })
      MenuRow({ text: t('mobile.view.none'), checked: this.model.lists.view === 'none', icon: 'list', tag: 'view.none', onTap: () => this.pickView('none') })
      if (this.model.lists.view === 'project') {
        MenuGap()
        MenuCaption({ title: t('session.view.pinned') })
        MenuRow({
          text: t('session.view.pinned.top'), checked: this.model.lists.pinPlace === 'top', icon: 'pin', tag: 'view.pin.top',
          onTap: () => { this.model.lists.pickPinPlace('top'); this.viewMenu = false; }
        })
        MenuRow({
          text: t('session.view.pinned.project'), checked: this.model.lists.pinPlace === 'project', icon: 'folder', tag: 'view.pin.project',
          onTap: () => { this.model.lists.pickPinPlace('project'); this.viewMenu = false; }
        })
      }
      MenuGap()
      MenuRow({
        text: t('session.view.status.archived'), checked: this.archivedOnly, icon: 'archive', tag: 'view.archived',
        onTap: () => { this.archivedOnly = !this.archivedOnly; this.viewMenu = false; }
      })
      if (this.model.summaries.some((s: SessionSummary) => isUnread(s))) {
        MenuRow({
          text: t('session.view.markAllRead'), checked: false, icon: 'mailCheck', tag: 'view.markAllRead',
          onTap: () => { this.model.lists.markAllRead(); this.viewMenu = false; }
        })
      }
    }
  }

  pickView(view: string): void {
    this.model.lists.pickView(view);
    this.viewMenu = false;
  }

  @Builder
  entry(e: ListEntry) {
    if (e.kind === 'caption') {
      Text(e.text)
        .fontSize(Type.text3)
        .fontWeight(FontWeight.Medium)
        .fontColor($r('app.color.fg3'))
        .padding({ left: Metrics.pad, right: Metrics.pad, bottom: Metrics.s1 })
        .constraintSize({ minHeight: ListMetrics.groupCaption })
        .align(Alignment.BottomStart)
        .width('100%')
    } else if (e.kind === 'pinRule') {
      // The end of a pinned run: the desktop's short faded rule
      Column()
        .height(Metrics.hairline)
        .width('100%')
        .margin({ top: Metrics.s1, bottom: Metrics.s1 })
        .linearGradient({
          angle: 90,
          colors: [[$r('app.color.bg0'), 0], [$r('app.color.line_strong'), 0.5], [$r('app.color.bg0'), 1]]
        })
        .padding({ left: Metrics.pad, right: Metrics.pad })
    } else if (e.kind === 'project') {
      ProjectHeader({
        model: this.model, cwd: e.cwd, open: e.open, first: e.first, sessions: e.sessions,
        onNew: (cwd: string) => this.onNew(cwd)
      })
    } else if (e.kind === 'more') {
      this.moreRow(e)
    } else if (e.kind === 'newChat') {
      Row({ space: Metrics.gap }) {
        Stack() {
          Lucide({ name: 'plus', tint: $r('app.color.fg3'), extent: Metrics.icon })
        }
        .width(Metrics.lead)
        .alignContent(Alignment.Start)
        Text(e.text).fontSize(Type.text2).fontColor($r('app.color.fg3')).layoutWeight(1)
      }
      .id('list.newChat')
      .constraintSize({ minHeight: ListMetrics.sheetRow })
      .padding({ left: Metrics.pad, right: Metrics.pad })
      .width('100%')
      .onClick(() => this.onNew(''))
    } else if (e.kind === 'empty') {
      Text(e.text)
        .id('list.empty')
        .fontSize(Type.text2)
        .fontColor($r('app.color.fg3'))
        .textAlign(TextAlign.Center)
        .constraintSize({ minHeight: ListMetrics.sheetRow })
        .width('100%')
    }
  }

  /**
   * SessionList.tsx MoreRow, under an open project's latest few: how many the rest holds, the agents among them and
   * the most urgent state inside; with all of them out it reads "show less", the chevron turned over
   */
  @Builder
  moreRow(e: ListEntry) {
    Row({ space: Metrics.gap }) {
      Stack() {
        Lucide({ name: 'chevronDown', tint: $r('app.color.fg3'), extent: ListMetrics.chevron })
          .rotate({ angle: e.rest.length === 0 ? 180 : 0 })
      }
      .width(Metrics.rowMark)
      .margin({ right: Metrics.lead - Metrics.rowMark })
      Text(e.rest.length === 0 ? t('session.showLess') : t('session.showMore', new Map<string, string>([['n', `${e.rest.length}`]])))
        .fontSize(Type.text2)
        .fontColor($r('app.color.fg3'))
        .maxLines(1)
      if (e.rest.length > 0) {
        Row({ space: Metrics.s1 }) {
          ForEach(moreAgents(e.rest), (a: string) => {
            AgentMark({ agent: a, name: this.model.agentName(a), tint: $r('app.color.fg2'), extent: ListMetrics.moreMark })
          }, (a: string) => a)
        }
      }
      Blank()
      if (pressing(e.rest) !== undefined) {
        StateMark({ state: pressing(e.rest)?.state ?? '', unread: pressing(e.rest)?.unread ?? false, motion: this.model.motion })
      }
    }
    .id(`more:${e.cwd}`)
    .constraintSize({ minHeight: ListMetrics.sheetRow })
    .padding({ left: Metrics.pad + (e.nested ? ListMetrics.nest : 0), right: Metrics.pad })
    .width('100%')
    .onClick(() => this.more(e.cwd, e.rest))
  }

  /** The bared room behind a swiped row: the system's red Delete */
  @Builder
  deleteAction(id: string) {
    Row() {
      Text(t('common.delete'))
        .fontSize(Type.text2)
        .fontWeight(FontWeight.Medium)
        .fontColor(Color.White)
        .maxLines(1)
    }
    .id(`session.delete:${id}`)
    .justifyContent(FlexAlign.Center)
    .width(ListMetrics.swipeAction)
    .height('100%')
    .padding({ top: Metrics.s2, bottom: Metrics.s2 })
    .margin({ right: Metrics.pad, left: Metrics.s2 })
    .borderRadius(ListMetrics.sheetRow / 2)
    .backgroundColor($r('app.color.swipe_delete'))
    .onClick(() => this.model.lists.delete(id))
  }

  @Builder
  searchField() {
    Row({ space: Metrics.s2 + Metrics.s1 }) {
      Lucide({ name: 'search', tint: this.sidebar ? $r('app.color.fg3') : $r('app.color.fg1'), extent: this.sidebar ? Metrics.icon : Metrics.iconCtl })
      TextInput({ placeholder: t('session.searchContent'), text: this.query })
        .id('list.search')
        .fontSize(Type.text1)
        .fontColor($r('app.color.fg1'))
        .placeholderColor($r('app.color.fg3'))
        .placeholderFont({ size: Type.text1 })
        .backgroundColor(Color.Transparent)
        .padding(0)
        .cancelButton({ style: CancelButtonStyle.INPUT })
        .layoutWeight(1)
        .height('100%')
        .onChange((v: string) => this.typed(v))
    }
    .width('100%')
    .padding({ left: Metrics.pad, right: Metrics.pad })
    .height(this.sidebar ? Metrics.ctl : Metrics.barCtl)
    .borderRadius(this.sidebar ? Metrics.ctl / 2 : Metrics.barCtl / 2)
    .backgroundColor(this.sidebar ? $r('app.color.bg2') : $r('app.color.glass_bg'))
    .border({ width: this.sidebar ? 0 : Metrics.hairline, color: $r('app.color.glass_line') })
    .shadow(this.sidebar ? { radius: 0, color: Color.Transparent } :
      { radius: ListMetrics.barShadow, color: $r('app.color.line'), offsetY: Metrics.s1 / 2 })
  }

  /** The phone's bottom bar: search, and a new session in the latest project */
  @Builder
  bottomBar() {
    Row({ space: Metrics.gap }) {
      Row() {
        this.searchField()
      }
      .layoutWeight(1)
      GlassButton({
        icon: 'squarePen', tag: 'list.new', onTap: () => {
          const projects = this.model.lists.projects();
          this.onNew(projects.length > 0 ? projects[0] : undefined);
        }
      })
    }
    .padding({ left: Metrics.pad, right: Metrics.pad, top: Metrics.s2, bottom: Metrics.s2 })
    .constraintSize({ maxWidth: Metrics.contentW })
    .width('100%')
  }

  /** The sidebar's foot, as the desktop's session column ends: settings, then the engine followed and its state */
  @Builder
  foot() {
    Row({ space: Metrics.gap }) {
      GlassButton({ icon: 'settings', tag: 'sidebar.settings', onTap: () => this.onSettings() })
      // The followed engine, as iOS shows the selected computer; the demo has none
      if (this.model.engineAddress.length > 0) {
        Row({ space: ListMetrics.s1_5 }) {
          Lucide({ name: 'laptop', tint: $r('app.color.fg2'), extent: Metrics.icon })
          Text(this.model.engineAddress)
            .fontSize(Type.text2)
            .fontColor($r('app.color.fg1'))
            .maxLines(1)
            .textOverflow({ overflow: TextOverflow.Ellipsis })
            .layoutWeight(1)
          Circle()
            .width(ListMetrics.footDot)
            .height(ListMetrics.footDot)
            .fill(this.model.connection === Connection.Live ? $r('app.color.ok') : $r('app.color.fg3'))
        }
        .id('sidebar.engine')
        .padding({ left: Metrics.pad, right: Metrics.pad })
        .height(Metrics.barCtl)
        .layoutWeight(1)
        .borderRadius(Metrics.barCtl / 2)
        .backgroundColor($r('app.color.glass_bg'))
        .border({ width: Metrics.hairline, color: $r('app.color.glass_line') })
        .onClick(() => this.onSettings())
      }
    }
    .padding({ left: Metrics.pad, right: Metrics.pad, top: Metrics.s2, bottom: Metrics.s2 })
    .width('100%')
  }
}

function latest(a: SessionSummary, b: SessionSummary): number {
  return b.updatedAt.localeCompare(a.updatedAt);
}

function moreAgents(rest: SessionSummary[]): string[] {
  const out: string[] = [];
  rest.forEach((s: SessionSummary) => {
    if (!out.includes(s.agent) && out.length < ListMetrics.moreMarks) {
      out.push(s.agent);
    }
  });
  return out;
}

/** `text` with a search's terms in the strong ink and weight (SessionList.tsx Marked) */
@Builder
function marked(text: string, terms: string[], size: number, color: ResourceColor, lines: number) {
  Text() {
    ForEach(searchParts(text, terms), (part: string, i: number) => {
      Span(part)
        .fontColor(i % 2 === 1 ? $r('app.color.fg_strong') : color)
        .fontWeight(i % 2 === 1 ? FontWeight.Bold : FontWeight.Normal)
    }, (part: string, i: number) => `${i}:${part}`)
  }
  .fontSize(size)
  .fontColor(color)
  .maxLines(lines)
  .textOverflow({ overflow: TextOverflow.Ellipsis })
}

/**
 * One session, full width, one fact per block: the agent's bare mark; the title, with the time at its right end where
 * no day caption says it, and the state's mark in a trailing slot on that line; under it the place, and a search's
 * content hit. Inside a project's group the row is one line, a step in from the folder. A long press opens its menu
 */
@ComponentV2
struct SessionRowItem {
  @Require @Param model: AppModel;
  @Require @Param session: SessionSummary;
  @Param place: string | undefined = undefined;
  @Param time: string | undefined = undefined;
  @Param snippet: string | undefined = undefined;
  @Param terms: string[] = [];
  @Param divider: boolean = false;
  @Param nested: boolean = false;
  @Param active: boolean = false;
  @Event onOpen: (id: string) => void = () => {};

  @Builder
  menu() {
    SessionMenu({
      model: this.model,
      sessionId: this.session.id,
      onClose: () => this.getUIContext().getContextMenuController().close()
    })
  }

  build() {
    Row({ space: Metrics.gap }) {
      // The bare mark; its column starts the slot, so the ink stands on the title's and captions' edge
      Stack() {
        AgentMark({ agent: this.session.agent, name: this.model.agentName(this.session.agent), tint: $r('app.color.fg2'), extent: Metrics.rowMark })
      }
      .width(Metrics.lead)
      .alignContent(Alignment.Start)
      Column({ space: ListMetrics.s0_5 }) {
        Row() {
          Row() {
            marked(this.session.title, this.terms, Type.text1, $r('app.color.fg_strong'), 1)
          }
          .layoutWeight(1)
          if (this.time !== undefined) {
            Text(this.time)
              .fontSize(Type.text3)
              .fontColor($r('app.color.fg3'))
              .fontFeature('"tnum" on')
              .maxLines(1)
              .margin({ left: Metrics.s2 })
          }
          // The state is its mark alone; every row keeps the slot, so the times above line up
          Stack() {
            if (this.session.state !== undefined && this.session.state.length > 0) {
              StateMark({ state: this.session.state, unread: this.session.unread === true, motion: this.model.motion })
            }
          }
          .width(ListMetrics.stateSlot)
          .height(ListMetrics.stateSlot)
          .margin({ left: Metrics.gap - Metrics.s2 })
        }
        .width('100%')
        if (this.place !== undefined) {
          Text(this.place)
            .fontSize(Type.text3)
            .fontColor($r('app.color.fg3'))
            .maxLines(1)
            .textOverflow({ overflow: TextOverflow.Ellipsis })
        }
        if (this.snippet !== undefined) {
          Row() {
            marked(this.snippet, this.terms, Type.text3, $r('app.color.fg2'), 2)
          }
          .width('100%')
          .id(`session.snippet:${this.session.id}`)
        }
      }
      .alignItems(HorizontalAlign.Start)
      .layoutWeight(1)
    }
    .id(`session:${this.session.id}`)
    .constraintSize({ minHeight: this.place === undefined && this.snippet === undefined ? ListMetrics.sheetRow : ListMetrics.sessionRow })
    .padding({
      left: Metrics.pad + (this.nested ? ListMetrics.nest : 0), right: Metrics.pad,
      top: this.snippet === undefined ? 0 : Metrics.s2, bottom: this.snippet === undefined ? 0 : Metrics.s2
    })
    .width('100%')
    .backgroundColor(this.active ? $r('app.color.active') : $r('app.color.bg0'))
    .border({
      width: { bottom: this.divider ? Metrics.hairline : 0 },
      color: { bottom: $r('app.color.line') }
    })
    .stateStyles({ pressed: { .backgroundColor($r('app.color.active')) } })
    .onClick(() => this.onOpen(this.session.id))
    .bindContextMenu(this.menu(), ResponseType.LongPress, {
      preview: MenuPreviewMode.NONE, enableArrow: false, backgroundColor: Color.Transparent, backgroundBlurStyle: BlurStyle.NONE
    })
  }
}

/**
 * A project's header in the by-project view (apps/ios ProjectHeader): the folder in the marks' column, the name and a
 * chevron turning down while open; folded, the project's most pressing state sits in the rows' state column. A folder
 * on an SSH host wears the globe and ends in its host's alias. A tap folds it open or shut; its long press opens its
 * menu (a session in it, move to top, a section, copy path, off the list)
 */
@ComponentV2
struct ProjectHeader {
  @Require @Param model: AppModel;
  @Param cwd: string = '';
  @Param open: boolean = false;
  @Param first: boolean = false;
  @Param sessions: SessionSummary[] = [];
  @Event onNew: (cwd: string) => void = () => {};

  @Builder
  menu() {
    ProjectMenu({
      model: this.model, cwd: this.cwd, first: this.first,
      onNew: (cwd: string) => this.onNew(cwd),
      onClose: () => this.getUIContext().getContextMenuController().close()
    })
  }

  pressingNow(): Pressing | undefined {
    return this.open ? undefined : pressing(this.sessions);
  }

  build() {
    Row({ space: Metrics.gap }) {
      Stack() {
        Lucide({ name: this.open ? 'folderOpen' : 'folder', tint: $r('app.color.fg3'), extent: Metrics.icon })
        if (remoteAlias(this.cwd) !== undefined) {
          Stack() {
            Lucide({ name: 'globe', tint: $r('app.color.fg3'), extent: Metrics.icon / 2 })
          }
          .backgroundColor($r('app.color.bg0'))
          .borderRadius(Metrics.icon / 2)
          .position({ x: Metrics.icon / 2, y: Metrics.icon / 2 })
        }
      }
      .width(Metrics.rowMark)
      .margin({ right: Metrics.lead - Metrics.rowMark })
      Row({ space: ListMetrics.s1_5 }) {
        Text(projectName(this.cwd))
          .fontSize(Type.text1)
          .fontWeight(FontWeight.Medium)
          .fontColor($r('app.color.fg_strong'))
          .maxLines(1)
          .textOverflow({ overflow: TextOverflow.Ellipsis })
          .flexShrink(1)
        Lucide({ name: 'chevronRight', tint: $r('app.color.fg3'), extent: ListMetrics.chevron })
          .rotate({ angle: this.open ? 90 : 0 })
          .animation({ duration: this.model.motion ? ListMetrics.motionMs : 0 })
      }
      .layoutWeight(1)
      if (remoteAlias(this.cwd) !== undefined) {
        Text(remoteAlias(this.cwd) ?? '')
          .fontSize(Type.text3)
          .fontColor($r('app.color.fg3'))
          .maxLines(1)
          .constraintSize({ maxWidth: ListMetrics.hostAlias })
      }
      Stack() {
        if (this.pressingNow() !== undefined) {
          StateMark({ state: this.pressingNow()?.state ?? '', unread: this.pressingNow()?.unread ?? false, motion: this.model.motion })
        }
      }
      .width(ListMetrics.stateSlot)
      .height(ListMetrics.stateSlot)
    }
    .id(`project:${this.cwd}`)
    .constraintSize({ minHeight: ListMetrics.sheetRow })
    .padding({ left: Metrics.pad, right: Metrics.pad })
    .width('100%')
    .backgroundColor($r('app.color.bg0'))
    .stateStyles({ pressed: { .backgroundColor($r('app.color.active')) } })
    .onClick(() => this.model.lists.toggleOpen(this.cwd))
    .bindContextMenu(this.menu(), ResponseType.LongPress, {
      preview: MenuPreviewMode.NONE, enableArrow: false, backgroundColor: Color.Transparent, backgroundBlurStyle: BlurStyle.NONE
    })
  }
}

/** The project header's menu (the desktop project menu, less revealing the folder on the computer) */
@ComponentV2
export struct ProjectMenu {
  @Require @Param model: AppModel;
  @Param cwd: string = '';
  @Param first: boolean = false;
  @Event onNew: (cwd: string) => void = () => {};
  @Event onClose: () => void = () => {};
  @Local sections: boolean = false;

  pick(action: () => void): void {
    this.onClose();
    action();
  }

  current(): string | undefined {
    return this.model.lists.sections.find((s: SessionSection) => s.projects.includes(this.cwd))?.id;
  }

  build() {
    MenuCard() {
      if (!this.sections) {
        MenuRow({ text: t('session.project.newSession'), icon: 'squarePen', tag: 'project.new', onTap: () => this.pick(() => this.onNew(this.cwd)) })
        MenuGap()
        if (!this.first) {
          MenuRow({ text: t('mobile.project.moveTop'), icon: 'arrowUp', tag: 'project.top', onTap: () => this.pick(() => this.model.lists.moveProjectToTop(this.cwd)) })
        }
        // Sections are made on the desktop; the entry shows once there is one
        if (this.model.lists.sections.length > 0) {
          MenuRow({ text: t('session.section.moveTo'), icon: 'chevronRight', tag: 'project.section', onTap: () => { this.sections = true; } })
        }
        MenuRow({ text: t('session.project.copyPath'), icon: 'copy', tag: 'project.copyPath', onTap: () => this.pick(() => this.model.lists.copyPath(this.cwd)) })
        MenuGap()
        MenuRow({ text: t('session.project.remove'), icon: 'minus', tag: 'project.remove', onTap: () => this.pick(() => this.model.lists.removeProject(this.cwd)) })
      } else {
        MenuBack({ title: t('session.section.moveTo'), onTap: () => { this.sections = false; } })
        MenuGap()
        ForEach(this.model.lists.sections, (s: SessionSection) => {
          MenuRow({
            text: s.name.length > 0 ? s.name : t('session.section.untitled'),
            checked: s.id === this.current(),
            onTap: () => this.pick(() => {
              if (s.id !== this.current()) {
                this.model.lists.setProjectSection(this.cwd, s.id);
              }
            })
          })
        }, (s: SessionSection) => s.id)
        if (this.current() !== undefined) {
          MenuGap()
          MenuRow({ text: t('session.section.remove'), checked: false, icon: 'minus', onTap: () => this.pick(() => this.model.lists.setProjectSection(this.cwd, undefined)) })
        }
      }
    }
  }
}
