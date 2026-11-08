import { bundleManager, common } from '@kit.AbilityKit';
import { notificationManager } from '@kit.NotificationKit';
import { AgentInfo } from '../wire/Wire';
import { FormMetrics, ListMetrics, Metrics, Type } from '../design/Tokens';
import { AppModel, Connection } from '../store/AppModel';
import { LOCALES, t } from '../store/L10n';
import { PrefKey, prefs } from '../store/Prefs';
import { AgentMark, Lucide } from '../components/Glyphs';
import { Lead, LeadButton } from '../components/ChatBar';
import { MenuCard, MenuRow, menuOptions } from '../components/Menu';
import { confirmDestructive, openTextPrompt, PromptArgs } from '../components/Dialogs';
import { PairedComputer } from '../remote/Stores';

// Settings (apps/ios Settings/SettingsScreen.swift and EngineSettings.swift, apps/android ui/SettingsScreen.kt and
// AgentSettings.kt): grouped sections on the form page. The look of the app (theme, language, motion), the agents (the
// default one, the list in the order every list follows), how sessions run, the engine the app follows, notifications,
// the version and the developer page. The phone's own choices persist in its preferences (store/Prefs.ets); the engine's
// are saved on the computer and shared with Acpira there.

/** What a settings page leads to: an agent's page, the developer page, a page still to come (by its title key) */
export class SettingsRoutes {
  agent: (id: string) => void = () => {};
  developer: () => void = () => {};
  coming: (title: string) => void = () => {};
  /** A paired computer's page (pages/Pairing.ets ComputerPage) */
  computer: (id: string) => void = () => {};
}

/** One choice of a pick row: its value and the name it reads as */
class Choice {
  value: string = '';
  name: string = '';
}

function choice(value: string, name: string): Choice {
  const c = new Choice();
  c.value = value;
  c.name = name;
  return c;
}

/** 0 first (each session's own model window), then the presets, the current value kept in its place */
function compactPresets(current: number): number[] {
  const out = [0, 100000, 200000, 300000, 400000, 600000, 800000, 1000000];
  if (!out.includes(current)) {
    out.push(current);
  }
  return out.sort((a: number, b: number) => a - b);
}

/** A pushed form page: the bar (back, the title centred) and the sections scrolling under it */
@ComponentV2
export struct FormPage {
  @Param title: string = '';
  @Param lead: Lead = Lead.Back;
  @Event onBack: () => void = () => {};
  @BuilderParam content: () => void;

  build() {
    Column() {
      Stack() {
        Text(this.title)
          .fontSize(Type.text1)
          .fontWeight(FontWeight.Bold)
          .fontColor($r('app.color.fg_strong'))
          .maxLines(1)
        Row() {
          LeadButton({ lead: this.lead, tag: 'settings.back', onBack: () => this.onBack() })
          Blank()
        }
        .width('100%')
        .padding({ left: Metrics.pad, right: Metrics.pad })
      }
      .height(Metrics.barCtl + Metrics.gap)
      .width('100%')
      Scroll() {
        Column({ space: FormMetrics.sectionGap }) {
          this.content()
        }
        .constraintSize({ maxWidth: Metrics.contentW })
        .padding({ left: Metrics.pad, right: Metrics.pad, top: Metrics.s2, bottom: Metrics.page })
        .width('100%')
      }
      .align(Alignment.Top)
      .scrollBar(BarState.Off)
      .edgeEffect(EdgeEffect.Spring)
      .layoutWeight(1)
    }
    .backgroundColor($r('app.color.form_page'))
    .height('100%')
    .width('100%')
  }
}

/** A titled group: the caption, the card, then a footnote under it */
@ComponentV2
export struct FormSection {
  @Param title: string = '';
  @Param footer: string = '';
  @BuilderParam content: () => void;

