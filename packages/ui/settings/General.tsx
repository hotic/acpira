import { useEffect, useState, type KeyboardEvent } from 'react';
import { useDesktop } from '../desktop';
import type { AgentInfo } from '@acpira/shared/transcript';
import { ACCOUNT_SWITCH_STRATEGIES, AGENT_CPU_CAP, AUTO_PROXY_URL, FILE_OPENER_NAMES, FILE_OPENERS, MIN_COMPACT_AT_TOKENS, PANEL_SCOPES, proxyUrl, SESSION_SCOPES, type AccountSwitchStrategy, type FileOpener, type PanelScope, type SessionScope, type SettingsView } from '@acpira/shared/settings';
import { LANGUAGES, type Language } from '@acpira/shared/i18n';
import { launchable, pickDefaultAgent } from '@acpira/shared/agentOrder';
import { AgentMark } from '../chat/AgentMark';
import { playChime } from '../effects/chime';
import { t } from '../i18n';
import { Field, NumberField, Section, Select, Switch } from './controls';
import type { SettingsHandlers } from './SettingsShell';

// The threshold field works in thousands of tokens
const K = 1000;

// General: the shell's own knobs. Rendering preferences live on the Appearance page; the LAB appearance axes (bar motion) stay design decisions.
// The settings shell owns the page heading and content measure.
export function General({ settings, agents, on }: { settings: SettingsView; agents: AgentInfo[]; on: SettingsHandlers }) {
  const desktop = useDesktop();
  // `auto` follows the host's language: the editor's in an IDE, the system's in the desktop app (app.getLocale)
  const languages = LANGUAGES.map(l => ({ value: l, label: t(desktop && l === 'auto' ? 'settings.language.system' : `settings.language.${l}` as const) }));
  // Switched-off agents are not offered; a default that was switched off reads as the agent new sessions actually fall back to
  const agentOptions = launchable(agents).map(a => ({ value: a.id, label: a.name, icon: <AgentMark id={a.id} name={a.name} />, disabled: a.available === false }));
  const scopes = SESSION_SCOPES.map(s => ({ value: s, label: t(`settings.sessionScope.${s}` as const) }));
  const switchStrategies = ACCOUNT_SWITCH_STRATEGIES.map(s => ({ value: s, label: t(`settings.accountSwitch.${s}` as const) }));
  const fileOpeners = FILE_OPENERS.map(o => ({ value: o, label: o === 'system' ? t('settings.openFilesWith.system') : FILE_OPENER_NAMES[o] }));
  return (
    <>
      <Section title={t('settings.group.basic')} desc={t('settings.group.basic.desc')}>
        <Field label={t('settings.language')} desc={t('settings.language.desc')}>
          <Select<Language> options={languages} value={settings.language} onChange={v => on.setSetting('language', v)} label={t('settings.language')} />
        </Field>
        <Field label={t('settings.defaultAgent')} desc={t('settings.defaultAgent.desc')}>
          <Select options={agentOptions} value={pickDefaultAgent(agents, settings.defaultAgent)} onChange={v => on.setSetting('defaultAgent', v)} label={t('settings.defaultAgent')} />
        </Field>
        {/* The desktop app always lists every project, grouped (rust/crates/acpira-desktop/src/settings.rs seeds `all`) */}
        {!desktop && <Field label={t('settings.sessionScope')} desc={t('settings.sessionScope.desc')}>
          <Select<SessionScope> options={scopes} value={settings.sessionScope} onChange={v => on.setSetting('sessionScope', v)} label={t('settings.sessionScope')} />
        </Field>}
        {/* IDE shells open files in their own editor; the desktop app asks the system or an editor's command line */}
        {desktop && <>
          <Field label={t('settings.panelScope')} desc={t('settings.panelScope.desc')}>
            <Select<PanelScope> options={PANEL_SCOPES.map(v => ({ value: v, label: t(`settings.panelScope.${v}`) }))} value={settings.panelScope ?? 'project'} onChange={v => on.setSetting('panelScope', v)} label={t('settings.panelScope')} />
          </Field>
          <Field label={t('settings.openFilesWith')} desc={t('settings.openFilesWith.desc')}>
            <Select<FileOpener> options={fileOpeners} value={settings.openFilesWith} onChange={v => on.setSetting('openFilesWith', v)} label={t('settings.openFilesWith')} />
          </Field>
        </>}
      </Section>

      <Section title={t('settings.group.input')} desc={t('settings.group.input.desc')}>
        <Field label={t('settings.shareEditorSelection')} desc={t('settings.shareEditorSelection.desc')}>
          <Switch checked={settings.shareEditorSelection} onChange={v => on.setSetting('shareEditorSelection', v)} label={t('settings.shareEditorSelection')} />
        </Field>
        <Field label={t('settings.steerQueued')} desc={t('settings.steerQueued.desc')}>
          <Switch checked={settings.steerQueued} onChange={v => on.setSetting('steerQueued', v)} label={t('settings.steerQueued')} />
        </Field>
      </Section>

      <Section title={t('settings.compaction.title')} desc={t('settings.compaction.desc')}>
        <Field label={t('settings.autoCompact')} desc={t('settings.autoCompact.desc')}>
          <Switch checked={settings.autoCompact} onChange={v => on.setSetting('autoCompact', v)} label={t('settings.autoCompact')} />
        </Field>
        <Field label={t('settings.compactAt')} desc={t('settings.compactAt.desc')}>
          <NumberField
            value={Math.round(settings.compactAtTokens / K)}
            min={MIN_COMPACT_AT_TOKENS / K}
            step={10}
            unit={t('settings.compactAt.unit')}
            label={t('settings.compactAt')}
            onCommit={v => on.setSetting('compactAtTokens', v * K)}
          />
        </Field>
      </Section>

      <Section title={t('settings.group.run')} desc={t('settings.group.run.desc')}>
        <Field label={t('settings.accountSwitch')} desc={t('settings.accountSwitch.desc')}>
          <Select<AccountSwitchStrategy> options={switchStrategies} value={settings.accountSwitch} onChange={v => on.setSetting('accountSwitch', v)} label={t('settings.accountSwitch')} />
        </Field>
        <Field label={t('settings.agentCpuCap')} desc={t('settings.agentCpuCap.desc')}>
          <NumberField
            value={settings.agentCpuCap}
            min={AGENT_CPU_CAP.min}
            max={AGENT_CPU_CAP.max}
            step={10}
            unit="%"
            label={t('settings.agentCpuCap')}
            onCommit={v => on.setSetting('agentCpuCap', v)}
          />
        </Field>
        <Field label={t('settings.proxy')} desc={t('settings.proxy.desc')}>
          <ProxyField value={settings.proxy} onCommit={v => on.setSetting('proxy', v)} />
        </Field>
      </Section>

      <Section title={t('settings.notifications.title')} desc={t('settings.notifications.desc')}>
        <Field label={t('settings.notifySound')} desc={t('settings.notifySound.desc')}>
          {/* Switching it on plays a preview: the user hears it, and the click lets the page's audio start later on its own.
              The setting is saved first, so the switch flips even if audio output is slow to come up */}
          <Switch checked={settings.notifySound} onChange={v => { on.setSetting('notifySound', v); if (v) playChime(); }} label={t('settings.notifySound')} />
        </Field>
        <Field label={t('settings.notifySystem')} desc={t('settings.notifySystem.desc')}>
          <Switch checked={settings.notifySystem} onChange={v => on.setSetting('notifySystem', v)} label={t('settings.notifySystem')} />
        </Field>
      </Section>
    </>
  );
}

