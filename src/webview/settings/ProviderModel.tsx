import { useState, type KeyboardEvent, type ReactNode } from 'react';
import { ChevronDown, ChevronUp, LoaderCircle, X } from 'lucide-react';
import type { ProbeOutcome, ProviderModel, Thinking } from '@shared/providers';
import { Button, Chip, IconButton } from '../ui/Button';
import { cn } from '../ui/cn';
import { t } from '../i18n';
import { ModelMark } from '../chat/ModelMark';
import { inputBox, ItemRow, Select, Switch } from './controls';

// Token counts as the page writes them, in whichever base gives the round number: 128000 → 128k, 131072 → 128k,
// 1000000 → 1M, 1048576 → 1M, 393216 → 384k
export function fmtTokens(n: number): string {
  const trim = (v: number) => String(Math.round(v * 100) / 100);
  if (n >= 1_000_000) return `${trim(n % 1_048_576 === 0 ? n / 1_048_576 : n / 1_000_000)}M`;
  if (n >= 1000) return `${trim(n % 1000 !== 0 && n % 1024 === 0 ? n / 1024 : n / 1000)}k`;
  return String(n);
}

// "128k" / "1M" / "131072" → tokens (k and M count in 1024s, like the quick values); undefined for anything else
export function parseTokens(text: string): number | undefined {
  const m = /^\s*(\d+(?:\.\d+)?)\s*([kKmM])?\s*$/.exec(text);
  if (!m) return undefined;
  const unit = m[2]?.toLowerCase() === 'm' ? 1_048_576 : m[2] ? 1024 : 1;
  const n = Math.round(Number(m[1]) * unit);
  return n > 0 ? n : undefined;
}

const CONTEXT_QUICK = [131_072, 262_144, 524_288, 1_048_576];
const OUTPUT_QUICK = [4096, 16_384, 32_768, 131_072];

// The second line of a model row: its window, output limit, levels and image input
export function modelSummary(m: ProviderModel): string {
  return [
    m.context && t('settings.providers.model.ctx', { n: fmtTokens(m.context) }),
    m.output && t('settings.providers.model.out', { n: fmtTokens(m.output) }),
    m.efforts.length > 0 && t('settings.providers.model.levels', { count: m.efforts.length }),
    m.input.includes('image') && t('settings.providers.model.image'),
  ].filter(Boolean).join(t('common.metaSep'));
}

// The marker for values the endpoint did not report (catalogue or defaults): check them against the provider's docs
export function Unconfirmed({ fields }: { fields: string[] }) {
  if (!fields.length) return null;
  return <span className="shrink-0 text-3 text-warn" title={t('settings.providers.unconfirmed.hint', { fields: fields.join(', ') })}>{t('settings.providers.unconfirmed')}</span>;
}

// One model of a source: mark, name, summary, the unconfirmed marker, then edit and the enable switch. The editor opens
// in place under the row
export function ModelRow({ model, families, open, onToggleOpen, onChange, onRemove, onTest, test }: {
  model: ProviderModel;
  families: string[];
  open: boolean;
  onToggleOpen: () => void;
  onChange: (m: ProviderModel) => void;
  onRemove: () => void;
  onTest: (m: ProviderModel) => void;
  test?: { pending: boolean; outcome?: ProbeOutcome };
}) {
  const name = model.name || model.id;
  const label = t('settings.providers.model.edit', { name });
  return (
    <>
      <ItemRow
        lead={<ModelMark family={model.id} />}
        title={<span className="flex min-w-0 items-baseline gap-2"><span className="truncate">{name}</span><Unconfirmed fields={model.estimated} /></span>}
        desc={[model.name && model.name !== model.id ? model.id : '', modelSummary(model)].filter(Boolean).join(t('common.metaSep')) || undefined}
        dim={!model.enabled}
        trailing={<>
          <IconButton title={label} aria-label={label} aria-expanded={open} onClick={onToggleOpen} className="text-fg-2">
            {open ? <ChevronUp strokeWidth={1.5} /> : <ChevronDown strokeWidth={1.5} />}
          </IconButton>
          <Switch checked={model.enabled} onChange={enabled => onChange({ ...model, enabled })} label={name} />
        </>}
      />
      {open && <ModelEditor model={model} families={families} onSave={m => { onChange(m); onToggleOpen(); }} onCancel={onToggleOpen} onRemove={onRemove} onTest={onTest} test={test} />}
    </>
  );
}

