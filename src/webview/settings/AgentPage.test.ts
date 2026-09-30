import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { AgentInfo } from '@shared/transcript';
import type { AgentInventory } from '@shared/inventory';
import { DEFAULT_SETTINGS } from '@shared/settings';
import { setLocale, t } from '../i18n';
import { AgentPage } from './AgentPage';
import type { SettingsHandlers } from './SettingsShell';

const handlers: SettingsHandlers = {
  setSetting: vi.fn(), setAppearance: vi.fn(), openPath: vi.fn(), refreshInventory: vi.fn(),
  refreshAgent: vi.fn(), selectAccount: vi.fn(), addAccount: vi.fn(), removeAccount: vi.fn(),
  installAgent: vi.fn(), openExternal: vi.fn(),
};

function renderAgent(agent: AgentInfo, extra: Partial<AgentInventory> = {}) {
  return renderToStaticMarkup(createElement(AgentPage, {
    agent, accounts: [], settings: DEFAULT_SETTINGS, env: { home: '/preview', cwd: '/preview/project' }, on: handlers,
    inventory: {
      agent: agent.id, binary: agent.available ? `/preview/bin/${agent.id}` : null,
      steer: false, scannedAt: '2026-09-30T00:00:00Z', config: [], mcp: [], skills: [], rules: [], ...extra,
    },
  }));
}

afterEach(() => setLocale('en'));

describe('agent installation settings', () => {
  it.each([
    ['codex', ['codex-acp']], ['claude', ['claude-agent-acp']], ['dsh', ['dsh']], ['pi', ['pi-acp', 'pi']], ['codex', ['node']],
  ] as const)('names the missing executables for %s without appending them to the description', (id, missing) => {
    setLocale('zh-CN');
    const markup = renderAgent({ id, name: id, available: false, missing: [...missing], install: { command: 'npm install -g example-agent' } });
    expect(markup).toContain(`未找到 ${missing.join(', ')}`);
    expect(markup).toContain(`>${t('settings.install.desc')}</p>`);
    expect(markup).not.toContain('。:');
    if (id === 'codex') expect(markup).not.toContain('>未找到 codex<');
  });

  it.each(['grok', 'kimi', 'devin'])('keeps quick installation available for an installed %s', id => {
    const markup = renderAgent({ id, name: id, available: true, install: { command: `install-${id}`, docs: `https://example.com/${id}` } });
    expect(markup).toContain(`Install ${id}`);
    expect(markup).toContain('Install in terminal');
    expect(markup).toContain(`install-${id}`);
    expect(markup).toContain('Install guide');
    expect(handlers.installAgent).not.toHaveBeenCalled();
  });

  it('lists the searched directories for a missing CLI with the PATH hint in the tooltip', () => {
    const markup = renderAgent({ id: 'codex', name: 'Codex', available: false, missing: ['codex-acp'], searched: ['/preview/.local/bin', '/usr/local/bin'] });
    expect(markup).toContain('Searched');
    expect(markup).toContain('~/.local/bin  /usr/local/bin');
    expect(markup).toContain('login shell profile');
  });

  it('shows no searched row once the CLI is found', () => {
    const markup = renderAgent({ id: 'codex', name: 'Codex', available: true, searched: ['/usr/local/bin'] });
    expect(markup).not.toContain('Searched');
  });

  it('flags a bundled engine whose native platform package is missing', () => {
    const pkg = '@anthropic-ai/claude-agent-sdk-linux-x64';
    const markup = renderAgent({ id: 'claude', name: 'Claude', available: true }, {
      adapter: { engine: { name: 'Claude Agent SDK', version: '0.3.284', nativeMissing: pkg } },
    });
    expect(markup).toContain(`native binary missing (${pkg})`);
    expect(markup).toContain('--include=optional');
    expect(markup).not.toContain('bundled');
  });

  it('does not invent an executable name when the host has no missing-command details', () => {
    const markup = renderAgent({ id: 'custom-agent', name: 'Custom agent', available: false });
    expect(markup).toContain('Executable not detected');
    expect(markup).not.toContain('custom-agent not found');
    expect(markup).not.toContain('Install in terminal');
  });

  it('shows a docs-only installation guide without offering an absent command', () => {
    const markup = renderAgent({ id: 'custom-agent', name: 'Custom agent', available: true, install: { docs: 'https://example.com/install' } });
    expect(markup).toContain('Install guide');
    expect(markup).not.toContain('Install in terminal');
    expect(markup).not.toContain(t('settings.install.desc'));
  });
});