  build() {
    Column({ space: Metrics.s2 }) {
      if (this.title.length > 0) {
        Text(this.title)
          .fontSize(Type.text3)
          .fontWeight(FontWeight.Medium)
          .fontColor($r('app.color.fg3'))
          .padding({ left: Metrics.pad, right: Metrics.pad })
      }
      Column() {
        this.content()
      }
      .width('100%')
      .borderRadius(FormMetrics.radius)
      .backgroundColor($r('app.color.form_row'))
      .clip(true)
      if (this.footer.length > 0) {
        Text(this.footer)
          .fontSize(Type.text3)
          .fontColor($r('app.color.fg3'))
          .padding({ left: Metrics.pad, right: Metrics.pad })
      }
    }
    .alignItems(HorizontalAlign.Start)
    .width('100%')
  }
}

/** The hairline between rows, from the text's start (past the mark with `lead`) to the card's end */
@Builder
function rowRule(lead: boolean) {
  Divider()
    .color($r('app.color.line'))
    .strokeWidth(Metrics.hairline)
    .margin({ left: lead ? Metrics.pad + Metrics.rowMark + Metrics.gap : Metrics.pad })
}

/** A label with its value at the end, or a chevron when it leads somewhere */
@ComponentV2
struct ValueRow {
  @Param label: string = '';
  @Param value: string = '';
  @Param chevron: boolean = false;
  @Param danger: boolean = false;
  @Param dot: ResourceColor | undefined = undefined;
  @Param tag: string = '';
  @Event onTap: () => void = () => {};

  build() {
    Row({ space: Metrics.s2 }) {
      Text(this.label)
        .fontSize(Type.text1)
        .fontColor(this.danger ? $r('app.color.danger') : $r('app.color.fg1'))
        .maxLines(1)
        .textOverflow({ overflow: TextOverflow.Ellipsis })
        .layoutWeight(1)
      if (this.dot !== undefined) {
        Circle().width(FormMetrics.dot).height(FormMetrics.dot).fill(this.dot)
      }
      if (this.value.length > 0) {
        Text(this.value)
          .fontSize(Type.text1)
          .fontColor($r('app.color.fg2'))
          .maxLines(1)
          .textOverflow({ overflow: TextOverflow.Ellipsis })
          .flexShrink(1)
      }
      if (this.chevron) {
        Lucide({ name: 'chevronRight', tint: $r('app.color.fg3'), extent: ListMetrics.chevron })
      }
    }
    .id(this.tag)
    .constraintSize({ minHeight: FormMetrics.row })
    .padding({ left: Metrics.pad, right: Metrics.pad })
    .width('100%')
    .stateStyles({ pressed: { .backgroundColor($r('app.color.active')) }, normal: { .backgroundColor(Color.Transparent) } })
    .onClick(() => this.onTap())
  }
}

/** A row whose value opens a menu of choices at the row's end; the picked one carries a tick */
@ComponentV2
struct PickRow {
  @Param label: string = '';
  @Param value: string = '';
  @Param choices: Choice[] = [];
  @Param tag: string = '';
  @Event onPick: (value: string) => void = () => {};
  @Local open: boolean = false;

  name(): string {
    return this.choices.find((c: Choice) => c.value === this.value)?.name ?? this.value;
  }

  @Builder
  card() {
    MenuCard() {
      ForEach(this.choices, (c: Choice) => {
        MenuRow({
          text: c.name, checked: c.value === this.value, tag: `${this.tag}:${c.value}`, onTap: () => {
            this.open = false;
            if (c.value !== this.value) {
              this.onPick(c.value);
            }
          }
        })
      }, (c: Choice) => c.value)
    }
  }

  build() {
    Row({ space: Metrics.s2 }) {
      Text(this.label)
        .fontSize(Type.text1)
        .fontColor($r('app.color.fg1'))
        .maxLines(1)
        .layoutWeight(1)
      Row({ space: Metrics.s1 }) {
        Text(this.name())
          .fontSize(Type.text1)
          .fontColor($r('app.color.fg2'))
          .maxLines(1)
        Lucide({ name: 'chevronsUpDown', tint: $r('app.color.fg3'), extent: ListMetrics.chevron })
      }
      .bindMenu(this.open, this.card(), menuOptions(() => { this.open = false; }))
    }
    .id(this.tag)
    .constraintSize({ minHeight: FormMetrics.row })
    .padding({ left: Metrics.pad, right: Metrics.pad })
    .width('100%')
    .onClick(() => {
      this.open = true;
    })
  }
}

