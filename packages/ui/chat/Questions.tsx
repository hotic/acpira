import { useCallback, useContext, useEffect, useId, useMemo, useRef, useState, type ClipboardEvent, type DragEvent, type KeyboardEvent, type ReactNode } from 'react';
import { Check, ImagePlus, MessageCircleQuestion } from 'lucide-react';
import type { ImageDraft, Question, QuestionAnswer, QuestionAnswers, QuestionBlock } from '@acpira/shared/transcript';
import { t } from '../i18n';
import { Card } from '../ui/Card';
import { Button } from '../ui/Button';
import { Disclosure } from '../ui/Disclosure';
import { Row, RowLabel } from '../ui/Row';
import { cn } from '../ui/cn';
import { useScrollFade } from '../ui/useScrollFade';
import { imeKey } from '../ui/ime';
import { questionOptionIndex } from './questionKeys';
import { DraftChips, TurnAttachments } from './Attachments';
import { collectImages } from './drafts';
import { BlobUrlContext } from './fileLinks';

// `images`: image drafts by question id, sent only by a card that takes them (QuestionBlock.images)
export type OnAnswer = (blockId: string, answers: QuestionAnswers, skip?: boolean, images?: Record<string, ImageDraft[]>) => void;

interface Pick {
  options: string[];
  other: boolean;
  text: string;
  // Images sent with this answer, inside the "other" field (or the text field); alone they answer the question
  images: ImageDraft[];
}

const EMPTY: Pick = { options: [], other: false, text: '', images: [] };
// Images one answer may carry (the engine keeps to the same number)
const MAX_ANSWER_IMAGES = 4;
// A question takes images where it has a field of its own to type in
const takesImages = (block: QuestionBlock, q: Question) => !!block.images && (q.kind === 'text' || !!q.other);
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
// `subtitle` is a faint provenance line above the question (a subagent's card pinned at the root).
export function Questions({ block, onAnswer, subtitle }: { block: QuestionBlock; onAnswer: OnAnswer; subtitle?: string }) {
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
  const images = useMemo(() => {
    const out: Record<string, ImageDraft[]> = {};
    for (const q of qs) { const list = picks[q.id]?.images; if (list?.length && takesImages(block, q)) out[q.id] = list; }
    return out;
  }, [qs, picks, block]);
  const answered = (q: Question) => answers[q.id] !== undefined || !!images[q.id];
  const canSubmit = qs.some(answered) && qs.every(q => !q.required || answered(q));
  const canAdvance = last ? canSubmit : !!current && (!current.required || answered(current));
  const skip = useCallback(() => onAnswer(block.id, answers, true), [onAnswer, block.id, answers]);
  const submit = () => onAnswer(block.id, answers, false, Object.keys(images).length ? images : undefined);
  const update = (id: string, fn: (p: Pick) => Pick) => setPicks(ps => ({ ...ps, [id]: fn(ps[id] ?? EMPTY) }));
  const accepts = !!current && takesImages(block, current);
  // Pasted, dropped or picked images join the current question's answer
  const attach = (files: Iterable<File>) => {
    if (!current || !accepts) return;
    const id = current.id;
    void collectImages(files).then(list => {
      // Images live in the "other" field, so attaching picks it like typing does
      if (list.length) update(id, p => ({
        ...p, images: [...p.images, ...list].slice(0, MAX_ANSWER_IMAGES),
        ...(current.kind !== 'text' ? { other: true } : {}), ...(current.kind === 'single' ? { options: [] } : {}),
      }));
    });
  };
  const hasFiles = (dt: DataTransfer | null) => !!dt && [...dt.types].includes('Files');
  const onPaste = (e: ClipboardEvent<HTMLDivElement>) => {
    if (!accepts || !e.clipboardData.files.length) return;
    e.preventDefault();
    attach(Array.from(e.clipboardData.files));
  };
  const onDragOver = (e: DragEvent<HTMLDivElement>) => { if (accepts && hasFiles(e.dataTransfer)) { e.preventDefault(); e.dataTransfer.dropEffect = 'copy'; } };
  const onDrop = (e: DragEvent<HTMLDivElement>) => {
    if (!accepts || !hasFiles(e.dataTransfer)) return;
    e.preventDefault();
    attach(Array.from(e.dataTransfer.files));
  };

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
    if (last) submit();
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
    if (imeKey(e.nativeEvent) || e.metaKey || e.ctrlKey || e.altKey) return;
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
    <div className="px-dock">
      <Card ref={root} tabIndex={-1} role="form" aria-label={t('question.title')} onKeyDown={onKeyDown}
        onPaste={onPaste} onDragOver={onDragOver} onDrop={onDrop} className="flex min-w-0 flex-col overflow-hidden">
        <div className="flex min-h-0 max-h-question-body flex-col">
          {/* Keep the question and counter outside the answer scrollport and its edge fade. */}
          <div className="flex shrink-0 flex-col gap-(--question-section-gap) px-pad pt-(--question-top-pad) pb-(--question-section-gap)">
            {subtitle && <div className="text-3 text-fg-3">{subtitle}</div>}
            {/* No row min height: its slack sat under the text and pushed the answers off centre between it and the footer rule */}
            <Row dense className="min-h-0 items-start lead-top" lead={<MessageCircleQuestion className="size-icon" strokeWidth={1.5} />}
              trailing={<span aria-live="polite">{t('question.of', { n: page + 1, total })}</span>}>
              <h3 id={titleId} className="m-0 min-w-0 text-2 font-medium text-fg-strong [overflow-wrap:anywhere]">{current.text}</h3>
            </Row>
          </div>
          <div key={current.id} ref={fade} className="scroll-fade scroll-thin flex min-h-0 flex-col gap-(--question-section-gap) overflow-y-auto overscroll-y-contain px-pad pb-(--question-section-gap) [--scroll-fade-size:var(--question-section-gap)] [&>*]:shrink-0">
            {block.message && page === 0 && <p className="m-0 text-2 text-fg-2 [overflow-wrap:anywhere]">{block.message}</p>}
            <QuestionItem question={current} titleId={titleId} pick={picks[current.id] ?? EMPTY}
              onAttach={accepts ? attach : undefined}
              onRemoveImages={indices => update(current.id, p => ({ ...p, images: p.images.filter((_, i) => !indices.includes(i)) }))}
              onChoose={id => choose(current, id)} onOther={() => chooseOther(current)}
              onOtherFocus={() => update(current.id, p => ({ ...p, other: true, ...(current.kind === 'single' ? { options: [] } : {}) }))}
              onText={text => update(current.id, p => ({ ...p, text, ...(current.kind !== 'text' ? { other: true } : {}), ...(current.kind === 'single' ? { options: [] } : {}) }))} />
          </div>
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
  // Set when the answer may carry images: the field gets an attach button and the images show below it
  onAttach?: (files: Iterable<File>) => void;
  onRemoveImages: (indices: number[]) => void;
  onChoose: (optionId: string) => void;
  onOther: () => void;
  onOtherFocus: () => void;
  onText: (text: string) => void;
}

