import { useEffect, useRef, useState, type KeyboardEvent } from 'react';
import { Pencil, Plus, Server, X } from 'lucide-react';
import { BUILTIN_AGENT_ID, newModel, type Provider, type ProviderAction, type ProvidersView, type ProviderView } from '@shared/providers';
import { Button, IconButton } from '../ui/Button';
import { cn } from '../ui/cn';
import { t } from '../i18n';
import { inputBox, ItemRow, Note, Section, SectionAction, SectionHead } from './controls';
import type { SettingsHandlers } from './SettingsShell';

// The page's last providers view; error when the action that produced it failed
export interface ProvidersState {
  view: ProvidersView;
  error?: string;
}

// The built-in agent's model sources: one row per source, edited in place with an inline form. The key goes to the
// host once and is never shown again; the page only knows whether one is stored
export function ProvidersSection({ state, on }: { state?: ProvidersState; on: SettingsHandlers }) {
  useEffect(() => { on.providers?.(); }, [on]);
  // The id being edited, 'new' for an added source
  const [editing, setEditing] = useState<string>();
  // An action is in flight until the next view lands; a reply without an error closes the form and re-reads the model
  // list, so the visibility switches below pick up the new models
  const [busy, setBusy] = useState(false);
  const pending = useRef(false);
  useEffect(() => {
    setBusy(false);
    if (!pending.current || !state) return;
    pending.current = false;
    if (!state.error) {
      setEditing(undefined);
      on.refreshAgent(BUILTIN_AGENT_ID);
    }
  }, [state, on]);
  const act = (a: ProviderAction) => { pending.current = true; setBusy(true); on.providerAction?.(a); };

  const error = state?.error ?? state?.view.error;
  const providers = state?.view.providers ?? [];
  const form = (p?: ProviderView) => (
    <ProviderForm key={p?.id ?? 'new'} provider={p} busy={busy} onCancel={() => setEditing(undefined)}
      onSave={(provider, key) => act({ kind: 'save', provider, key })} />
  );
  return (
    <div className="flex flex-col gap-2">
      <SectionHead action={<SectionAction icon={<Plus strokeWidth={1.75} />} disabled={!state || editing === 'new'} onClick={() => setEditing('new')}>{t('settings.providers.add')}</SectionAction>}>
        {t('settings.providers')}
      </SectionHead>
      <Section desc={t('settings.providers.desc')}>
        {!state && <Note shimmer>{t('settings.loading')}</Note>}
        {error && <Note><span className="text-danger [overflow-wrap:anywhere]">{error}</span></Note>}
        {state && !providers.length && editing !== 'new' && <Note>{t('settings.providers.none')}</Note>}
        {providers.map(p => editing === p.id ? form(p) : (
          <ItemRow
            key={p.id}
            lead={<Server strokeWidth={1.5} />}
            title={p.name || p.id}
            desc={[p.baseUrl, p.models.map(m => m.id).join(t('common.listSep'))].filter(Boolean).join(t('common.metaSep'))}
            extra={!p.hasKey && <span className="truncate text-2 text-fg-3">{t('settings.providers.noKey')}</span>}
            dim={!p.enabled}
            trailing={<>
              <IconButton title={t('settings.providers.edit', { name: p.name || p.id })} aria-label={t('settings.providers.edit', { name: p.name || p.id })}
                disabled={busy} onClick={() => setEditing(p.id)} className="text-fg-2 opacity-0 group-hover/row:opacity-100 focus-visible:opacity-100">
                <Pencil strokeWidth={1.5} />
              </IconButton>
              <IconButton title={t('common.remove')} aria-label={t('common.removeNamed', { name: p.name || p.id })} disabled={busy}
                onClick={() => act({ kind: 'delete', id: p.id })} className="-mr-1.5 text-fg-2 opacity-0 group-hover/row:opacity-100 focus-visible:opacity-100">
                <X strokeWidth={1.5} />
              </IconButton>
            </>}
          />
        ))}
        {editing === 'new' && form()}
      </Section>
    </div>
  );
}

// One source's fields. Model ids are typed by hand for now, comma or newline separated; ids already in the entry keep
// their settings (context, efforts, …), new ones get the file's defaults
function ProviderForm({ provider, busy, onSave, onCancel }: { provider?: ProviderView; busy: boolean; onSave: (p: Provider, key?: string) => void; onCancel: () => void }) {
  const [name, setName] = useState(provider?.name ?? '');
  const [baseUrl, setBaseUrl] = useState(provider?.baseUrl ?? '');
  const [key, setKey] = useState('');
  const [models, setModels] = useState(provider?.models.map(m => m.id).join(', ') ?? '');
  const ids = [...new Set(models.split(/[,\n]/).map(s => s.trim()).filter(Boolean))];
  const ok = /^https?:\/\/\S+$/.test(baseUrl.trim()) && ids.length > 0 && !busy;
  const submit = () => {
    if (!ok) return;
    const { hasKey: _shown, ...base } = provider ?? { id: '', preset: 'custom', format: 'openai-chat', fullUrl: false, enabled: true, models: [], hasKey: false };
    const kept = new Map(base.models.map(m => [m.id, m]));
    onSave({ ...base, name: name.trim(), baseUrl: baseUrl.trim(), models: ids.map(id => kept.get(id) ?? newModel(id)) }, key.trim() || undefined);
  };
  const keys = (e: KeyboardEvent) => { if (e.key === 'Enter') submit(); if (e.key === 'Escape') onCancel(); };
  const field = cn(inputBox, 'h-ctl w-full');
  return (
    <div className="flex flex-col gap-2 py-(--setting-row-pad)">
      <input autoFocus value={name} onChange={e => setName(e.target.value)} onKeyDown={keys}
        aria-label={t('settings.providers.name')} placeholder={t('settings.providers.name')} className={field} />
      <input value={baseUrl} onChange={e => setBaseUrl(e.target.value)} onKeyDown={keys} spellCheck={false}
        aria-label={t('settings.providers.baseUrl')} placeholder={t('settings.providers.baseUrl')} className={field} />
      <input type="password" autoComplete="off" value={key} onChange={e => setKey(e.target.value)} onKeyDown={keys}
        aria-label={t('settings.providers.key')} placeholder={provider?.hasKey ? t('settings.providers.keyKept') : t('settings.providers.key')} className={field} />
      <input value={models} onChange={e => setModels(e.target.value)} onKeyDown={keys} spellCheck={false}
        aria-label={t('settings.providers.models')} placeholder={t('settings.providers.models')} className={field} />
      <div className="flex flex-wrap items-center gap-2">
        <Button variant="primary" disabled={!ok} onClick={submit}>{t('settings.providers.save')}</Button>
        <Button onClick={onCancel}>{t('settings.providers.cancel')}</Button>
      </div>
    </div>
  );
}