/** A switch row: the whole row flips it; the switch keeps the system's green so "on" reads as on */
@ComponentV2
struct SwitchRow {
  @Param label: string = '';
  @Param on: boolean = false;
  @Param active: boolean = true;
  @Param tag: string = '';
  @Event onChange: (on: boolean) => void = () => {};

  build() {
    Row({ space: Metrics.s2 }) {
      Text(this.label)
        .fontSize(Type.text1)
        .fontColor(this.active ? $r('app.color.fg1') : $r('app.color.fg3'))
        .maxLines(1)
        .textOverflow({ overflow: TextOverflow.Ellipsis })
        .layoutWeight(1)
      Toggle({ type: ToggleType.Switch, isOn: this.on })
        .selectedColor($r('app.color.switch_on'))
        .enabled(this.active)
        .hitTestBehavior(HitTestMode.None)
    }
    .id(this.tag)
    .constraintSize({ minHeight: FormMetrics.row })
    .padding({ left: Metrics.pad, right: Metrics.pad })
    .width('100%')
    .onClick(() => {
      if (this.active) {
        this.onChange(!this.on);
      }
    })
  }
}

/** The settings form */
@ComponentV2
export struct SettingsPage {
  @Require @Param model: AppModel;
  @Param routes: SettingsRoutes = new SettingsRoutes();
  @Param lead: Lead = Lead.Back;
  @Event onBack: () => void = () => {};
  @Local notify: string = 'mobile.notify.ask';

  aboutToAppear(): void {
    this.notify = this.notifyState();
  }

  /** On, off, or not asked yet */
  notifyState(): string {
    try {
      if (notificationManager.isNotificationEnabledSync()) {
        return 'mobile.notify.on';
      }
    } catch (e) {
      // No notification service (the Previewer)
    }
    return prefs.bool(PrefKey.notifyAsked, false) ? 'mobile.notify.off' : 'mobile.notify.ask';
  }

  askNotify(): void {
    prefs.set(PrefKey.notifyAsked, true);
    try {
      notificationManager.requestEnableNotification(this.getUIContext().getHostContext() as common.UIAbilityContext).then(() => {
        this.notify = this.notifyState();
      }).catch(() => {
        this.notify = this.notifyState();
      });
    } catch (e) {
      this.notify = this.notifyState();
    }
  }

  version(): string {
    try {
      const info = bundleManager.getBundleInfoForSelfSync(bundleManager.BundleFlag.GET_BUNDLE_INFO_DEFAULT);
      return `${info.versionName} (${info.versionCode})`;
    } catch (e) {
      return '';
    }
  }

  /** The engine's address and token, typed anew: the app follows it from then on */
  editEngine(): void {
    const args = new PromptArgs();
    args.title = t('mobile.pair.connectTitle');
    args.placeholder = 'host:port';
    args.initial = this.model.engineAddress;
    args.secondPlaceholder = 'token';
    args.save = t('mobile.pair.connect');
    args.onSave = (address: string, token: string) => {
      const a = address.trim();
      if (a.split(':').length === 2 && token.trim().length > 0) {
        this.model.follow(a, token.trim(), true);
      }
    };
    openTextPrompt(this.getUIContext(), args);
  }

