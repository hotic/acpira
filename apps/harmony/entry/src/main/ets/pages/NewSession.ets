import { curves } from '@kit.ArkUI';
import { AgentInfo, SessionSummary } from '../wire/Wire';
import { ListMetrics, Metrics, RingMetrics, Type } from '../design/Tokens';
import { AppModel } from '../store/AppModel';
import { t } from '../store/L10n';
import { folderLabel, folderSubtitle, Places } from '../store/SessionLogic';
import { AgentMark, Lucide } from '../components/Glyphs';
import { Composer } from '../components/Composer';
import { Lead, LeadButton } from '../components/ChatBar';
import { MenuBack, MenuCard, MenuGap, MenuRow, menuOptions } from '../components/Menu';
import { openTextPrompt, PromptArgs } from '../components/Dialogs';
import { Attachment } from '../wire/Wire';
import { Cmp } from '../design/Tokens';
import { ModelChoices } from '../logic/ModelCatalog';
import { attachBeans, Bean, modelBeans } from '../components/composer/Beans';
import { GooLayer } from '../components/composer/GooLayer';

// A new session (apps/ios Sessions/NewSessionScreen.swift, apps/android ui/NewSessionScreen.kt): the agent's mark in
// the middle with its name and the folder it starts in, both pickable; the first prompt from the composer starts it. A
// tap on the mark buds the other agents off it on a ring (AgentRing), clockwise from the top; a pick lands its mark in
// the middle. The folder capsule's menu holds the chats (where the engine keeps them), this computer's projects and each
// SSH host's as submenus, and a folder typed in. On a wide window it is the detail column's first page.

/** What the page is opened with: an agent and a folder to start in ('' asks for a chat) */
export class NewArgs {
  agent: string | undefined = undefined;
  cwd: string | undefined = undefined;
}

@ComponentV2
export struct NewSession {
  @Require @Param model: AppModel;
  @Param args: NewArgs = new NewArgs();
  @Param lead: Lead = Lead.Back;
  /** The ring stands out on launch (screenshots: the Previewer cannot tap) */
  @Param ringOnLaunch: boolean = false;
  @Event onBack: () => void = () => {};
  @Event onSidebar: () => void = () => {};
  @Event onStart: (agent: string, cwd: string | undefined, text: string) => void = () => {};
  @Local pickedAgent: string | undefined = undefined;
  /** A folder was picked (undefined then means a chat) */
  @Local picked: boolean = false;
  @Local pickedCwd: string | undefined = undefined;
  @Local ring: boolean = false;
  @Local folderMenu: boolean = false;
  @Local folderLevel: string | undefined = undefined;
  /** Where the drop sits on the page, so the ring over the dim lands on it */
  @Local dropX: number = 0;
  @Local dropY: number = 0;
  @Local pageX: number = 0;
  @Local pageY: number = 0;

  aboutToAppear(): void {
    if (this.ringOnLaunch) {
      setTimeout(() => {
        this.ring = true;
      }, 0);
    }
  }

  /** composer: the open chip's beans, acting on the picks the session will start with */
  beans(): Bean[] {
    const goo = this.model.newGoo;
    const agent = this.agent();
    const model = this.model;
    if (goo.last === 'attach') {
      return attachBeans(goo, false, [], () => {});
    }
    const choices = new ModelChoices(model.controlsFor(agent), model.hiddenModels,
      (id: string, value: string) => model.setNewConfig(agent, id, value));
    return modelBeans(choices, goo, {
      setMode: (mode: string) => model.setNewMode(agent, mode),
      setConfig: (id: string, value: string) => model.setNewConfig(agent, id, value)
    });
  }

  /** The asked agent, else the newest session's while it can still start, else the first that can, else the default */
  agent(): string {
    if (this.pickedAgent !== undefined) {
      return this.pickedAgent;
    }
    if (this.args.agent !== undefined && this.args.agent.length > 0) {
      return this.args.agent;
    }
    const startable = this.model.startable();
    const newest = this.model.summaries.filter((s: SessionSummary) => startable.some((a: AgentInfo) => a.id === s.agent))
      .sort((a: SessionSummary, b: SessionSummary) => b.updatedAt.localeCompare(a.updatedAt));
    if (newest.length > 0) {
      return newest[0].agent;
    }
    return startable.length > 0 ? startable[0].id : this.model.defaultAgent();
  }

  /** Folders added by hand lead, then the user's order, then the folders sessions ran in */
  projects(): string[] {
    const used = this.model.summaries.slice().sort((a: SessionSummary, b: SessionSummary) => b.updatedAt.localeCompare(a.updatedAt))
      .map((s: SessionSummary) => s.cwd);
    const out: string[] = [];
    this.model.addedProjects.concat(this.model.projectOrder).concat(used).forEach((c: string) => {
      if (c.length > 0 && !this.model.isChat(c) && !out.includes(c)) {
        out.push(c);
      }
    });
    return out;
  }