function QuestionItem({ question: q, titleId, pick, onAttach, onRemoveImages, onChoose, onOther, onOtherFocus, onText }: ItemProps) {
  const groupRole = q.kind === 'multiple' ? 'group' : q.kind === 'single' ? 'radiogroup' : undefined;
  const placeholder = q.kind === 'text' ? t(q.numeric ? 'question.numberPlaceholder' : 'question.textPlaceholder') : t('question.otherPlaceholder');
  const attachButton = onAttach && <AttachImages full={pick.images.length >= MAX_ANSWER_IMAGES} onFiles={onAttach} />;
  // The answer's images, shown inside its field above the text
  const thumbs = pick.images.length > 0 && <DraftChips drafts={pick.images} onRemove={onRemoveImages} className="p-0" />;
  return (
    <section className="flex min-w-0 flex-col gap-(--question-section-gap)">
      {q.kind === 'multiple' && <p className="m-0 text-3 text-fg-2">{t('question.multiple')}</p>}
      {/* A text question has no choices: an empty group would still take a section gap above the field */}
      {q.kind !== 'text' && <div role={groupRole} aria-labelledby={titleId} className="flex min-w-0 flex-col">
        {q.options.map((o, i) => (
          <OptionRow key={o.id} keyLabel={KEYS[i] ?? String(i + 1)} selected={pick.options.includes(o.id)}
            role={q.kind === 'multiple' ? 'checkbox' : 'radio'} onClick={() => onChoose(o.id)}>
            <span className="text-fg-1 [overflow-wrap:anywhere]">{o.label}</span>
            {o.description && <span className="text-3 text-fg-2 [overflow-wrap:anywhere]">{o.description}</span>}
          </OptionRow>
        ))}
        {q.other && (
          // A one-line field keeps Row's centring (the row's min height exceeds the padded field). Images sit inside the
          // field above the text, as in the composer; the key and the button then stay on the text line
          <Row className={cn(optionRowClass, 'hover:not-focus-within:bg-hover focus-within:bg-active focus-within:shadow-[inset_0_0_0_var(--question-focus-width)_var(--fg-2)]', pick.other && 'bg-active', thumbs && 'items-end')}
            trailing={attachButton}
            lead={
              <button type="button" data-question-option="" className={cn(optionKeyClass, focusRing, pick.other && 'text-fg-1')}
                role={q.kind === 'multiple' ? 'checkbox' : 'radio'} aria-checked={pick.other}
                aria-label={t('question.other')} aria-keyshortcuts={KEYS[q.options.length]}
                onClick={onOther}>
                {pick.other ? <Check className="size-icon" strokeWidth={1.75} /> : KEYS[q.options.length]}
              </button>
            }>
            <div className="flex min-w-0 flex-1 flex-col gap-gap">
              {thumbs}
              <input type="text" value={pick.text} placeholder={t('question.other')}
                aria-label={t('question.otherPlaceholder')} onFocus={onOtherFocus}
                onChange={e => onText(e.target.value)}
                className="h-lead w-full min-w-0 bg-transparent text-2 text-fg-1 placeholder:text-fg-2" />
            </div>
          </Row>
        )}
      </div>}
      {q.kind === 'text' && !attachButton && (
        <input type="text" value={pick.text} inputMode={q.numeric ? 'decimal' : undefined}
          placeholder={placeholder} aria-labelledby={titleId} onChange={e => onText(e.target.value)}
          className={cn('h-ctl w-full min-w-0 rounded-md border border-line-strong bg-transparent px-gap text-2 text-fg-1 placeholder:text-fg-3', focusRing)} />
      )}
      {/* With an attach button the border and ring move to a box around the images, the field and the button */}
      {q.kind === 'text' && attachButton && (
        <div className="flex w-full min-w-0 flex-col rounded-md border border-line-strong focus-within:shadow-[inset_0_0_0_var(--question-focus-width)_var(--fg-2)]">
          {thumbs && <div className="px-gap pt-gap">{thumbs}</div>}
          <div className="flex h-ctl min-w-0 items-center gap-gap pl-gap pr-(--question-option-pad)">
            <input type="text" value={pick.text} inputMode={q.numeric ? 'decimal' : undefined}
              placeholder={placeholder} aria-labelledby={titleId} onChange={e => onText(e.target.value)}
              className="h-full min-w-0 flex-1 bg-transparent text-2 text-fg-1 outline-none placeholder:text-fg-3" />
            {attachButton}
          </div>
        </div>
      )}
    </section>
  );
}