  build() {
    FormPage({ title: t('session.settings'), lead: this.lead, onBack: () => this.onBack() }) {
      // Everything about how the app looks and reads in one group
      FormSection({ title: t('settings.appearance.title') }) {
        PickRow({
          label: t('settings.theme'), value: this.model.theme, tag: 'settings.theme',
          choices: [choice('system', t('mobile.theme.system')), choice('light', t('settings.theme.light')), choice('dark', t('settings.theme.dark'))],
          onPick: (v: string) => this.model.pickTheme(v)
        })
        rowRule(false)
        PickRow({
          label: t('settings.language'), value: this.model.language, tag: 'settings.language',
          choices: [choice('auto', t('mobile.language.system'))].concat(LOCALES.map((l: string) => choice(l, t(`settings.language.${l}`)))),
          onPick: (v: string) => this.model.pickLanguage(v)
        })
        rowRule(false)
        SwitchRow({ label: t('settings.motion'), on: this.model.motion, tag: 'settings.motion', onChange: (on: boolean) => this.model.pickMotion(on) })
      }
      // The engine's own settings, shared with the computer
      FormSection({ title: t('mobile.settings.agents') }) {
        PickRow({
          label: t('settings.defaultAgent'), value: this.model.defaultAgent(), tag: 'settings.defaultAgent',
          choices: this.model.agents.filter((a: AgentInfo) => a.disabled !== true).map((a: AgentInfo) => choice(a.id, a.name)),
          onPick: (v: string) => this.model.setDefaultAgent(v)
        })
      }
      FormSection({ footer: t('mobile.settings.agents.desc') }) {
        this.agentList()
      }
      FormSection({ title: t('mobile.settings.sessions'), footer: t('mobile.settings.engine.desc') }) {
        PickRow({
          label: t('settings.accountSwitch'), value: this.model.settings.accountSwitch, tag: 'settings.accountSwitch',
          choices: ['off', 'earliestReset', 'mostRemaining', 'listOrder'].map((v: string) => choice(v, t(`settings.accountSwitch.${v}`))),
          onPick: (v: string) => this.model.setAccountSwitch(v)
        })
        rowRule(false)
        SwitchRow({
          label: t('settings.steerQueued'), on: this.model.settings.steerQueued, tag: 'settings.steerQueued',
          onChange: (on: boolean) => this.model.setSteerQueued(on)
        })
        rowRule(false)
        SwitchRow({
          label: t('settings.autoCompact'), on: this.model.settings.autoCompact, tag: 'settings.autoCompact',
          onChange: (on: boolean) => this.model.setAutoCompact(on)
        })
        if (this.model.settings.autoCompact) {
          rowRule(false)
          // Presets in the menu instead of a number field; a value set on the computer stays listed
          PickRow({
            label: t('settings.compactAt'), value: `${this.model.settings.compactAtTokens}`, tag: 'settings.compactAt',
            choices: compactPresets(this.model.settings.compactAtTokens).map((n: number) =>
              choice(`${n}`, n === 0 ? t('settings.compactAt.model') : `${n / 1000}${t('settings.compactAt.unit')}`)),
            onPick: (v: string) => this.model.setCompactAt(Number(v))
          })
        }
      }
      // Paired remote access in place of the engine by address: every computer with how it is reached, then add one
      if (this.model.pairing !== undefined && this.model.engineAddress.length === 0) {
        FormSection({ title: t('mobile.pair.computers'), footer: t('mobile.pair.computers.desc') }) {
          ForEach(this.model.pairing.computers, (c: PairedComputer) => {
            ValueRow({
              label: c.name, value: this.model.pairing?.status(c) ?? '', chevron: true, tag: `settings.computer:${c.id}`,
              dot: this.model.pairing?.online(c) === true ? $r('app.color.ok') : undefined,
              onTap: () => this.routes.computer(c.id)
            })
            rowRule(false)
          }, (c: PairedComputer) => `${c.id}:${c.name}`)
          ValueRow({
            label: t('mobile.pair.add'), chevron: true, tag: 'settings.computers.add', onTap: () => {
              if (this.model.pairing !== undefined) {
                this.model.pairing.addOpen = true;
              }
            }
          })
          rowRule(false)
          // An engine by address instead (the development engine): it takes over from the computers
          ValueRow({ label: t('connections.address'), chevron: true, tag: 'settings.engine.address', onTap: () => this.editEngine() })
        }
      } else {
        // The engine followed: one fact, the state with its dot, then where it is; what it means goes in the footnote
        FormSection({ title: t('mobile.engine'), footer: this.model.live ? '' : t('mobile.engine.desc') }) {
          ValueRow({
            label: t('mobile.engine.status'), tag: 'settings.engine',
            value: t(this.model.connection === Connection.Live ? 'connections.state.connected' :
              this.model.connection === Connection.Connecting ? 'connections.state.connecting' : 'connections.state.off'),
            dot: this.model.connection === Connection.Live ? $r('app.color.ok') : $r('app.color.fg3')
          })
          rowRule(false)
          ValueRow({
            label: t('connections.address'), tag: 'settings.engine.address',
            value: this.model.engineAddress.length > 0 ? this.model.engineAddress : t('mobile.pair.none'),
            chevron: true, onTap: () => this.editEngine()
          })
          if (this.model.engineAddress.length > 0) {
            rowRule(false)
            ValueRow({
              label: t('mobile.pair.remove'), danger: true, tag: 'settings.engine.remove',
              onTap: () => confirmDestructive(this.getUIContext(),
                t('mobile.pair.remove.confirm', new Map<string, string>([['name', this.model.engineAddress]])),
                t('common.delete'), () => this.model.forgetEngine())
            })
          }
        }
      }
      // Notices about the engine's sessions; the demo has none
      if (this.model.live) {
        FormSection({ footer: t('mobile.notify.desc') }) {
          ValueRow({
            label: t('mobile.notify'), value: t(this.notify), tag: 'settings.notify', onTap: () => {
              if (this.notify === 'mobile.notify.ask') {
                this.askNotify();
              }
            }
          })
        }
      }
      FormSection({ title: t('mobile.about') }) {
        ValueRow({ label: t('settings.fact.version'), value: this.version() })
        rowRule(false)
        // Developer tools sit under the version, one tap away
        ValueRow({ label: t('mobile.dev'), chevron: true, tag: 'settings.developer', onTap: () => this.routes.developer() })
      }
    }
  }

