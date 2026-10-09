import { useEffect, useState, type KeyboardEvent } from 'react';
import type { AgentInfo } from '@shared/transcript';
import { ACCOUNT_SWITCH_STRATEGIES, AGENT_CPU_CAP, AUTO_PROXY_URL, MIN_COMPACT_AT_TOKENS, proxyUrl, SESSION_SCOPES, type AccountSwitchStrategy, type SessionScope, type SettingsView } from '@shared/settings';
import { LANGUAGES, type Language } from '@shared/i18n';
import { launchable, pickDefaultAgent } from '@shared/agentOrder';
import { AgentMark } from '../chat/AgentMark';
import { t } from '../i18n';
import { Field, NumberField, Section, Select, Switch } from './controls';
import type { SettingsHandlers } from './SettingsShell';

// The threshold field works in thousands of tokens
const K = 1000;

// General: the shell's own knobs. Rendering preferences live on the Appearance page; the LAB appearance axes (bar motion) stay design decisions.
// The settings shell owns the page heading and content measure.
export function General({ settings, agents, on }: { settings: SettingsView; agents: AgentInfo[]; on: SettingsHandlers }) {
  const languages = LANGUAGES.map(l => ({ value: l, label: t(`settings.language.${l}` as const) }));
  // Switched-off agents are not offered; a default that was switched off reads as the agent new sessions actually fall back to
  const agentOptions = launchable(agents).map(a => ({ value: a.id, label: a.name, icon: <AgentMark id={a.id} name={a.name} />, disabled: a.available === false }));
  const scopes = SESSION_SCOPES.map(s => ({ value: s, label: t(`settings.sessionScope.${s}` as const) }));
  const switchStrategies = ACCOUNT_SWITCH_STRATEGIES.map(s => ({ value: s, label: t(`settings.accountSwitch.${s}` as const) }));
  return (
    <>
      <Section>
        <Field label={t('settings.language')} desc={t('settings.language.desc')}>
          <Select<Language> options={languages} value={settings.language} onChange={v => on.setSetting('language', v)} label={t('settings.language')} />
        </Field>
        <Field label={t('settings.defaultAgent')} desc={t('settings.defaultAgent.desc')}>
          <Select options={agentOptions} value={pickDefaultAgent(agents, settings.defaultAgent)} onChange={v => on.setSetting('defaultAgent', v)} label={t('settings.defaultAgent')} />
        </Field>
        <Field label={t('settings.sessionScope')} desc={t('settings.sessionScope.desc')}>
          <Select<SessionScope> options={scopes} value={settings.sessionScope} onChange={v => on.setSetting('sessionScope', v)} label={t('settings.sessionScope')} />
        </Field>
        <Field label={t('settings.accountSwitch')} desc={t('settings.accountSwitch.desc')}>
          <Select<AccountSwitchStrategy> options={switchStrategies} value={settings.accountSwitch} onChange={v => on.setSetting('accountSwitch', v)} label={t('settings.accountSwitch')} />
        </Field>
        <Field label={t('settings.shareEditorSelection')} desc={t('settings.shareEditorSelection.desc')}>
          <Switch checked={settings.shareEditorSelection} onChange={v => on.setSetting('shareEditorSelection', v)} label={t('settings.shareEditorSelection')} />
        </Field>
        <Field label={t('settings.steerQueued')} desc={t('settings.steerQueued.desc')}>
          <Switch checked={settings.steerQueued} onChange={v => on.setSetting('steerQueued', v)} label={t('settings.steerQueued')} />
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

      <Section title={t('settings.compaction.title')}>
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
    <div className="flex items-center gap-2">
      {choice === 'custom' && (
        <label className="inline-flex h-ctl items-center rounded-md border border-line bg-hover px-3 text-2 text-fg-1 transition-colors focus-within:bg-active">
          <input
            aria-label={t('settings.proxy.url')}
            placeholder={AUTO_PROXY_URL}
            spellCheck={false}
            value={text}
            onChange={e => setText(e.target.value)}
            onBlur={commit}
            onKeyDown={onKey}
            className="w-(--ctl-w) min-w-0 bg-transparent font-mono text-mono outline-none placeholder:text-fg-3"
          />
        </label>
      )}
      <Select<ProxyChoice> options={choices} value={choice} onChange={pick} label={t('settings.proxy')} />
    </div>
  );
}