// A labelled line of the editor: label on the left, the control on the right (wraps under it when narrow)
function Line({ label, children, hint }: { label: string; children: ReactNode; hint?: string }) {
  return (
    <div className="flex flex-wrap items-center gap-2">
      <span className="min-w-0 flex-[1_1_var(--setting-header-copy)] text-2 text-fg-1" title={hint}>{label}</span>
      <div className="flex min-w-0 flex-wrap items-center justify-end gap-2">{children}</div>
    </div>
  );
}

// A token-count field with quick values; empty leaves the limit unset
function TokenField({ value, onChange, quick, label }: { value: string; onChange: (v: string) => void; quick: number[]; label: string }) {
  const bad = value.trim() !== '' && parseTokens(value) === undefined;
  return <>
    {quick.map(n => (
      <Chip key={n} caret={false} aria-pressed={parseTokens(value) === n} onClick={() => onChange(fmtTokens(n))}
        className={cn('h-ctl-sm', parseTokens(value) === n && 'bg-active text-fg-1')}>{fmtTokens(n)}</Chip>
    ))}
    <input value={value} onChange={e => onChange(e.target.value)} aria-label={label} aria-invalid={bad} spellCheck={false}
      className={cn(inputBox, 'h-ctl w-(--num-w) text-right tabular-nums', bad && 'border-danger')} />
  </>;
}

const num = (s: string) => (s.trim() === '' ? undefined : Number(s));