  /** The latest project unless asked otherwise; a chat when the computer has none */
  cwd(): string | undefined {
    if (this.picked) {
      return this.pickedCwd;
    }
    if (this.args.cwd === '') {
      return undefined;
    }
    const projects = this.projects();
    return this.args.cwd ?? (projects.length > 0 ? projects[0] : undefined);
  }

  others(): AgentInfo[] {
    const id = this.agent();
    return this.model.startable().filter((a: AgentInfo) => a.id !== id);
  }

  pickFolder(cwd: string | undefined): void {
    this.pickedCwd = cwd;
    this.picked = true;
    this.folderMenu = false;
  }

  addFolder(): void {
    this.folderMenu = false;
    const args = new PromptArgs();
    args.title = t('mobile.projects.typePath');
    args.hint = t('mobile.projects.pathHint');
    args.placeholder = t('mobile.projects.path');
    args.save = t('mobile.add');
    args.onSave = (text: string) => {
      this.model.addProject(text);
      this.pickFolder(text.trim());
    };
    openTextPrompt(this.getUIContext(), args);
  }

  build() {
    Stack() {
      Column() {
        Row({ space: Metrics.gap }) {
          LeadButton({ lead: this.lead, tag: 'new.back', onBack: () => this.onBack(), onSidebar: () => this.onSidebar() })
          Text(t('session.new'))
            .fontSize(Type.text1)
            .fontWeight(FontWeight.Medium)
            .fontColor($r('app.color.fg_strong'))
            .layoutWeight(1)
        }
        .padding({ left: Metrics.pad, right: Metrics.pad })
        .height(Metrics.barCtl + Metrics.gap)
        .width('100%')

        Column({ space: Metrics.gap }) {
          AgentRing({
            model: this.model, agent: this.agent(), others: this.others(), open: this.ring,
            onOpen: (open: boolean) => {
              this.ring = open;
            },
            onPick: (id: string) => {
              this.pickedAgent = id;
              this.ring = false;
            }
          })
            .margin({ bottom: Metrics.s3 })
            .onAreaChange((o: Area, n: Area) => {
              this.dropX = Number(n.globalPosition.x);
              this.dropY = Number(n.globalPosition.y);
            })
          Text(this.model.agentName(this.agent()))
            .id('new.agentName')
            .fontSize(Type.text1)
            .fontWeight(FontWeight.Bold)
            .fontColor($r('app.color.fg_strong'))
            .opacity(this.ring ? 0 : 1)
            .translate({ y: this.ring ? -Metrics.s3 : 0 })
            .animation({ duration: this.model.motion ? RingMetrics.heroAwayMs : 0 })
          Row({ space: Metrics.s2 }) {
            Lucide({ name: this.cwd() === undefined ? 'messageSquare' : 'folder', tint: $r('app.color.fg2'), extent: Metrics.icon })
            Text(this.cwd() === undefined ? t('session.group.chats') : this.model.placeName(this.cwd() ?? ''))
              .fontSize(Type.text1)
              .fontColor($r('app.color.fg2'))
              .maxLines(1)
          }
          .id('new.folder')
          .padding({ left: Metrics.pad, right: Metrics.pad, top: Metrics.s2, bottom: Metrics.s2 })
          .borderRadius(Metrics.barCtl / 2)
          .backgroundColor($r('app.color.glass_bg'))
          .border({ width: Metrics.hairline, color: $r('app.color.conversation_line') })
          .opacity(this.ring ? 0 : 1)
          .translate({ y: this.ring ? -Metrics.s3 : 0 })
          .animation({ duration: this.model.motion ? RingMetrics.heroAwayMs : 0 })
          .onClick(() => {
            if (!this.ring) {
              this.folderLevel = undefined;
              this.folderMenu = true;
            }
          })
          .bindMenu(this.folderMenu, this.folderCard(), menuOptions(() => { this.folderMenu = false; }, Placement.Bottom))
        }
        .justifyContent(FlexAlign.Center)
        .layoutWeight(1)
        .width('100%')
        // composer: the hero steps aside while the composer's beans are out
        .opacity(this.model.newGoo.open !== undefined ? 0 : 1)
        .animation({ duration: Cmp.quickMs })

        Row() {
          // composer: the agent's model, effort and modes as the composer's beans, set on the session it starts
          Composer({
            draft: this.model.newDraft,
            placeholder: t('mobile.new.placeholder'),
            controls: this.model.controlsFor(this.agent()),
            hidden: this.model.hiddenModels,
            commands: this.model.commandsFor(this.agent()),
            personas: this.model.personas,
            goo: this.model.newGoo,
            onSend: (text: string, staged: Attachment[]) => {
              this.model.prepareStart(this.agent(), staged);
              this.onStart(this.agent(), this.cwd(), text);
            }
          })
        }
        .justifyContent(FlexAlign.Center)
        .padding({ left: Metrics.pad, right: Metrics.pad, top: Metrics.s2, bottom: Metrics.s2 })
        .width('100%')
        .zIndex(2)

        // composer: the picker's backdrop under the composer, and its beans over everything
        Column()
          .position({ x: 0, y: 0 })
          .width('100%')
          .height('100%')
          .backgroundColor($r('app.color.goo_dim'))
          .opacity(this.model.newGoo.open !== undefined ? 1 : 0)
          .animation({ curve: Curve.Friction, duration: Cmp.quickMs * 2 })
          .hitTestBehavior(HitTestMode.None)
          .zIndex(1)
        GooLayer({ goo: this.model.newGoo, beans: this.beans() })
          .position({ x: 0, y: 0 })
          .width('100%')
          .height('100%')
          .zIndex(3)
          // The component's own wrapper node covers the page too: closed, it must let touches through like its Stack
          .hitTestBehavior(this.model.newGoo.open !== undefined ? HitTestMode.Default : HitTestMode.None)
      }
      // The page dims under an open ring; a tap on it folds the ring
      if (this.ring) {
        Column()
          .width('100%')
          .height('100%')
          .backgroundColor($r('app.color.goo_dim'))
          .transition(TransitionEffect.OPACITY.animation({ duration: this.model.motion ? RingMetrics.heroAwayMs : 0 }))
          .onClick(() => {
            this.ring = false;
          })
      }
      if (this.ring) {
        // The ring itself over the dim, at the drop's place
        AgentRing({
            model: this.model, agent: this.agent(), others: this.others(), open: true, floating: true,
            onOpen: (open: boolean) => {
              this.ring = open;
            },
            onPick: (id: string) => {
              this.pickedAgent = id;
              this.ring = false;
            }
          })
          .position({
            x: this.dropX - this.pageX - (ringSpan(this.others().length) - RingMetrics.heroTile) / 2,
            y: this.dropY - this.pageY - (ringSpan(this.others().length) - RingMetrics.heroTile) / 2
          })
          // The component's own wrapper node covers the ring's span too (docs/dev/harmony.md)
          .hitTestBehavior(HitTestMode.Transparent)
      }
    }
    .alignContent(Alignment.TopStart)
    .backgroundColor($r('app.color.bg0'))
    .height('100%')
    .width('100%')
    .onAreaChange((o: Area, n: Area) => {
      this.pageX = Number(n.globalPosition.x);
      this.pageY = Number(n.globalPosition.y);
    })
  }

