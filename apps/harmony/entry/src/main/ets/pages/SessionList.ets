import { SessionSummary } from '../wire/Wire';
import { Metrics, Type } from '../design/Tokens';
import { AppModel, Connection } from '../store/AppModel';
import { t } from '../store/L10n';
import { AgentMark, Lucide } from '../components/Glyphs';

// The session list (apps/ios Sessions/SessionListView.swift): pinned first, then by when each was last touched, one
// row per session with its agent's mark, its title and project, and its time or state on the right. The search field
// and the new-session button sit at the bottom, in thumb reach.

const DAY_MS: number = 86400000;

class Section {
  key: string = '';
  title: string = '';
  rows: SessionSummary[] = [];
}

function startOfDay(ms: number): number {
  const d = new Date(ms);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
}

function sections(list: SessionSummary[], query: string, now: number): Section[] {
  const q = query.trim().toLowerCase();
  const shown = list.filter((s: SessionSummary) => s.archived !== true && (q.length === 0 || s.title.toLowerCase().includes(q)));
  const today = startOfDay(now);
  const buckets: Section[] = [
    { key: 'pinned', title: t('session.group.pinned'), rows: [] } as Section,
    { key: 'today', title: t('session.when.today'), rows: [] } as Section,
    { key: 'yesterday', title: t('session.when.yesterday'), rows: [] } as Section,
    { key: 'week', title: t('session.when.week'), rows: [] } as Section,
    { key: 'month', title: t('session.when.month'), rows: [] } as Section,
    { key: 'older', title: t('session.when.older'), rows: [] } as Section
  ];
  for (const s of shown) {
    const at = Date.parse(s.updatedAt);
    let i = 5;
    if (s.pinned === true) {
      i = 0;
    } else if (at >= today) {
      i = 1;
    } else if (at >= today - DAY_MS) {
      i = 2;
    } else if (at >= today - 7 * DAY_MS) {
      i = 3;
    } else if (at >= today - 30 * DAY_MS) {
      i = 4;
    }
    buckets[i].rows.push(s);
  }
  return buckets.filter((b: Section) => b.rows.length > 0);
}

function project(cwd: string): string {
  const parts = cwd.split('/').filter((p: string) => p.length > 0);
  return parts.length > 0 ? parts[parts.length - 1] : cwd;
}

function clock(iso: string, now: number): string {
  const d = new Date(Date.parse(iso));
  if (d.getTime() >= startOfDay(now)) {
    const h = d.getHours();
    const m = `${d.getMinutes()}`.padStart(2, '0');
    return `${h % 12 === 0 ? 12 : h % 12}:${m} ${h < 12 ? 'AM' : 'PM'}`;
  }
  return `${d.getMonth() + 1}/${d.getDate()}`;
}

@ComponentV2
struct SessionRow {
  @Param session: SessionSummary = { id: '', title: '', agent: '', cwd: '', updatedAt: '' } as SessionSummary;
  @Param agentName: string = '';
  @Param now: number = 0;
  @Param active: boolean = false;

  build() {
    Row({ space: Metrics.gap }) {
      AgentMark({ agent: this.session.agent, name: this.agentName, tint: $r('app.color.fg2'), extent: Metrics.rowMark })
      Column({ space: Metrics.s1 / 2 }) {
        Text(this.session.title)
          .fontSize(Type.text1)
          .fontColor($r('app.color.fg1'))
          .maxLines(1)
          .textOverflow({ overflow: TextOverflow.Ellipsis })
        Text(project(this.session.cwd))
          .fontSize(Type.text3)
          .fontColor($r('app.color.fg3'))
          .maxLines(1)
      }
      .alignItems(HorizontalAlign.Start)
      .layoutWeight(1)
      if (this.session.state === 'working') {
        LoadingProgress().color($r('app.color.fg3')).width(Metrics.iconCtl).height(Metrics.iconCtl)
      } else if (this.session.state === 'waiting' || this.session.state === 'error' ||
        this.session.state === 'unread' || this.session.unread === true) {
        Circle()
          .width(Metrics.s2)
          .height(Metrics.s2)
          .fill(this.session.state === 'waiting' ? $r('app.color.brand') :
            this.session.state === 'error' ? $r('app.color.danger') : $r('app.color.command'))
      } else {
        Text(clock(this.session.updatedAt, this.now))
          .fontSize(Type.text3)
          .fontColor($r('app.color.fg3'))
      }
    }
    .padding({ left: Metrics.pad, right: Metrics.pad, top: Metrics.s3, bottom: Metrics.s3 })
    .backgroundColor(this.active ? $r('app.color.active') : Color.Transparent)
    .borderRadius(Metrics.rLg)
    .width('100%')
  }
}

/**
 * A row's identity for ForEach: everything it draws, so a changed summary renders again. A group's key joins its rows'
 * (a group whose key stays keeps the rows it was built with, however its summaries changed)
 */
