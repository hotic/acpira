import { useRef, useState, type CSSProperties, type KeyboardEvent } from 'react';
import { ChevronDown, LoaderCircle } from 'lucide-react';
import { effortLabel } from '@shared/composerControls';
import type { ProbeOutcome, ProviderModel, Thinking } from '@shared/providers';
import { Button, IconButton } from '../ui/Button';
import { DropdownMenu } from '../ui/DropdownMenu';
import { OptionContent } from '../ui/Panel';
import { cn } from '../ui/cn';
import { t } from '../i18n';
import { ModelMark } from '../chat/ModelMark';
import { fieldTone, FormField, inputBox, ModelLine, SettingsDialog, Select, Switch, type ModelTags } from './controls';

// Token counts as the page writes them, in whichever base gives the round number: 128000 → 128k, 131072 → 128k,
// 1000000 → 1M, 1048576 → 1M, 393216 → 384k
export function fmtTokens(n: number): string {
  const trim = (v: number) => String(Math.round(v * 100) / 100);
  if (n >= 1_000_000) return `${trim(n % 1_048_576 === 0 ? n / 1_048_576 : n / 1_000_000)}M`;
  if (n >= 1000) return `${trim(n % 1000 !== 0 && n % 1024 === 0 ? n / 1024 : n / 1000)}k`;
  return String(n);
}

