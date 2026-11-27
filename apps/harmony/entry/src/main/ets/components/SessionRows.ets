import { SessionSection, SessionSummary } from '../wire/Wire';
import { ListMetrics, Metrics, Type } from '../design/Tokens';
import { AppModel } from '../store/AppModel';
import { t } from '../store/L10n';
import { pressing, Pressing, projectName, remoteAlias, searchParts } from '../store/SessionLogic';
import { AgentMark, Lucide } from './Glyphs';
import { StateMark } from './StateMark';
import { SessionMenu } from './SessionMenu';
import { MenuBack, MenuCard, MenuGap, MenuRow } from './Menu';
import { openTextPrompt, PromptArgs } from './Dialogs';

// The session list's rows (pages/SessionList.ets; apps/ios Sessions/SessionRows.swift, apps/android
// ui/SessionRows.kt): a session, a project header and its menu, a section caption

/** `text` with a search's terms in the strong ink and weight (SessionRows.tsx Marked) */
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
export struct SessionRowItem {
  @Require @Param model: AppModel;
  @Require @Param session: SessionSummary;
  @Param place: string | undefined = undefined;
  @Param time: string | undefined = undefined;
  @Param snippet: string | undefined = undefined;
  @Param terms: string[] = [];
  @Param divider: boolean = false;
  @Param nested: boolean = false;
  @Param active: boolean = false;

  /** A finger is down on the row */
  @Local pressed: boolean = false;
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
    Column() {
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
      .constraintSize({ minHeight: this.place === undefined && this.snippet === undefined ? ListMetrics.sheetRow : ListMetrics.sessionRow })
      .padding({
        left: Metrics.pad - Metrics.s2 + (this.nested ? ListMetrics.nest : 0), right: Metrics.pad - Metrics.s2,
        top: this.snippet === undefined ? 0 : Metrics.s2, bottom: this.snippet === undefined ? 0 : Metrics.s2
      })
      .width('100%')
      // The sidebar's open chat: the desktop column's fill, rounded and held in from both edges
      .borderRadius(Metrics.rLg)
      .backgroundColor(this.active ? $r('app.color.active') : Color.Transparent)
    }
    .id(`session:${this.session.id}`)
    .padding({ left: Metrics.s2, right: Metrics.s2 })
    .width('100%')
    // A press lights the whole row. Kept by hand: a pressed state style stayed on after the tap opened the chat
    .backgroundColor(this.pressed ? $r('app.color.active') : $r('app.color.bg0'))
    .border({
      width: { bottom: this.divider ? Metrics.hairline : 0 },
      color: { bottom: $r('app.color.line') }
    })
    .onTouch((e: TouchEvent) => {
      if (e.type === TouchType.Down) {
        this.pressed = true;
      } else if (e.type === TouchType.Up || e.type === TouchType.Cancel) {
        this.pressed = false;
      }
    })
    .onClick(() => {
      this.pressed = false;
      this.onOpen(this.session.id);
    })
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
export struct ProjectHeader {
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
        MenuRow({ text: t('session.section.moveTo'), icon: 'chevronRight', tag: 'project.section', onTap: () => { this.sections = true; } })
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
        if (this.model.lists.sections.length > 0) {
          MenuGap()
        }
        MenuRow({ text: t('session.section.newEllipsis'), checked: false, icon: 'plus', tag: 'project.section.new', onTap: () => this.createSection() })
        if (this.current() !== undefined) {
          MenuRow({ text: t('session.section.remove'), checked: false, icon: 'minus', onTap: () => this.pick(() => this.model.lists.setProjectSection(this.cwd, undefined)) })
        }
      }
    }
  }

  createSection(): void {
    const ui = this.getUIContext();
    const lists = this.model.lists;
    const cwd = this.cwd;
    const args = new PromptArgs();
    args.title = t('session.section.new');
    args.placeholder = t('session.section.untitled');
    args.allowBlank = true;
    args.onSave = (text: string) => lists.createSection(text, cwd);
    this.pick(() => openTextPrompt(ui, args));
  }
}