  /**
   * The folder capsule's menu (apps/ios NativeProjectMenu): the chats on top, then this computer's projects and the SSH
   * hosts' as submenus, so a long project list never opens as one tall menu; a folder typed in at the end
   */
  @Builder
  folderCard() {
    MenuCard() {
      if (this.folderLevel === undefined) {
        if (this.model.chatRoot !== undefined) {
          MenuRow({ text: t('session.group.chats'), checked: this.cwd() === undefined, icon: 'messagesSquare', onTap: () => this.pickFolder(undefined) })
          MenuGap()
        }
        if (Places.of(this.projects()).local.length > 0) {
          MenuRow({
            text: t('mobile.projects.local'), checked: this.cwd() !== undefined && !(this.cwd() ?? '').startsWith('ssh://'),
            icon: 'chevronRight', tag: 'folder.local', onTap: () => { this.folderLevel = ''; }
          })
        }
        ForEach(Places.of(this.projects()).hosts, (h: string) => {
          MenuRow({
            text: h, subtitle: t('mobile.projects.remote'), checked: (this.cwd() ?? '').startsWith(`ssh://${h}/`), icon: 'chevronRight',
            onTap: () => { this.folderLevel = h; }
          })
        }, (h: string) => h)
        MenuGap()
        MenuRow({ text: t('mobile.projects.add'), checked: false, icon: 'folderPlus', tag: 'folder.add', onTap: () => this.addFolder() })
      } else {
        MenuBack({
          title: this.folderLevel.length === 0 ? t('mobile.projects.local') : this.folderLevel,
          onTap: () => { this.folderLevel = undefined; }
        })
        MenuGap()
        ForEach(this.folderLevel.length === 0 ? Places.of(this.projects()).local : (Places.of(this.projects()).byHost.get(this.folderLevel) ?? []),
          (p: string) => {
            MenuRow({
              text: folderLabel(p), subtitle: folderSubtitle(p), checked: p === this.cwd(),
              icon: (this.folderLevel ?? '').length === 0 ? 'folder' : 'globe', onTap: () => this.pickFolder(p)
            })
          }, (p: string) => p)
      }
    }
  }
}

