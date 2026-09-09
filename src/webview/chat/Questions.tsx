import { useCallback, useEffect, useId, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from 'react';
import { Check, MessageCircleQuestion } from 'lucide-react';
import type { Question, QuestionAnswer, QuestionAnswers, QuestionBlock } from '@shared/transcript';
import { useAppearance } from '../appearance';
import { t } from '../i18n';
import { Card } from '../ui/Card';
import { Button } from '../ui/Button';
import { Disclosure } from '../ui/Disclosure';
import { Row, RowLabel } from '../ui/Row';
import { cn } from '../ui/cn';
import { useScrollFade } from '../ui/useScrollFade';
import { questionOptionIndex } from './questionKeys';

export type OnAnswer = (blockId: string, answers: QuestionAnswers, skip?: boolean) => void;

interface Pick {
  options: string[];
  other: boolean;
  text: string;
}

const EMPTY: Pick = { options: [], other: false, text: '' };
const KEYS = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ';

// Picks remain keyed by question ID while only the current question is mounted.
function answerOf(q: Question, p: Pick): QuestionAnswer | undefined {
  const text = p.text.trim();
  if (q.kind === 'text') return text || undefined;
  if (q.kind === 'single') return p.other ? text || undefined : p.options[0];
  const list = [...p.options, ...(p.other && text ? [text] : [])];
  return list.length ? list : undefined;
}

// A waiting question stays visible. One question per page gives the counter and
// navigation a single meaning; selecting an answer never changes the page.
export function Questions({ block, onAnswer }: { block: QuestionBlock; onAnswer: OnAnswer }) {
  const root = useRef<HTMLDivElement>(null);
  const titleId = useId();
  const [picks, setPicks] = useState<Record<string, Pick>>({});
  const [active, setActive] = useState(0);
  const [focusField, setFocusField] = useState(false);
  const fade = useScrollFade<HTMLDivElement>();
  const qs = block.questions;
  const total = qs.length;
  const page = Math.min(active, Math.max(0, total - 1));
  const current = qs[page];
  const last = page === total - 1;
  const answers = useMemo(() => {
    const out: QuestionAnswers = {};
    for (const q of qs) { const a = answerOf(q, picks[q.id] ?? EMPTY); if (a !== undefined) out[q.id] = a; }
    return out;
  }, [qs, picks]);
  const canSubmit = Object.keys(answers).length > 0 && qs.every(q => !q.required || answers[q.id] !== undefined);
  const canAdvance = last ? canSubmit : !!current && (!current.required || answers[current.id] !== undefined);
  const skip = useCallback(() => onAnswer(block.id, answers, true), [onAnswer, block.id, answers]);
  const update = (id: string, fn: (p: Pick) => Pick) => setPicks(ps => ({ ...ps, [id]: fn(ps[id] ?? EMPTY) }));

  // Focus the card on entry and page changes without moving the conversation.
  useEffect(() => { root.current?.focus({ preventScroll: true }); }, [page]);
  useEffect(() => {
    if (!focusField) return;
    root.current?.querySelector<HTMLInputElement>('input')?.focus({ preventScroll: true });
    setFocusField(false);
  }, [focusField]);
  const go = (i: number) => setActive(Math.max(0, Math.min(total - 1, i)));
  const advance = () => {
    if (!canAdvance) return;
    if (last) onAnswer(block.id, answers);
    else go(page + 1);
  };
  const choose = (q: Question, optionId: string) => {
    if (q.kind === 'multiple') update(q.id, p => ({ ...p, options: p.options.includes(optionId) ? p.options.filter(o => o !== optionId) : [...p.options, optionId] }));
    else update(q.id, p => ({ ...p, other: false, options: [optionId] }));
  };
  const chooseOther = (q: Question) => {
    const selected = picks[q.id]?.other;
    update(q.id, p => ({ ...p, other: q.kind === 'multiple' ? !p.other : true, ...(q.kind === 'single' ? { options: [] } : {}) }));
    if (q.kind !== 'multiple' || !selected) setFocusField(true);
  };
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    // Composition and modified keys belong to the editor or the operating system.
    if (e.nativeEvent.isComposing || e.metaKey || e.ctrlKey || e.altKey) return;
    const target = e.target as HTMLElement;
    const typing = target.tagName === 'INPUT' || target.tagName === 'TEXTAREA' || target.isContentEditable;
    if (e.key === 'Escape') {
      e.preventDefault();
      if (typing) root.current?.focus({ preventScroll: true });
      else skip();
      return;
    }
    if (e.key === 'Enter' && !e.shiftKey) {
      // Focused buttons retain native activation, including options and Back.
      if (target.closest('button')) return;
      e.preventDefault();
      advance();
      return;
    }
    if (typing) return;
    if (e.key === '[') { e.preventDefault(); go(page - 1); return; }
    if (e.key === ']') { e.preventDefault(); if (!last) advance(); return; }
    if (e.key === 'ArrowUp' || e.key === 'ArrowDown') {
      const options = Array.from(root.current?.querySelectorAll<HTMLButtonElement>('[data-question-option]') ?? []);
      if (!options.length) return;
      const index = options.indexOf(target.closest('button') as HTMLButtonElement);
      const next = index < 0 ? (e.key === 'ArrowDown' ? 0 : options.length - 1)
        : (index + (e.key === 'ArrowDown' ? 1 : -1) + options.length) % options.length;
      e.preventDefault();
      options[next]?.focus({ preventScroll: true });
      options[next]?.scrollIntoView({ block: 'nearest' });
      return;
    }
    if (!current || current.kind === 'text') return;
    const index = questionOptionIndex(e.key);
    if (index === undefined) return;
    if (index < current.options.length) { e.preventDefault(); choose(current, current.options[index]!.id); }
    else if (index === current.options.length && current.other) { e.preventDefault(); chooseOther(current); }
  };
  if (!current) return null;
  return (
    <div className="px-page">
      <Card ref={root} tabIndex={-1} role="form" aria-label={t('question.title')} onKeyDown={onKeyDown} className="flex min-w-0 flex-col overflow-hidden">
        <div key={current.id} ref={fade} className="scroll-fade scroll-thin flex max-h-question-body flex-col gap-(--question-section-gap) overflow-y-auto px-pad pt-(--question-top-pad) pb-(--question-section-gap) [--scroll-fade-size:var(--question-section-gap)] [&>*]:shrink-0">
          {/* The question itself labels the card; a separate title row adds no context. */}
          <Row dense className="items-start lead-top" lead={<MessageCircleQuestion className="size-icon" strokeWidth={1.5} />}
            trailing={<span aria-live="polite">{t('question.of', { n: page + 1, total })}</span>}>
            <h3 id={titleId} className="m-0 min-w-0 text-2 font-medium text-fg-strong [overflow-wrap:anywhere]">{current.text}</h3>
          </Row>
          {block.message && page === 0 && <p className="m-0 text-2 text-fg-2 [overflow-wrap:anywhere]">{block.message}</p>}
          <QuestionItem question={current} titleId={titleId} pick={picks[current.id] ?? EMPTY}
            onChoose={id => choose(current, id)} onOther={() => chooseOther(current)}
            onOtherFocus={() => update(current.id, p => ({ ...p, other: true, ...(current.kind === 'single' ? { options: [] } : {}) }))}
            onText={text => update(current.id, p => ({ ...p, text, ...(current.kind !== 'text' ? { other: true } : {}), ...(current.kind === 'single' ? { options: [] } : {}) }))} />
        </div>
        <div className="flex shrink-0 items-center justify-between gap-gap border-t border-line px-pad py-(--question-section-gap)">
          <Button variant="secondary" onClick={skip} className={cn(focusRing, 'h-ctl-sm border-line/50')}>{t('question.skip')}</Button>
          <div className="flex items-center gap-gap">
            {page > 0 && <Button variant="secondary" onClick={() => go(page - 1)} className={cn(focusRing, 'h-ctl-sm border-line/50')}>{t('question.prev')}</Button>}
            <Button variant="primary" disabled={!canAdvance} onClick={advance} className={cn(focusRing, 'h-ctl-sm disabled:cursor-not-allowed disabled:border-line/50 disabled:bg-transparent disabled:text-fg-3')}>
              {t(last ? 'question.submit' : 'question.next')}
            </Button>
          </div>
        </div>
      </Card>
    </div>
  );
}

