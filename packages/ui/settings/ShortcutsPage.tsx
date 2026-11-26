import { useEffect, useMemo, useRef, useState } from 'react';
import { RotateCcw } from 'lucide-react';
import type { MsgKey } from '@acpira/shared/i18n/core';
import {
  SHORTCUT_DEFS,
  effectiveShortcut,
  formatShortcut,
  recordShortcutEvent,
  type KeybindingMap,
  type ShortcutDef,
  type ShortcutGroup,
  type ShortcutId,
} from '@acpira/shared/keybindings';
import type { SettingsView } from '@acpira/shared/settings';
import { macOS, useDesktop } from '../desktop';
import { t, useLocale } from '../i18n';
import { IconButton } from '../ui/Button';
import { imeKey } from '../ui/ime';
import { ItemRow, Note, SearchBox, Section } from '../chrome/controls';
import type { SettingsHandlers } from './SettingsShell';

export const SHORTCUT_LABEL: Record<ShortcutId, MsgKey> = {
  openSettings: 'session.settings',
  openShortcuts: 'settings.shortcuts.title',
  openFolder: 'desktop.empty.open',
  quickOpen: 'panel.searchFiles',
  toggleSidebar: 'desktop.sidebar',
  back: 'settings.shortcuts.back',
  forward: 'settings.shortcuts.forward',
  closeTab: 'panel.closeTab',
  focusComposer: 'settings.shortcuts.focusComposer',
  cycleMode: 'settings.shortcuts.cycleMode',
  openModelPicker: 'settings.shortcuts.openModelPicker',
  invertSteerSend: 'settings.shortcuts.invertSteerSend',
  findInChat: 'settings.shortcuts.findInChat',
  findNext: 'terminal.findNext',
  findPrev: 'terminal.findPrevious',
  newSession: 'session.new',
  searchSessions: 'session.search',
  nextSession: 'settings.shortcuts.nextSession',
  prevSession: 'settings.shortcuts.prevSession',
  nextAttention: 'settings.shortcuts.nextAttention',
  toggleToolPanel: 'panel.label',
  toggleTerminal: 'panel.terminal',
  openChanges: 'panel.allChanges',
  toggleBrowser: 'desktop.menu.browser',
};

export const SHORTCUT_GROUP_LABEL: Record<ShortcutGroup, MsgKey> = {
  general: 'settings.nav.general',
  chat: 'settings.shortcuts.group.chat',
  sessions: 'settings.shortcuts.group.sessions',
  panel: 'panel.label',
};

const GROUPS: readonly ShortcutGroup[] = ['general', 'chat', 'sessions', 'panel'];