type ProxyChoice = 'auto' | 'off' | 'custom';

// The `proxy` setting: Auto / Off, or Custom with a URL field. Picking Custom shows the field without writing anything; a URL
// is written on Enter or blur once it parses (proxyUrl), otherwise the field goes back to the saved value
function ProxyField({ value, onCommit }: { value: string; onCommit: (v: string) => void }) {
  const saved: ProxyChoice = value === 'auto' || value === 'off' ? value : 'custom';
  const [choice, setChoice] = useState<ProxyChoice>(saved);
  const [text, setText] = useState(saved === 'custom' ? value : '');
  // A change from elsewhere (another window, settings.json) wins; picking Custom alone changes nothing saved, so it stays shown
  useEffect(() => {
    setChoice(saved);
    if (saved === 'custom') setText(value);
  }, [saved, value]);
  const choices: { value: ProxyChoice; label: string }[] = [
    { value: 'auto', label: t('settings.proxy.auto') },
    { value: 'off', label: t('settings.proxy.off') },
    { value: 'custom', label: t('settings.proxy.custom') },
  ];
  const pick = (next: ProxyChoice) => {
    setChoice(next);
    if (next !== 'custom') onCommit(next);
    else if (proxyUrl(text)) onCommit(proxyUrl(text)!);
  };
  const commit = () => {
    const url = proxyUrl(text);
    if (!url) { setText(saved === 'custom' ? value : ''); return; }
    setText(url);
    if (url !== value) onCommit(url);
  };
  const onKey = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === 'Enter') (e.target as HTMLInputElement).blur();
    if (e.key === 'Escape') setText(saved === 'custom' ? value : '');
  };
  return (
    <div className="flex flex-wrap items-center justify-end gap-2">
      {choice === 'custom' && (
        <label className="inline-flex h-ctl w-(--ctl-w) max-w-full items-center rounded-md border border-line bg-hover px-3 text-2 text-fg-1 transition-colors focus-within:bg-active hover:border-line-strong">
          <input
            aria-label={t('settings.proxy.url')}
            placeholder={AUTO_PROXY_URL}
            spellCheck={false}
            value={text}
            onChange={e => setText(e.target.value)}
            onBlur={commit}
            onKeyDown={onKey}
            className="w-full min-w-0 bg-transparent font-mono text-mono outline-none placeholder:text-fg-3"
          />
        </label>
      )}
      <Select<ProxyChoice> options={choices} value={choice} onChange={pick} label={t('settings.proxy')} />
    </div>
  );
}