// Hallmark · component: question card · theme: existing Acpira tokens.
// One question per page, compact type, shared row alignment and persistent actions.
// Every focusable control in the card shows the same inset ring; the "other" row lights up while its input has focus.
const focusRing = 'focus-visible:shadow-[inset_0_0_0_var(--question-focus-width)_var(--fg-2)]';
const optionRowClass = 'w-full rounded-md p-(--question-option-pad) active:bg-active';
const optionKeyClass = 'flex size-lead items-center justify-center rounded-sm border border-line-strong text-3 text-fg-2';

interface ItemProps {
  question: Question;
  titleId: string;
  pick: Pick;
  onChoose: (optionId: string) => void;
  onOther: () => void;
  onOtherFocus: () => void;
  onText: (text: string) => void;
}

function QuestionItem({ question: q, titleId, pick, onChoose, onOther, onOtherFocus, onText }: ItemProps) {
  const groupRole = q.kind === 'multiple' ? 'group' : q.kind === 'single' ? 'radiogroup' : undefined;
  const placeholder = q.kind === 'text' ? t(q.numeric ? 'question.numberPlaceholder' : 'question.textPlaceholder') : t('question.otherPlaceholder');
  return (
    <section className="flex min-w-0 flex-col gap-(--question-section-gap)">
      {q.kind === 'multiple' && <p className="m-0 text-3 text-fg-2">{t('question.multiple')}</p>}
      <div role={groupRole} aria-labelledby={titleId} className="flex min-w-0 flex-col">
        {q.options.map((o, i) => (
          <OptionRow key={o.id} keyLabel={KEYS[i] ?? String(i + 1)} selected={pick.options.includes(o.id)}
            role={q.kind === 'multiple' ? 'checkbox' : 'radio'} onClick={() => onChoose(o.id)}>
            <span className="text-fg-1 [overflow-wrap:anywhere]">{o.label}</span>
            {o.description && <span className="text-3 text-fg-2 [overflow-wrap:anywhere]">{o.description}</span>}
          </OptionRow>
        ))}
        {q.kind !== 'text' && q.other && (
          <Row className={cn(optionRowClass, 'items-start hover:not-focus-within:bg-hover focus-within:bg-active focus-within:shadow-[inset_0_0_0_var(--question-focus-width)_var(--fg-2)]', pick.other && 'bg-active')}
            lead={
              <button type="button" data-question-option="" className={cn(optionKeyClass, focusRing, pick.other && 'text-fg-1')}
                role={q.kind === 'multiple' ? 'checkbox' : 'radio'} aria-checked={pick.other}
                aria-label={t('question.other')} aria-keyshortcuts={KEYS[q.options.length]}
                onClick={onOther}>
                {pick.other ? <Check className="size-icon" strokeWidth={1.75} /> : KEYS[q.options.length]}
              </button>
            }>
            <input type="text" value={pick.text} placeholder={t('question.other')}
              aria-label={t('question.otherPlaceholder')} onFocus={onOtherFocus}
              onChange={e => onText(e.target.value)}
              className="h-lead min-w-0 flex-1 bg-transparent text-2 text-fg-1 placeholder:text-fg-2" />
          </Row>
        )}
      </div>
      {q.kind === 'text' && (
        <input type="text" value={pick.text} inputMode={q.numeric ? 'decimal' : undefined}
          placeholder={placeholder} aria-labelledby={titleId} onChange={e => onText(e.target.value)}
          className={cn('h-ctl w-full min-w-0 rounded-md border border-line-strong bg-transparent px-gap text-2 text-fg-1 placeholder:text-fg-3', focusRing)} />
      )}
    </section>
  );
}

