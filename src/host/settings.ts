import type { AgentId } from '@shared/transcript';
import type { AgentInventory, AgentRuntimeInfo } from '@shared/inventory';
import { AXES, type AxisKey } from '@shared/appearance';
import { sanitizeSetting, type SettingKey, type SettingsView } from '@shared/settings';
import { resolveLocale, type Locale } from '@shared/i18n';
import type { AgentRegistry } from './acp/AgentRegistry';
import { agentExt } from './agentExt';
import { cloneJson } from './clone';
import { scanInventory } from './inventory';

export interface SettingsDeps {
  // Reads one acpira.* setting; object values come back as a read-only Proxy, so view() JSON-round-trips them
  read: (key: SettingKey) => unknown;
  // Writes acpira.<key> at user scope
  write: (key: SettingKey, value: unknown) => PromiseLike<void>;
  // Writes acpira.appearance.<axis> at user scope (the one axis the page exposes: motion)
  writeAppearance: (axis: AxisKey, value: string) => PromiseLike<void>;
  // The host's display language (vscode.env.language), for resolving `auto`
  hostLanguage: () => string;
  registry: () => AgentRegistry;
  // Runtime info of an agent's live session (version, MCP capabilities), when one is running
  runtimeInfo: (agent: AgentId) => AgentRuntimeInfo | undefined;
  home: () => string;
  cwd: () => string;
}

export type SettingsEvent = { type: 'settings'; settings: SettingsView; locale: Locale };

// The settings page's host-side counterpart: builds the SettingsView from acpira.*, writes edits back, and scans agent inventories on demand.
// No vscode import, so it runs under vitest with injected deps
export class SettingsCenter {
  private listeners = new Set<(ev: SettingsEvent) => void>();

  constructor(private deps: SettingsDeps) {}

  // getConfiguration().get() hands back a read-only Proxy that postMessage can't clone; a JSON round-trip fixes the object values.
  // Values are then checked against the shape the readers expect, so a hand-edited settings.json cannot break the page
  private read<K extends SettingKey>(key: K): SettingsView[K] {
    const v = this.deps.read(key);
    return sanitizeSetting(key, typeof v === 'object' && v !== null ? cloneJson(v) : v);
  }

  view(): SettingsView {
    return {
      language: this.read('language'),
      locale: this.locale(),
      defaultAgent: this.read('defaultAgent'),
      agentOrder: this.read('agentOrder'),
      disabledAgents: this.read('disabledAgents'),
      sessionScope: this.read('sessionScope'),
      sessionListPosition: this.read('sessionListPosition'),
      autoCompact: this.read('autoCompact'),
      compactAtTokens: this.read('compactAtTokens'),
      hiddenOptions: this.read('hiddenOptions'),
      theme: this.read('theme'),
      uiFontSize: this.read('uiFontSize'),
      codeFontSize: this.read('codeFontSize'),
      diffMarkers: this.read('diffMarkers'),
      fontSmoothing: this.read('fontSmoothing'),
    };
  }

  locale(): Locale {
    return resolveLocale(this.read('language'), this.deps.hostLanguage());
  }

  // Write, then push: VS Code's own onDidChangeConfiguration also fires (and covers hand edits of settings.json), a double push is harmless.
  // The page only ever sends values it rendered, but the message can come from any webview script, so the value is checked like a file edit
  async set(key: SettingKey, value: unknown): Promise<void> {
    await this.deps.write(key, sanitizeSetting(key, value));
    this.emit();
  }

  // An appearance axis from the page: only values the axis declares are written; the configuration listener pushes the new Appearance to every bridge
  async setAppearance(axis: AxisKey, value: unknown): Promise<void> {
    const ax = AXES.find(a => a.key === axis);
    if (!ax || typeof value !== 'string' || !ax.options.some(o => o.value === value)) return;
    await this.deps.writeAppearance(axis, value);
  }

  // Scan one agent's extension points fresh; every request is a rescan (the page's refresh button sends the same message)
  async inventory(agent: AgentId): Promise<AgentInventory> {
    const binary = await this.deps.registry().resolveBinary(agent);
    const env = { home: this.deps.home(), cwd: this.deps.cwd() };
    return scanInventory({ agent, ext: agentExt(agent), binary, runtime: this.deps.runtimeInfo(agent) }, env);
  }

  subscribe(fn: (ev: SettingsEvent) => void): () => void {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  }

  // Re-read everything and push (a setting changed in the page, in the Settings UI, or in settings.json)
  emit() {
    const ev: SettingsEvent = { type: 'settings', settings: this.view(), locale: this.locale() };
    for (const fn of this.listeners) fn(ev);
  }
}