// The image button at the end of an answer field: opens the file picker; paste and drop work on the whole card
function AttachImages({ full, onFiles }: { full: boolean; onFiles: (files: Iterable<File>) => void }) {
  const input = useRef<HTMLInputElement>(null);
  return (
    <>
      <button type="button" disabled={full} title={t('question.attachImage')} aria-label={t('question.attachImage')}
        onClick={() => input.current?.click()}
        className={cn('flex size-lead shrink-0 items-center justify-center rounded-sm text-fg-3 hover:bg-hover hover:text-fg-1 disabled:pointer-events-none disabled:opacity-50', focusRing)}>
        <ImagePlus className="size-icon" strokeWidth={1.5} />
      </button>
      <input ref={input} type="file" accept="image/*" multiple hidden
        onChange={e => { const files = Array.from(e.target.files ?? []); e.target.value = ''; onFiles(files); }} />
    </>
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
  const blobUrl = useContext(BlobUrlContext);
  const answers = block.answers ?? {};
  const pictures = block.answerImages ?? {};
  const n = block.questions.filter(q => answers[q.id] !== undefined || pictures[q.id]?.length).length;
  const label = block.outcome === 'answered' ? (n === 1 ? t('question.answeredOne') : t('question.answered', { n }))
    : block.outcome === 'skipped' ? t('question.skipped') : t('question.cancelled');
  return (
    <Disclosure
      lead={<MessageCircleQuestion className="size-icon" strokeWidth={1.5} />}
      defaultOpen={block.outcome === 'answered'}
      body={
        <dl className="m-0 flex flex-col gap-1.5 text-2">
          {block.questions.map(q => {
            const a = answers[q.id];
            const names = a === undefined ? [] : (Array.isArray(a) ? a : [a]).map(v => q.options.find(o => o.id === v)?.label ?? v);
            const shots = pictures[q.id] ?? [];
            return (
              <div key={q.id} className="flex min-w-0 flex-col">
                <dt className="text-fg-3 [overflow-wrap:anywhere]">{q.text}</dt>
                {(names.length > 0 || !shots.length) && <dd className={cn('m-0 [overflow-wrap:anywhere]', names.length ? 'text-fg-1' : 'text-fg-3')}>{names.length ? names.join(', ') : t('question.noAnswer')}</dd>}
                {shots.length > 0 && (
                  <dd className="m-0 pt-gap-half">
                    <TurnAttachments blobUrl={blobUrl} attachments={shots.map(i => ({ kind: 'image' as const, blob: i.blob, mimeType: i.mimeType }))} />
                  </dd>
                )}
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