// A shared Row aligns the key with the label's first line; descriptions stay in
// the same text column. Selected rows use a check rather than an inverted tile.
function OptionRow({ keyLabel, selected, role, onClick, children }: {
  keyLabel: string; selected: boolean; role: 'radio' | 'checkbox'; onClick: () => void; children: ReactNode;
}) {
  return (
    <Row as="button" data-question-option="" role={role} aria-checked={selected} aria-keyshortcuts={keyLabel} onClick={onClick}
      className={cn(optionRowClass, 'items-start lead-top hover:bg-hover focus-visible:bg-hover', focusRing, selected && 'bg-active')}
      lead={<span className={cn(optionKeyClass, selected && 'text-fg-1')}>{selected ? <Check className="size-icon" strokeWidth={1.75} /> : keyLabel}</span>}>
      <span className="flex min-w-0 flex-1 flex-col gap-(--question-description-gap)">{children}</span>
    </Row>
  );
}

// The record a resolved card leaves in the message: one row, expandable to the questions and what was picked
export function QuestionRecord({ block }: { block: QuestionBlock }) {
  const { toolLine } = useAppearance();
  const answers = block.answers ?? {};
  const n = Object.keys(answers).length;
  const label = block.outcome === 'answered' ? (n === 1 ? t('question.answeredOne') : t('question.answered', { n }))
    : block.outcome === 'skipped' ? t('question.skipped') : t('question.cancelled');
  return (
    <Disclosure
      lead={toolLine === 'text' ? undefined : <MessageCircleQuestion className="size-icon" strokeWidth={1.5} />}
      defaultOpen={block.outcome === 'answered'}
      body={
        <dl className="m-0 flex flex-col gap-1.5 text-2">
          {block.questions.map(q => {
            const a = answers[q.id];
            const names = a === undefined ? [] : (Array.isArray(a) ? a : [a]).map(v => q.options.find(o => o.id === v)?.label ?? v);
            return (
              <div key={q.id} className="flex min-w-0 flex-col">
                <dt className="text-fg-3 [overflow-wrap:anywhere]">{q.text}</dt>
                <dd className={cn('m-0 [overflow-wrap:anywhere]', names.length ? 'text-fg-1' : 'text-fg-3')}>{names.length ? names.join(', ') : t('question.noAnswer')}</dd>
              </div>
            );
          })}
        </dl>
      }
    >
      <RowLabel>{label}</RowLabel>
    </Disclosure>
  );
}