/** A custom project section's caption: taps fold or unfold its projects, a long press renames, reorders or deletes it */
@ComponentV2
export struct SectionCaptionHeader {
  @Require @Param model: AppModel;
  @Param sectionId: string = '';
  @Param label: string = '';
  @Param open: boolean = true;
  @Param canMoveUp: boolean = false;
  @Param canMoveDown: boolean = false;

  closeMenu(): void {
    this.getUIContext().getContextMenuController().close();
  }

  rename(): void {
    const ui = this.getUIContext();
    const lists = this.model.lists;
    const id = this.sectionId;
    const initial = lists.sections.find((s: SessionSection) => s.id === id)?.name ?? '';
    const args = new PromptArgs();
    args.title = t('common.rename');
    args.initial = initial;
    args.placeholder = t('session.section.untitled');
    args.onSave = (text: string) => lists.renameSection(id, text);
    this.closeMenu();
    openTextPrompt(ui, args);
  }

  moveUp(): void {
    const secs = this.model.lists.sections;
    const at = secs.findIndex((s: SessionSection) => s.id === this.sectionId);
    this.closeMenu();
    if (at > 0) {
      this.model.lists.reorderSection(this.sectionId, secs[at - 1].id);
    }
  }

  moveDown(): void {
    const secs = this.model.lists.sections;
    const at = secs.findIndex((s: SessionSection) => s.id === this.sectionId);
    this.closeMenu();
    if (at >= 0 && at < secs.length - 1) {
      this.model.lists.reorderSection(this.sectionId, at + 2 < secs.length ? secs[at + 2].id : undefined);
    }
  }

  @Builder
  menu() {
    MenuCard() {
      MenuRow({ text: t('common.rename'), icon: 'pencil', tag: 'section.rename', onTap: () => this.rename() })
      if (this.canMoveUp) {
        MenuRow({ text: t('session.section.moveUp'), icon: 'arrowUp', tag: 'section.up', onTap: () => this.moveUp() })
      }
      if (this.canMoveDown) {
        MenuRow({ text: t('session.section.moveDown'), icon: 'arrowDown', tag: 'section.down', onTap: () => this.moveDown() })
      }
      MenuGap()
      MenuRow({
        text: t('session.section.delete'), icon: 'trash2', danger: true, tag: 'section.delete',
        onTap: () => {
          this.closeMenu();
          this.model.lists.deleteSection(this.sectionId);
        }
      })
    }
  }

  build() {
    Row({ space: ListMetrics.s1_5 }) {
      Text(this.label)
        .fontSize(Type.text3)
        .fontWeight(FontWeight.Medium)
        .fontColor($r('app.color.fg3'))
        .maxLines(1)
        .textOverflow({ overflow: TextOverflow.Ellipsis })
        .flexShrink(1)
      Lucide({ name: 'chevronRight', tint: $r('app.color.fg3'), extent: ListMetrics.chevron })
        .rotate({ angle: this.open ? 90 : 0 })
        .animation({ duration: this.model.motion ? ListMetrics.motionMs : 0 })
      Blank()
    }
    .id(`section:${this.sectionId}`)
    .alignItems(VerticalAlign.Bottom)
    .padding({ left: Metrics.pad, right: Metrics.pad, bottom: Metrics.s1 })
    .constraintSize({ minHeight: ListMetrics.groupCaption })
    .width('100%')
    .backgroundColor($r('app.color.bg0'))
    .onClick(() => this.model.lists.collapseSection(this.sectionId, this.open))
    .bindContextMenu(this.menu(), ResponseType.LongPress, {
      preview: MenuPreviewMode.NONE, enableArrow: false, backgroundColor: Color.Transparent, backgroundBlurStyle: BlurStyle.NONE
    })
  }
}
