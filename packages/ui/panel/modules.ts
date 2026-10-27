import { Files, Globe, Puzzle, SquareTerminal, type LucideIcon } from 'lucide-react';
import { t } from '../i18n';
import { usePluginPieces } from '../pluginSlots';
import { usePanelValue } from './store';

// The tool panel's modules, one list for the built-in ones and the plugins' (docs/dev/plugins.md Interface): what the
// new-tab page offers and what "+" can make. Files, Terminal and Browser are compiled in and switched in settings; a
// plugin's `panel.tab` piece joins while its plugin is in effect, after them, in the plugins' order

export type TabPick =
  | { kind: 'browser'; url?: string }
  | { kind: 'terminal' }
  | { kind: 'files' }
  | { kind: 'plugin'; plugin: string; piece: string; title: string };

export interface PanelModule {
  // `files`, `terminal`, `browser`, or `<plugin>/<piece>`
  id: string;
  title: string;
  hint: string;
  Icon: LucideIcon;
  pick: TabPick;
  // Who it comes from: the app, or the plugin's id
  source: 'builtin' | string;
}

export function usePanelModules(): PanelModule[] {
  const files = usePanelValue(s => s.files);
  const terminal = usePanelValue(s => s.terminal);
  const browser = usePanelValue(s => s.browser);
  const pieces = usePluginPieces('panel.tab');
  return [
    ...(files ? [{ id: 'files', title: t('panel.files'), hint: t('panel.filesHint'), Icon: Files, pick: { kind: 'files' } as const, source: 'builtin' }] : []),
    ...(terminal ? [{ id: 'terminal', title: t('panel.terminal'), hint: t('panel.terminalHint'), Icon: SquareTerminal, pick: { kind: 'terminal' } as const, source: 'builtin' }] : []),
    ...(browser ? [{ id: 'browser', title: t('panel.browser'), hint: t('panel.browserHint'), Icon: Globe, pick: { kind: 'browser' } as const, source: 'builtin' }] : []),
    ...pieces.map(p => {
      const title = p.title || p.plugin;
      return { id: `${p.plugin}/${p.id}`, title, hint: title, Icon: Puzzle, pick: { kind: 'plugin', plugin: p.plugin, piece: p.id, title } as const, source: p.plugin };
    }),
  ];
}