export function ShortcutsPage({ settings, on }: { settings: SettingsView; on: SettingsHandlers }) {
  useLocale();
  const desktop = useDesktop();
  const [query, setQuery] = useState('');
  const [recording, setRecording] = useState<ShortcutId>();
  const rowBox = useRef<HTMLDivElement>(null);
  const map: KeybindingMap = settings.keybindings ?? {};

  const defs = useMemo(
    () => SHORTCUT_DEFS.filter(d => desktop || !d.desktopOnly),
    [desktop],
  );

  const applyBinding = (id: ShortcutId, nextAccel: string | undefined) => {
    const def = SHORTCUT_DEFS.find(d => d.id === id);
    if (!def) return;
    const next: KeybindingMap = { ...map };
    if (nextAccel === undefined || nextAccel === def.defaultKey) {
      delete next[id];
    } else {
      next[id] = nextAccel;
    }
    // Unbind any visible sibling shortcut that previously owned the same non-empty chord
    const finalChord = effectiveShortcut(id, next);
    if (finalChord) {
      for (const other of defs) {
        if (other.id !== id && effectiveShortcut(other.id, next) === finalChord) {
          next[other.id] = '';
        }
      }
    }
    on.setSetting('keybindings', next);
  };

  useEffect(() => {
    if (!recording) return;
    const onKey = (e: KeyboardEvent) => {
      if (imeKey(e)) return;
      e.preventDefault();
      e.stopPropagation();
      const recorded = recordShortcutEvent(e, macOS);
      if (recorded === undefined) return;
      if (recorded === 'cancel') {
        setRecording(undefined);
        return;
      }
      applyBinding(recording, recorded === 'clear' ? '' : recorded);
      setRecording(undefined);
    };
    const onPointer = (e: PointerEvent) => {
      const target = e.target instanceof Node ? e.target : null;
      if (target && rowBox.current?.querySelector(`[data-shortcut-row="${recording}"]`)?.contains(target)) return;
      setRecording(undefined);
    };
    window.addEventListener('keydown', onKey, true);
    window.addEventListener('pointerdown', onPointer, true);
    return () => {
      window.removeEventListener('keydown', onKey, true);
      window.removeEventListener('pointerdown', onPointer, true);
    };
  });

  const q = query.trim().toLowerCase();
  const filtered = useMemo(() => {
    if (!q) return defs;
    return defs.filter(d => {
      const label = t(SHORTCUT_LABEL[d.id]).toLowerCase();
      const chord = formatShortcut(effectiveShortcut(d.id, map), macOS).toLowerCase();
      return label.includes(q) || chord.includes(q);
    });
  }, [defs, map, q]);

  const renderRow = (d: ShortcutDef) => {
    const active = effectiveShortcut(d.id, map);
    const formatted = formatShortcut(active, macOS);
    const customized = map[d.id] !== undefined;
    const isRecording = recording === d.id;
    const label = t(SHORTCUT_LABEL[d.id]);
    return (
      <div key={d.id} data-shortcut-row={d.id}>
        <ItemRow
          title={<span data-search-anchor>{label}</span>}
          editLabel={label}
          onEdit={() => setRecording(cur => (cur === d.id ? undefined : d.id))}
          trailing={isRecording ? (
            <span className="inline-flex h-ctl-sm items-center rounded-md border border-line-strong bg-active px-2 text-3 text-fg-1">
              {t('settings.shortcuts.recording')}
            </span>
          ) : (
            <span className="flex items-center gap-1.5">
              {customized && (
                <IconButton
                  size="sm"
                  title={t('settings.shortcuts.resetOne')}
                  aria-label={t('settings.shortcuts.resetOne')}
                  onClick={() => applyBinding(d.id, undefined)}
                  className="text-fg-3 opacity-0 group-hover/row:opacity-100 focus-visible:opacity-100"
                >
                  <RotateCcw strokeWidth={1.5} />
                </IconButton>
              )}
              {formatted ? (
                <span
                  className="inline-flex h-ctl-sm items-center rounded-md border border-line bg-chip px-2 text-3 text-fg-2 tabular-nums"
                  style={macOS ? { fontFamily: '-apple-system, BlinkMacSystemFont, var(--font)', letterSpacing: '0.04em' } : undefined}
                >
                  {formatted}
                </span>
              ) : (
                <span className="inline-flex h-ctl-sm items-center text-3 text-fg-3">{t('settings.shortcuts.unset')}</span>
              )}
            </span>
          )}
        />
      </div>
    );
  };

  return (
    <div ref={rowBox} className="flex flex-col gap-(--section-gap)">
      <SearchBox value={query} onChange={setQuery} label={t('settings.shortcuts.search')} />
      {filtered.length === 0 ? (
        <Section>
          <Note>{t('settings.search.none')}</Note>
        </Section>
      ) : (
        GROUPS.map(group => {
          const items = filtered.filter(d => d.group === group);
          if (!items.length) return null;
          return (
            <Section key={group} title={t(SHORTCUT_GROUP_LABEL[group])}>
              {items.map(renderRow)}
            </Section>
          );
        })
      )}
    </div>
  );
}