// The model's fields. Saving drops the unconfirmed marker of every field the editor shows: the user has looked at them
function ModelEditor({ model, families, onSave, onCancel, onRemove, onTest, test }: {
  model: ProviderModel;
  families: string[];
  onSave: (m: ProviderModel) => void;
  onCancel: () => void;
  onRemove: () => void;
  onTest: (m: ProviderModel) => void;
  test?: { pending: boolean; outcome?: ProbeOutcome };
}) {
  const [name, setName] = useState(model.name ?? '');
  const [context, setContext] = useState(model.context ? fmtTokens(model.context) : '');
  const [output, setOutput] = useState(model.output ? fmtTokens(model.output) : '');
  const [images, setImages] = useState(model.input.includes('image'));
  const [thinking, setThinking] = useState<Thinking>(model.thinking);
  const [efforts, setEfforts] = useState(model.efforts.join(', '));
  const [family, setFamily] = useState(model.family ?? '');
  const [temperature, setTemperature] = useState(model.sampling.temperature?.toString() ?? '');
  const [topP, setTopP] = useState(model.sampling.topP?.toString() ?? '');
  const [topK, setTopK] = useState(model.sampling.topK?.toString() ?? '');
  const [maxSteps, setMaxSteps] = useState(model.maxSteps?.toString() ?? '');
  const numbers = [temperature, topP, topK, maxSteps];
  const ok = [context, output].every(v => v.trim() === '' || parseTokens(v) !== undefined)
    && numbers.every(v => v.trim() === '' || Number.isFinite(Number(v)));
  const draft = (): ProviderModel => ({
    ...model,
    name: name.trim() || undefined,
    context: parseTokens(context),
    output: parseTokens(output),
    input: images ? ['text', 'image'] : ['text'],
    thinking,
    efforts: [...new Set(efforts.split(/[,\s]+/).map(s => s.trim()).filter(Boolean))],
    family: family || null,
    sampling: { temperature: num(temperature), topP: num(topP), topK: num(topK) },
    maxSteps: num(maxSteps) ?? null,
    estimated: [],
  });
  const keys = (e: KeyboardEvent) => { if (e.key === 'Enter' && ok) onSave(draft()); if (e.key === 'Escape') onCancel(); };
  const field = cn(inputBox, 'h-ctl');
  const thinkingOptions = (['auto', 'on', 'off'] as const).map(v => ({ value: v, label: t(`settings.providers.model.thinking.${v}`) }));
  const familyOptions = [{ value: '', label: t('settings.providers.model.family.auto') }, ...families.map(f => ({ value: f, label: f }))];
  const outcome = test?.outcome;
  return (
    <div className="flex flex-col gap-2 border-t border-line py-(--setting-row-pad)" onKeyDown={keys}>
      <Line label={t('settings.providers.model.name')}>
        <input value={name} onChange={e => setName(e.target.value)} placeholder={model.id} aria-label={t('settings.providers.model.name')} className={field} />
      </Line>
      <Line label={t('settings.providers.model.context')}>
        <TokenField value={context} onChange={setContext} quick={CONTEXT_QUICK} label={t('settings.providers.model.context')} />
      </Line>
      <Line label={t('settings.providers.model.output')}>
        <TokenField value={output} onChange={setOutput} quick={OUTPUT_QUICK} label={t('settings.providers.model.output')} />
      </Line>
      <Line label={t('settings.providers.model.images')}>
        <Switch checked={images} onChange={setImages} label={t('settings.providers.model.images')} />
      </Line>
      <Line label={t('settings.providers.model.thinking')}>
        <Select options={thinkingOptions} value={thinking} onChange={setThinking} label={t('settings.providers.model.thinking')} />
      </Line>
      {thinking !== 'off' && <Line label={t('settings.providers.model.efforts')} hint={t('settings.providers.model.efforts.hint')}>
        <input value={efforts} onChange={e => setEfforts(e.target.value)} placeholder="low, high, max" spellCheck={false}
          aria-label={t('settings.providers.model.efforts')} className={field} />
      </Line>}
      <Line label={t('settings.providers.model.family')}>
        <Select options={familyOptions} value={family} onChange={setFamily} label={t('settings.providers.model.family')} />
      </Line>
      <Line label={t('settings.providers.model.sampling')} hint={t('settings.providers.model.sampling.hint')}>
        {/* The request parameter names as placeholders: they fit the narrow number fields */}
        {([['Temperature', 'temp', temperature, setTemperature], ['Top P', 'top_p', topP, setTopP], ['Top K', 'top_k', topK, setTopK]] as const).map(([label, hint, value, set]) => (
          <input key={label} value={value} onChange={e => set(e.target.value)} placeholder={hint} aria-label={label} title={label} spellCheck={false}
            className={cn(field, 'w-(--num-w) text-right tabular-nums')} />
        ))}
      </Line>
      <Line label={t('settings.providers.model.maxSteps')}>
        <input value={maxSteps} onChange={e => setMaxSteps(e.target.value)} placeholder={t('settings.providers.model.maxSteps.none')}
          aria-label={t('settings.providers.model.maxSteps')} spellCheck={false} className={cn(field, 'text-right tabular-nums')} />
      </Line>
      {outcome && <p className={cn('m-0 text-2 [overflow-wrap:anywhere]', outcome.kind === 'failed' ? 'text-danger' : 'text-fg-2')}>
        {outcome.kind === 'failed' ? outcome.error : outcome.kind === 'test' ? t('settings.providers.model.testOk', { ms: outcome.ms, text: outcome.text.trim() || '—' }) : null}
      </p>}
      <div className="flex flex-wrap items-center gap-2">
        <Button variant="primary" disabled={!ok} onClick={() => onSave(draft())}>{t('settings.providers.save')}</Button>
        <Button onClick={onCancel}>{t('settings.providers.cancel')}</Button>
        <Button disabled={!ok || test?.pending} title={t('settings.providers.model.test.hint')} onClick={() => onTest(draft())}>
          {test?.pending && <LoaderCircle strokeWidth={1.5} className="size-icon animate-spin live-spin" />}
          {t('settings.providers.model.test')}
        </Button>
        <span className="min-w-0 flex-1 text-3 text-fg-3">{t('settings.providers.model.test.hint')}</span>
        <IconButton title={t('settings.providers.model.remove')} aria-label={t('settings.providers.model.remove')} onClick={onRemove} className="text-fg-2">
          <X strokeWidth={1.5} />
        </IconButton>
      </div>
    </div>
  );
}