function rowKey(s: SessionSummary): string {
  return `${s.id}:${s.title}:${s.state ?? ''}:${s.unread === true}:${s.updatedAt}:${s.pinned === true}`;
}

@ComponentV2
export struct SessionList {
  @Require @Param model: AppModel;
  @Param openId: string | undefined = undefined;
  @Event onOpen: (id: string) => void = () => {};
  @Event onNew: () => void = () => {};
  @Local query: string = '';

  build() {
    Column() {
      // The bar: the title centred, settings on the right (the system bar on iOS)
      Stack() {
        Text(t('mobile.home'))
          .fontSize(Type.barTitle)
          .fontWeight(FontWeight.Medium)
          .fontColor($r('app.color.fg1'))
        Row() {
          Blank()
          Lucide({ name: 'settings', tint: $r('app.color.fg1'), extent: Metrics.iconCtl })
        }
        .width('100%')
        .padding({ right: Metrics.pad })
      }
      .height(Metrics.barCtl + Metrics.s2)
      .width('100%')

      if (this.model.connection === Connection.Connecting && this.model.summaries.length === 0) {
        Column({ space: Metrics.gap }) {
          Lucide({ name: 'laptop', tint: $r('app.color.fg2'), extent: Metrics.barCtl })
          Row({ space: Metrics.s2 }) {
            LoadingProgress().color($r('app.color.fg3')).width(Metrics.iconCtl).height(Metrics.iconCtl)
            Text(t('host.working')).fontSize(Type.text2).fontColor($r('app.color.fg2'))
          }
        }
        .justifyContent(FlexAlign.Center)
        .layoutWeight(1)
      } else {
        List() {
          ForEach(sections(this.model.summaries, this.query, this.model.now()), (section: Section) => {
            ListItemGroup({ header: this.header(section.title) }) {
              ForEach(section.rows, (s: SessionSummary) => {
                ListItem() {
                  SessionRow({ session: s, agentName: this.model.agentName(s.agent), now: this.model.now(), active: s.id === this.openId })
                }
                .id(`session:${s.id}`)
                .onClick(() => this.onOpen(s.id))
              }, (s: SessionSummary) => rowKey(s))
            }
          }, (section: Section) => `${section.key}:${section.rows.map((s: SessionSummary) => rowKey(s)).join('|')}`)
        }
        .id('list.sessions')
        .alignListItem(ListItemAlign.Center)
        .padding({ left: Metrics.s1, right: Metrics.s1 })
        .scrollBar(BarState.Off)
        .layoutWeight(1)
      }

      // The bottom bar: search, and a new session in the latest project
      Row({ space: Metrics.gap }) {
        Row({ space: Metrics.s2 + Metrics.s1 }) {
          Lucide({ name: 'search', tint: $r('app.color.fg1'), extent: Metrics.iconCtl })
          TextInput({ placeholder: t('session.searchContent'), text: this.query })
            .id('list.search')
            .fontSize(Type.text1)
            .placeholderColor($r('app.color.fg3'))
            .placeholderFont({ size: Type.text1 })
            .backgroundColor(Color.Transparent)
            .padding(0)
            .cancelButton({ style: CancelButtonStyle.INPUT })
            .layoutWeight(1)
            .height('100%')
            .onChange((v: string) => {
              this.query = v;
            })
        }
        .padding({ left: Metrics.pad, right: Metrics.pad })
        .height(Metrics.barCtl)
        .borderRadius(Metrics.barCtl / 2)
        .backgroundColor($r('app.color.bg1'))
        .shadow({ radius: Metrics.gap, color: $r('app.color.line'), offsetY: Metrics.s1 })
        .layoutWeight(1)
        Stack() {
          Lucide({ name: 'squarePen', tint: $r('app.color.fg1'), extent: Metrics.iconCtl })
        }
        .id('list.new')
        .width(Metrics.barCtl)
        .height(Metrics.barCtl)
        .borderRadius(Metrics.barCtl / 2)
        .backgroundColor($r('app.color.bg1'))
        .shadow({ radius: Metrics.gap, color: $r('app.color.line'), offsetY: Metrics.s1 })
        .onClick(() => this.onNew())
      }
      .padding({ left: Metrics.pad, right: Metrics.pad, top: Metrics.s2, bottom: Metrics.s2 })
      .constraintSize({ maxWidth: Metrics.contentW })
      .width('100%')
    }
    .backgroundColor($r('app.color.bg0'))
    .height('100%')
    .width('100%')
  }

  @Builder
  header(title: string) {
    Text(title)
      .fontSize(Type.text3)
      .fontColor($r('app.color.fg3'))
      .padding({ left: Metrics.pad, top: Metrics.gap, bottom: Metrics.s1 })
      .width('100%')
      .backgroundColor($r('app.color.bg0'))
  }
}