/**
 * The agent ring (apps/ios NewSessionScreen AgentRing): the agent's mark in its round drop, which buds the other agents
 * off it on a ring, clockwise from the top, one after another; they fold back the last one first. The page draws one
 * ring in place and, while it is out, a floating copy over the dim (the dim covers the page but not the ring)
 */
@ComponentV2
struct AgentRing {
  @Require @Param model: AppModel;
  @Param agent: string = '';
  @Param others: AgentInfo[] = [];
  @Param open: boolean = false;
  /** The copy over the dim: its bubbles bud out as it appears */
  @Param floating: boolean = false;
  @Event onOpen: (open: boolean) => void = () => {};
  @Event onPick: (id: string) => void = () => {};
  /** Out on the ring: set a frame after appearing, so the bubbles grow out of the drop */
  @Local out: boolean = false;

  aboutToAppear(): void {
    if (this.floating) {
      setTimeout(() => {
        this.out = true;
      }, 0);
    }
  }

  radius(): number {
    return ringRadius(this.others.length);
  }

  spotX(i: number): number {
    return Math.cos(-Math.PI / 2 + 2 * Math.PI * i / Math.max(this.others.length, 1)) * this.radius();
  }

  spotY(i: number): number {
    return Math.sin(-Math.PI / 2 + 2 * Math.PI * i / Math.max(this.others.length, 1)) * this.radius();
  }

  lag(): number {
    return Math.min(RingMetrics.staggerMs, RingMetrics.staggerMaxMs / Math.max(this.others.length - 1, 1));
  }

  build() {
    Stack() {
      if (this.floating) {
        ForEach(this.others, (a: AgentInfo, i: number) => {
          Stack() {
            AgentMark({ agent: a.id, name: a.name, tint: $r('app.color.fg1'), extent: RingMetrics.mark })
            // Its name under it, in after the bubble
            Text(a.name)
              .fontSize(Type.text2)
              .fontWeight(FontWeight.Medium)
              .fontColor($r('app.color.fg_strong'))
              .maxLines(1)
              .textAlign(TextAlign.Center)
              .width(RingMetrics.pitch)
              .position({ x: (RingMetrics.bubble - RingMetrics.pitch) / 2, y: RingMetrics.bubble + Metrics.s1 })
          }
          .width(RingMetrics.bubble)
          .height(RingMetrics.bubble)
          .borderRadius(RingMetrics.bubble / 2)
          .backgroundColor($r('app.color.glass_bg'))
          .border({ width: Metrics.hairline, color: $r('app.color.conversation_line') })
          .id(`ring:${a.id}`)
          .translate({ x: this.out ? this.spotX(i) : 0, y: this.out ? this.spotY(i) : 0 })
          .scale({ x: this.out ? 1 : RingMetrics.seed, y: this.out ? 1 : RingMetrics.seed })
          .opacity(this.out ? 1 : 0)
          .animation({
            curve: curves.springMotion(),
            delay: this.model.motion ? i * this.lag() : 0,
            duration: this.model.motion ? RingMetrics.flightMs : 0
          })
          .onClick(() => this.onPick(a.id))
        }, (a: AgentInfo) => a.id)
      }
      // The drop: the current agent's mark
      Stack() {
        AgentMark({ agent: this.agent, name: this.model.agentName(this.agent), tint: $r('app.color.fg_strong'), extent: RingMetrics.heroBare })
      }
      .id('new.agent')
      .width(RingMetrics.heroTile)
      .height(RingMetrics.heroTile)
      .borderRadius(RingMetrics.heroTile / 2)
      .backgroundColor($r('app.color.glass_bg'))
      .border({ width: Metrics.hairline, color: $r('app.color.conversation_line') })
      .shadow({ radius: ListMetrics.barShadow, color: $r('app.color.line'), offsetY: Metrics.s1 / 2 })
      .onClick(() => this.onOpen(!this.open))
    }
    // A touch outside a parent's bounds never reaches its children, and `translate` leaves the bounds where they were:
    // the floating copy spans the whole ring, centred on the drop, and lets taps between the bubbles reach the dim
    .width(this.floating ? ringSpan(this.others.length) : RingMetrics.heroTile)
    .height(this.floating ? ringSpan(this.others.length) : RingMetrics.heroTile)
    .hitTestBehavior(this.floating ? HitTestMode.Transparent : HitTestMode.Default)
  }
}

/** The ring the bubbles sit on: one pitch apart, never tighter than the minimum */
function ringRadius(count: number): number {
  return Math.max(RingMetrics.radiusMin, RingMetrics.pitch * count / (2 * Math.PI));
}

/** The floating ring's side: the ring's diameter plus a bubble and its name each way */
function ringSpan(count: number): number {
  return 2 * (ringRadius(count) + RingMetrics.pitch);
}