  /**
   * Every agent laid out, as the system's Settings lists its apps: a row opens that agent's page, touch and hold lifts
   * it to drag into the order every list follows
   */
  @Builder
  agentList() {
    List() {
      ForEach(this.model.agents, (a: AgentInfo, i: number) => {
        ListItem() {
          Column() {
            if (i > 0) {
              rowRule(true)
            }
            Row({ space: Metrics.gap }) {
              AgentMark({ agent: a.id, name: a.name, tint: $r('app.color.fg2'), extent: Metrics.rowMark })
              Text(a.name)
                .fontSize(Type.text1)
                .fontColor($r('app.color.fg1'))
                .maxLines(1)
                .layoutWeight(1)
              if (a.disabled === true) {
                Text(t('mobile.settings.agentOff')).fontSize(Type.text1).fontColor($r('app.color.fg2'))
              }
              Lucide({ name: 'chevronRight', tint: $r('app.color.fg3'), extent: ListMetrics.chevron })
            }
            .constraintSize({ minHeight: FormMetrics.row })
            .padding({ left: Metrics.pad, right: Metrics.pad })
            .width('100%')
          }
          .backgroundColor($r('app.color.form_row'))
        }
        .id(`settings.agent:${a.id}`)
        .onClick(() => this.routes.agent(a.id))
      }, (a: AgentInfo, i: number) => `${a.id}:${a.disabled === true}:${i}`)
        .onMove((from: number, to: number) => this.model.moveAgent(from, to))
    }
    .scrollBar(BarState.Off)
    .edgeEffect(EdgeEffect.None)
    .width('100%')
    .height(this.model.agents.length * (FormMetrics.row + Metrics.hairline))
  }
}

/** One agent: on or off for new sessions, Plan-mode approval, and the models its menus show */
@ComponentV2
export struct AgentSettingsPage {
  @Require @Param model: AppModel;
  @Param agentId: string = '';
  @Param routes: SettingsRoutes = new SettingsRoutes();
  @Event onBack: () => void = () => {};

