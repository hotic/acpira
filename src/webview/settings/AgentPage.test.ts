import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { AgentInfo } from '@shared/transcript';
import type { AgentInventory } from '@shared/inventory';
import type { AgentInstallProgress } from '@shared/protocol';
import { DEFAULT_SETTINGS } from '@shared/settings';
import { setLocale, t } from '../i18n';
import { AgentPage } from './AgentPage';
import type { SettingsHandlers } from './SettingsShell';

const handlers: SettingsHandlers = {
  setSetting: vi.fn(), setAppearance: vi.fn(), openPath: vi.fn(), refreshInventory: vi.fn(),
  refreshAgent: vi.fn(), selectAccount: vi.fn(), addAccount: vi.fn(), removeAccount: vi.fn(),
  installAgent: vi.fn(), openExternal: vi.fn(),
};

function renderAgent(agent: AgentInfo, extra: Partial<AgentInventory> = {}, install?: AgentInstallProgress, on: SettingsHandlers = handlers) {
  return renderToStaticMarkup(createElement(AgentPage, {
    agent, accounts: [], settings: DEFAULT_SETTINGS, env: { home: '/preview', cwd: '/preview/project' }, on, install,
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
    expect(markup).toContain('>Install</span></button>');
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
    expect(markup).not.toContain('>Install</span></button>');
  });

  it('shows a docs-only installation guide without offering an absent command', () => {
    const markup = renderAgent({ id: 'custom-agent', name: 'Custom agent', available: true, install: { docs: 'https://example.com/install' } });
    expect(markup).toContain('Install guide');
    expect(markup).not.toContain('>Install</span></button>');
    expect(markup).not.toContain(t('settings.install.desc'));
  });
});

describe('in-app agent install', () => {
  const agent: AgentInfo = { id: 'codex', name: 'Codex', available: false, install: { command: 'npm install -g example-agent' } };
  const withTerminal: SettingsHandlers = { ...handlers, cancelInstall: vi.fn(), installInTerminal: vi.fn() };

  it('shows a running install with its route, its log and a Cancel action instead of Install', () => {
    const markup = renderAgent(agent, {}, {
      agent: 'codex', status: 'running', log: ['$ npm install -g example-agent', 'added 3 packages'],
      proxy: 'http://127.0.0.1:7890', prefix: '/preview/.local',
    }, withTerminal);
    expect(markup).toContain('Installing…');
    expect(markup).toContain('Proxy http://127.0.0.1:7890');
    expect(markup).toContain("installed to ~/.local");
    expect(markup).toContain('added 3 packages');
    expect(markup).toContain('>Cancel</span></button>');
    expect(markup).not.toContain('>Install</span></button>');
    // The terminal fallback stays on the command row, disabled while the in-app run holds the install
    expect(markup).toMatch(/aria-label="Run in terminal"[^>]*disabled|disabled[^>]*aria-label="Run in terminal"/);
  });

  it('shows a failure with its message and keeps the log; a success folds to its status line', () => {
    const failed = renderAgent(agent, {}, { agent: 'codex', status: 'failed', log: ['npm ERR! EACCES'], error: 'The installer exited with code 243.' }, withTerminal);
    expect(failed).toContain('Install failed');
    expect(failed).toContain('The installer exited with code 243.');
    expect(failed).toContain('npm ERR! EACCES');
    expect(failed).toContain('>Install</span></button>');
    const done = renderAgent(agent, {}, { agent: 'codex', status: 'success', log: ['secret noise'] }, withTerminal);
    expect(done).toContain('Installed');
    expect(done).not.toContain('secret noise');
  });

  it('offers no terminal fallback when the host has none', () => {
    expect(renderAgent(agent)).not.toContain('Run in terminal');
    expect(renderAgent(agent, {}, undefined, withTerminal)).toContain('Run in terminal');
  });
});
