import { useLayoutEffect, useRef, useState } from 'react';
import { Search } from 'lucide-react';
import type { FileHit } from '@acpira/shared/protocol';
import { t, useLocale } from '../i18n';
import { Dialog } from '../ui/Dialog';
import { cn } from '../ui/cn';
import { optionClass } from '../ui/DropdownMenu';
import { imeKey } from '../ui/ime';
import { useMentionHits } from '../chat/Mention';
import { FileGlyph, dirOf, nameOf } from './files';

// Desktop ⌘P / Ctrl+P file opener: queries the engine's workspace file index and opens the chosen path in the tool panel
export function QuickOpen({
  searchFiles,
  onOpenFile,
  onClose,
}: {
  searchFiles: (query: string) => Promise<FileHit[]>;
  onOpenFile: (path: string) => void;
  onClose: () => void;
}) {
  useLocale();
  const [query, setQuery] = useState('');
  const { hits, ready, active, setActive, move } = useMentionHits(query, searchFiles);
  const input = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const pointer = useRef(false);

  useLayoutEffect(() => {
    input.current?.focus({ preventScroll: true });
  }, []);

  useLayoutEffect(() => {
    if (pointer.current) return;
    const box = list.current;
    const row = box?.querySelector<HTMLElement>('[data-highlighted]');
    if (!box || !row) return;
    const br = box.getBoundingClientRect();
    const rr = row.getBoundingClientRect();
    if (rr.top < br.top) box.scrollTop += rr.top - br.top;
    else if (rr.bottom > br.bottom) box.scrollTop += rr.bottom - br.bottom;
  }, [active, hits]);

  const pick = (hit?: FileHit) => {
    if (!hit) return;
    onClose();
    onOpenFile(hit.path);
  };

  return (
    <Dialog.Root open disablePointerDismissal onOpenChange={open => { if (!open) onClose(); }}>
      <Dialog.Portal>
        <Dialog.Popup
          onClick={e => { if (e.target === e.currentTarget) onClose(); }}
          className="acp-settings absolute inset-0 z-30 flex items-center justify-center bg-scrim p-pad"
        >
          <div
            data-quick-open
            data-palette="menu"
            className="popup-surface flex w-(--dialog-w) max-w-full flex-col overflow-hidden rounded-lg border p-1 shadow-pop"
          >
            <Dialog.Title className="sr-only">{t('panel.searchFiles')}</Dialog.Title>
            <label className="mb-1 flex h-ctl shrink-0 items-center gap-2 border-b border-line px-2 text-fg-3">
              <Search className="size-icon shrink-0" strokeWidth={1.5} aria-hidden />
              <input
                ref={input}
                value={query}
                spellCheck={false}
                placeholder={t('panel.searchFiles')}
                aria-label={t('panel.searchFiles')}
                onChange={e => setQuery(e.target.value)}
                onKeyDown={e => {
                  if (imeKey(e.nativeEvent)) return;
                  pointer.current = false;
                  if (e.key === 'ArrowDown') { e.preventDefault(); move(1); }
                  else if (e.key === 'ArrowUp') { e.preventDefault(); move(-1); }
                  else if (e.key === 'Enter') { e.preventDefault(); if (ready) pick(hits[active]); }
                  else if (e.key === 'Escape') { e.preventDefault(); e.stopPropagation(); onClose(); }
                }}
                className="min-w-0 flex-1 bg-transparent text-2 text-fg-1 outline-none placeholder:text-fg-3"
              />
            </label>
            {/* Fixed --pop height so async engine replies never resize the dialog */}
            <div
              ref={list}
              role="listbox"
              onPointerMove={() => { pointer.current = true; }}
              className="scroll-thin scroll-stable flex h-pop flex-col overflow-y-auto"
            >
              {hits.map((h, i) => {
                const name = nameOf(h.path);
                const dir = dirOf(h.path);
                const highlighted = i === active;
                return (
                  <button
                    key={h.uri}
                    type="button"
                    role="option"
                    aria-selected={highlighted}
                    data-highlighted={highlighted ? '' : undefined}
                    onPointerMove={() => { if (pointer.current && active !== i) setActive(i); }}
                    onClick={() => pick(h)}
                    className={cn(optionClass, 'shrink-0')}
                  >
                    <FileGlyph name={name} className="text-fg-3" />
                    <span className="shrink-0 truncate font-mono text-mono">{name}</span>
                    {dir && <span className="min-w-0 flex-1 truncate text-left text-3 text-fg-3">{dir}</span>}
                  </button>
                );
              })}
              {ready && hits.length === 0 && (
                <div className="flex min-h-row items-center px-2 text-3 text-fg-3">{t('mention.noFiles')}</div>
              )}
            </div>
          </div>
        </Dialog.Popup>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