// A token count as the editor's field shows it, so saving an untouched field gives back the same number: the short
// form where it parses back exactly (k and M count in 1024s), digits otherwise: 131072 → 128k, 128000 → 128000
export function editTokens(n: number): string {
  const short = fmtTokens(n);
  return parseTokens(short) === n ? short : String(n);
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

// A model's facts in words (the row's tooltip, the add list's second line): window, output limit, levels, image input
export function modelSummary(m: ProviderModel): string {
  return [
    m.context && t('settings.providers.model.ctx', { n: fmtTokens(m.context) }),
    m.output && t('settings.providers.model.out', { n: fmtTokens(m.output) }),
    m.efforts.length > 0 && t('settings.providers.model.levels', { count: m.efforts.length }),
    m.input.includes('image') && t('settings.providers.model.image'),
  ].filter(Boolean).join(t('common.metaSep'));
}

// One model of a source on one compact line: mark, name, badges (context, thinking, images),
// then the enable switch. The id and the summary are the name's tooltip; a click on the row opens the editor in a dialog
export function ModelRow({ model, open, dim, onToggleOpen, onChange, onRemove, onTest, test }: {
  model: ProviderModel;
  open: boolean;
  // The source is switched off: the row reads as inactive whatever its own switch says
  dim?: boolean;
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
      <ModelLine
        lead={<ModelMark family={model.id} />}
        name={name}
        hint={[model.id, modelSummary(model)].filter(Boolean).join(t('common.metaSep'))}
        tags={modelTags(model)}
        dim={dim || !model.enabled}
        onOpen={onToggleOpen} open={open} openLabel={label}
        trailing={<Switch checked={model.enabled} onChange={enabled => onChange({ ...model, enabled })} label={name} />}
      />
      {open && <ModelEditor model={model} title={label} onSave={m => { onChange(m); onToggleOpen(); }} onCancel={onToggleOpen} onRemove={onRemove} onTest={onTest} test={test} />}
    </>
  );
}

// A model row's badges: thinking, image input, a long (1M and up) context window; the exact window is in the tooltip
export function modelTags(m: ProviderModel): ModelTags {
  const tags: ModelTags = {};
  if (m.efforts.length) tags.thinking = t('settings.providers.model.levels', { count: m.efforts.length });
  else if (m.thinking === 'on') tags.thinking = t('settings.providers.model.thinking');
  if (m.fast) tags.fast = t('settings.providers.model.fast');
  if (m.input.includes('image')) tags.image = t('settings.providers.model.image');
  if ((m.context ?? 0) >= 1_000_000) tags.long = t('settings.providers.model.ctx', { n: fmtTokens(m.context!) });
  return tags;
}

// The add list's second line: the id when the name differs, then the summary
export function modelDesc(m: ProviderModel): string | undefined {
  return [m.name && m.name !== m.id ? m.id : '', modelSummary(m)].filter(Boolean).join(t('common.metaSep')) || undefined;
}

// A token-count field: the typed value, with the common sizes in a list behind the chevron at its end. Empty leaves
// the limit unset, which the placeholder says
function TokenField({ value, onChange, quick, label }: { value: string; onChange: (v: string) => void; quick: number[]; label: string }) {
  const bad = value.trim() !== '' && parseTokens(value) === undefined;
  const box = useRef<HTMLDivElement>(null);
  const [tone, setTone] = useState<CSSProperties>();
  const cur = parseTokens(value);
  const pick = t('settings.providers.model.presets', { field: label });
  return (
    <div ref={box} className={cn('flex h-ctl min-w-0 items-center rounded-md border border-line bg-chip pl-3 focus-within:border-line-strong', bad && 'border-danger')}>
      <input value={value} onChange={e => onChange(e.target.value)} aria-label={label} aria-invalid={bad} spellCheck={false}
        placeholder={t('settings.providers.model.default')}
        className="min-w-0 flex-1 border-0 bg-transparent text-2 text-fg-1 tabular-nums outline-none placeholder:text-fg-3" />
      {/* The list opens under the whole field and as wide as it, in the field's material like a settings Select */}
      <DropdownMenu.Root onOpenChange={open => { if (open) setTone(fieldTone(box.current)); }}>
        <DropdownMenu.Trigger render={<IconButton title={pick} aria-label={pick} className="text-fg-3"><ChevronDown strokeWidth={1.75} /></IconButton>} />
        <DropdownMenu.Portal><DropdownMenu.Positioner anchor={box} side="bottom" align="start" width="anchor"><DropdownMenu.Popup style={tone}>
          <DropdownMenu.RadioGroup value={cur === undefined ? '' : String(cur)} className="flex flex-col">
            {quick.map(n => <DropdownMenu.RadioItem key={n} value={String(n)} onClick={() => onChange(editTokens(n))}>
              <OptionContent checked={cur === n} checkSlot>{fmtTokens(n)}</OptionContent>
            </DropdownMenu.RadioItem>)}
          </DropdownMenu.RadioGroup>
        </DropdownMenu.Popup></DropdownMenu.Positioner></DropdownMenu.Portal>
      </DropdownMenu.Root>
    </div>
  );
}

// The levels every family spells this way, in rising order; a model's own other spellings follow them
const LEVELS = ['minimal', 'low', 'medium', 'high', 'xhigh', 'max'];

// The thinking levels as one segmented row of toggles. A lit level is offered in the composer's effort select; none
// lit leaves the provider's default. Thinking itself is switched off by the switch above the row, which hides it
function ThinkingLevels({ levels, onChange }: { levels: string[]; onChange: (levels: string[]) => void }) {
  const all = [...LEVELS, ...levels.filter(l => !LEVELS.includes(l))];
  return (
    <div role="group" aria-label={t('settings.providers.model.efforts')} className="flex min-w-0 flex-wrap gap-0.5 rounded-md bg-hover p-0.5">
      {all.map(level => {
        const lit = levels.includes(level);
        return <button key={level} type="button" aria-pressed={lit}
          onClick={() => onChange(all.filter(l => (l === level ? !lit : levels.includes(l))))}
          className={cn('inline-flex h-ctl-sm min-w-0 flex-auto items-center justify-center whitespace-nowrap rounded-sm px-1.5 text-3 outline-none transition-colors focus-visible:ring-1 focus-visible:ring-inset focus-visible:ring-fg-2',
            lit ? 'bg-active text-fg-1' : 'text-fg-3 hover:bg-hover hover:text-fg-1')}>
          {effortLabel({ id: level, name: level })}
        </button>;
      })}
    </div>
  );
}

// The model's fields in a dialog, laid out like the source form: each label above its control. Only what a source
// commonly gets wrong is here: the name, the two token limits, thinking levels, image input and Fast; the rarer settings
// (family, sampling, tool rounds) stay in providers.json and are kept as they are. Values the endpoint did not report
// (catalogue or defaults) are named in a note at the top, so they can be checked against the provider's docs; saving
// clears that list. Test sits on the last line with its hint, which its result replaces, like the source form's Check;
// the footer holds Remove apart from Cancel and Save
export function ModelEditor({ model, title, onSave, onCancel, onRemove, onTest, test }: {
  model: ProviderModel;
  title: string;
  onSave: (m: ProviderModel) => void;
  onCancel: () => void;
  onRemove: () => void;
  onTest: (m: ProviderModel) => void;
  test?: { pending: boolean; outcome?: ProbeOutcome };
}) {
  const [name, setName] = useState(model.name ?? '');
  const [context, setContext] = useState(model.context ? editTokens(model.context) : '');
  const [output, setOutput] = useState(model.output ? editTokens(model.output) : '');
  const [images, setImages] = useState(model.input.includes('image'));
  const [thinking, setThinking] = useState<Thinking>(model.thinking);
  const [levels, setLevels] = useState(model.efforts);
  const [fast, setFast] = useState<'auto' | 'on' | 'off'>(model.fast == null ? 'auto' : model.fast ? 'on' : 'off');
  const ok = [context, output].every(v => v.trim() === '' || parseTokens(v) !== undefined);
  const draft = (): ProviderModel => ({
    ...model,
    name: name.trim() || undefined,
    context: parseTokens(context),
    output: parseTokens(output),
    input: images ? ['text', 'image'] : ['text'],
    thinking,
    efforts: thinking === 'off' ? [] : levels,
    // Automatic leaves the field out, so the agent's list decides
    fast: fast === 'auto' ? undefined : fast === 'on',
    estimated: [],
  });
  // Enter in a text field saves; a list or a button keeps its own Enter, and Escape is the dialog's close
  const keys = (e: KeyboardEvent) => { if (e.key === 'Enter' && ok && e.target instanceof HTMLInputElement) onSave(draft()); };
  const fastOptions = (['auto', 'on', 'off'] as const).map(v => ({ value: v, label: t(`settings.providers.model.fast.${v}`) }));
  const outcome = test?.outcome;
  const levelsHint = t(levels.length ? 'settings.providers.model.efforts.hint' : 'settings.providers.model.efforts.none');
  return (
    <SettingsDialog title={title} onClose={onCancel}
      footer={<>
        <Button className="text-danger" onClick={onRemove}>{t('settings.providers.model.remove')}</Button>
        <span className="flex-1" />
        <Button onClick={onCancel}>{t('settings.providers.cancel')}</Button>
        <Button variant="primary" disabled={!ok} onClick={() => onSave(draft())}>{t('settings.providers.save')}</Button>
      </>}>
      <div className="scroll-thin flex min-h-0 flex-col gap-3 overflow-y-auto p-pad" onKeyDown={keys}>
        {model.estimated.length > 0 && <p className="m-0 text-3 text-fg-3">
          {t('settings.providers.unconfirmed.hint', { fields: model.estimated.join(', ') })}
        </p>}
        <FormField label={t('settings.providers.model.name')}>
          <input autoFocus value={name} onChange={e => setName(e.target.value)} placeholder={model.id} aria-label={t('settings.providers.model.name')}
            className={cn(inputBox, 'h-ctl w-full')} />
        </FormField>
        {/* The two limits side by side, each under its own label; they stack when the dialog is narrow */}
        <div className="flex flex-wrap gap-3">
          <FormField label={t('settings.providers.model.context')} className="min-w-(--ctl-w) flex-1">
            <TokenField value={context} onChange={setContext} quick={CONTEXT_QUICK} label={t('settings.providers.model.context')} />
          </FormField>
          <FormField label={t('settings.providers.model.output')} className="min-w-(--ctl-w) flex-1">
            <TokenField value={output} onChange={setOutput} quick={OUTPUT_QUICK} label={t('settings.providers.model.output')} />
          </FormField>
        </div>
        {/* Thinking: a switch on its label's line; switched on, the levels row and what it means follow. Switching off
            keeps the levels in the dialog, so switching back restores them; they are dropped only on save */}
        <div className="flex flex-col gap-1">
          <div className="flex min-h-ctl items-center justify-between gap-2">
            <span className="text-3 text-fg-2">{t('settings.providers.model.thinking')}</span>
            <Switch checked={thinking !== 'off'} onChange={on => setThinking(on ? 'auto' : 'off')} label={t('settings.providers.model.thinking')} />
          </div>
          {thinking !== 'off' && <>
            <ThinkingLevels levels={levels} onChange={setLevels} />
            {/* One line: a longer translation truncates and keeps the whole sentence as its tooltip. With no level lit the
                switch alone sends nothing (thinking stays `auto`), so the hint says so rather than implying thinking is on */}
            <span className="truncate text-3 text-fg-3" title={levelsHint}>{levelsHint}</span>
          </>}
        </div>
        {/* A switch reads on one line: the label on the left, the switch at the end */}
        <div className="flex min-h-ctl items-center justify-between gap-2">
          <span className="text-3 text-fg-2">{t('settings.providers.model.images')}</span>
          <Switch checked={images} onChange={setImages} label={t('settings.providers.model.images')} />
        </div>
        {/* Fast is three-way (automatic, offered, not offered), so a select on the label's line; the long hint is the label's tooltip */}
        <div className="flex min-h-ctl items-center justify-between gap-2">
          <span className="text-3 text-fg-2" title={t('settings.providers.model.fast.hint')}>{t('settings.providers.model.fast')}</span>
          <Select options={fastOptions} value={fast} onChange={setFast} label={t('settings.providers.model.fast')} />
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <Button disabled={!ok || test?.pending} title={t('settings.providers.model.test.hint')} onClick={() => onTest(draft())}>
            {test?.pending && <LoaderCircle strokeWidth={1.5} className="size-icon animate-spin live-spin" />}
            {t('settings.providers.model.test')}
          </Button>
          <span className={cn('min-w-0 flex-1 text-3 [overflow-wrap:anywhere]', outcome?.kind === 'failed' ? 'text-danger' : outcome?.kind === 'test' ? 'text-ok' : 'text-fg-3')}>
            {outcome?.kind === 'failed' ? outcome.error
              : outcome?.kind === 'test' ? t('settings.providers.model.testOk', { ms: outcome.ms, text: outcome.text.trim() || '—' })
              : t('settings.providers.model.test.hint')}
          </span>
        </div>
      </div>
    </SettingsDialog>
  );
}
