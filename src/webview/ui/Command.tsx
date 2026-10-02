import { useLayoutEffect, useRef, useState, type ComponentProps } from 'react';
import { Combobox as Base } from '@base-ui/react/combobox';
import { Search } from 'lucide-react';
import { cn, cnState } from './cn';
import { useMergedRefs } from './mergeRefs';
import { optionClass } from './DropdownMenu';
import { t } from '../i18n';

// An inline combobox inside an existing popup, never a second portal or dialog.
function Root<Value>({ inputValue: controlled, onInputValueChange, ...props }: Base.Root.Props<Value>) {
  const [query, setQuery] = useState('');
  return <Base.Root inline open autoHighlight {...props} inputValue={controlled ?? query}
    onInputValueChange={(next, details) => {
      onInputValueChange?.(next, details);
      if (!details.isCanceled) setQuery(next);
    }} />;
}
function Input({ className, visible = true, ref, ...props }: ComponentProps<typeof Base.Input> & { visible?: boolean }) {
  const input = useRef<HTMLInputElement>(null);
  const inputRef = useMergedRefs(input, ref);
  useLayoutEffect(() => { if (visible) input.current?.focus({ preventScroll: true }); }, [visible]);
  if (!visible) return null;
  return <label className="mb-1 flex h-ctl shrink-0 items-center gap-2 border-b border-line px-2 text-fg-3">
    <Search className="size-icon shrink-0" strokeWidth={1.5} />
    <Base.Input ref={inputRef} placeholder={t('common.search')} spellCheck={false} aria-label={t('common.search')} {...props}
      className={cnState(cn('min-w-0 flex-1 bg-transparent text-2 text-fg-1 outline-none placeholder:text-fg-3'), className)} />
  </label>;
}
function List({ className, searchable = false, ref, ...props }: ComponentProps<typeof Base.List> & { searchable?: boolean }) {
  const list = useRef<HTMLDivElement>(null);
  const listRef = useMergedRefs(list, ref);
  useLayoutEffect(() => {
    const el = list.current;
    if (!el) return;
    const selected = el.querySelector<HTMLElement>('[data-selected]');
    if (searchable && selected) {
      const lr = el.getBoundingClientRect(), sr = selected.getBoundingClientRect();
      el.scrollTop += sr.top - lr.top - (lr.height - sr.height) / 2;
    }
    if (!searchable) el.focus({ preventScroll: true });
    // Only the list scrolls when virtual focus changes; the conversation stays put.
    const follow = () => {
      const active = el.querySelector<HTMLElement>('[data-highlighted]');
      if (!active) return;
      const lr = el.getBoundingClientRect(), ar = active.getBoundingClientRect();
      if (ar.top < lr.top) el.scrollTop += ar.top - lr.top;
      else if (ar.bottom > lr.bottom) el.scrollTop += ar.bottom - lr.bottom;
    };
    const observer = new MutationObserver(follow);
    observer.observe(el, { subtree: true, attributes: true, attributeFilter: ['data-highlighted'] });
    return () => observer.disconnect();
  }, [searchable]);
  return <Base.List ref={listRef} tabIndex={searchable ? -1 : 0} {...props}
    className={cnState(cn('scroll-thin flex max-h-pop flex-col overflow-y-auto outline-none', searchable && 'scroll-stable'), className)} />;
}
function Item({ className, ...props }: ComponentProps<typeof Base.Item>) {
  return <Base.Item render={<button type="button" />} nativeButton {...props}
    className={cnState(cn(optionClass, 'shrink-0 data-[selected]:option-selected data-[selected]:hover:option-selected data-[selected]:data-[highlighted]:option-selected data-[selected]:focus-visible:option-selected'), className)} />;
}
function Empty({ className, children = t('common.noMatch'), ...props }: ComponentProps<typeof Base.Empty>) {
  return <Base.Empty {...props} className={cnState(cn('flex min-h-row items-center px-2 text-3 text-fg-3 empty:hidden'), className)}>{children}</Base.Empty>;
}
export const Command = { Root, Input, List, Item, Empty, Group: Base.Group, GroupLabel: Base.GroupLabel };