  agent(): AgentInfo | undefined {
    return this.model.agents.find((a: AgentInfo) => a.id === this.agentId);
  }

  build() {
    FormPage({ title: this.model.agentName(this.agentId), onBack: () => this.onBack() }) {
      FormSection() {
        // The last agent on stays on: a new session needs one
        SwitchRow({
          label: t('settings.agent.enable'), on: this.agent()?.disabled !== true, tag: 'agent.enable',
          active: !(this.model.agents.filter((a: AgentInfo) => a.disabled !== true).length === 1 && this.agent()?.disabled !== true),
          onChange: (on: boolean) => this.model.setAgentEnabled(this.agentId, on)
        })
      }
      FormSection({
        title: t('settings.agent.permissions'),
        footer: t('settings.planAutoApprove.desc', new Map<string, string>([['agent', this.model.agentName(this.agentId)]]))
      }) {
        SwitchRow({
          label: t('settings.planAutoApprove'), on: this.model.settings.planAutoApprove.includes(this.agentId), tag: 'agent.planAutoApprove',
          onChange: (on: boolean) => this.model.setPlanAutoApprove(this.agentId, on)
        })
      }
      if (this.agentId === 'acpira') {
        FormSection() {
          ValueRow({ label: t('settings.providers'), chevron: true, tag: 'agent.providers', onTap: () => this.routes.coming('settings.providers') })
        }
      }
      FormSection({ footer: t('settings.models.desc') }) {
        ValueRow({ label: t('settings.models.selection'), chevron: true, tag: 'agent.models', onTap: () => this.routes.coming('settings.models.selection') })
      }
    }
  }
}

/** Developer: the frame-rate and hitch overlay's switches */
@ComponentV2
export struct DeveloperPage {
  @Event onBack: () => void = () => {};
  @Local fps: boolean = false;
  @Local hitches: boolean = false;

  aboutToAppear(): void {
    this.fps = prefs.bool(PrefKey.hudFps, false);
    this.hitches = prefs.bool(PrefKey.hudHitches, false);
  }

  build() {
    FormPage({ title: t('mobile.dev'), onBack: () => this.onBack() }) {
      FormSection({ footer: t('mobile.dev.desc') }) {
        SwitchRow({
          label: t('mobile.dev.fps'), on: this.fps, tag: 'developer.fps', onChange: (on: boolean) => {
            this.fps = on;
            prefs.set(PrefKey.hudFps, on);
          }
        })
        rowRule(false)
        SwitchRow({
          label: t('mobile.dev.hitches'), on: this.hitches, tag: 'developer.hitches', onChange: (on: boolean) => {
            this.hitches = on;
            prefs.set(PrefKey.hudHitches, on);
          }
        })
      }
    }
  }
}

/** A page another part of the app builds later: its title over a word that it is coming */
@ComponentV2
export struct ComingPage {
  @Param title: string = '';
  @Param lead: Lead = Lead.Back;
  @Event onBack: () => void = () => {};

  build() {
    Column() {
      Row({ space: Metrics.gap }) {
        LeadButton({ lead: this.lead, tag: 'coming.back', onBack: () => this.onBack() })
        Text(this.title)
          .fontSize(Type.text1)
          .fontWeight(FontWeight.Medium)
          .fontColor($r('app.color.fg_strong'))
          .maxLines(1)
          .layoutWeight(1)
      }
      .padding({ left: Metrics.pad, right: Metrics.pad })
      .height(Metrics.barCtl + Metrics.gap)
      .width('100%')
      Column({ space: Metrics.gap }) {
        Lucide({ name: 'clock', tint: $r('app.color.fg3'), extent: Metrics.barCtl })
        Text(t('devices.soon.badge'))
          .id('coming.note')
          .fontSize(Type.text2)
          .fontColor($r('app.color.fg3'))
      }
      .justifyContent(FlexAlign.Center)
      .layoutWeight(1)
    }
    .backgroundColor($r('app.color.bg0'))
    .height('100%')
    .width('100%')
  }
}
