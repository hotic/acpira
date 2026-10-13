import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { AgentInfo } from '@shared/transcript';
import type { AgentInventory } from '@shared/inventory';
import type { AgentInstallProgress } from '@shared/protocol';
import { DEFAULT_SETTINGS } from '@shared/settings';
import { newModel, type ProviderModel, type ProviderView } from '@shared/providers';
import { setLocale, t } from '../i18n';
import { AgentPage } from './AgentPage';
import { editTokens, fmtTokens, ModelEditor, parseTokens } from './ProviderModel';
import type { ProvidersState } from './ProvidersSection';
import type { SettingsHandlers } from './SettingsShell';

// A dialog portals into the shell's layer once mounted, so the server renders none of it; here every part renders its
// children in place, which lets the model editor's dialog be read like any other markup
vi.mock('../ui/Dialog', async () => {
  const { createElement: h, Fragment } = await import('react');
  const pass = ({ children }: { children?: unknown }) => h(Fragment, null, children as never);
  const tag = (name: string) => ({ children, className }: { children?: unknown; className?: string }) => h(name, { className }, children as never);
  return { Dialog: { Root: pass, Portal: pass, Popup: tag('div'), Close: pass, Title: tag('h2'), Description: tag('p') } };
});

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

describe('built-in agent model sources', () => {
  const model = (id: string, extra: Partial<ProviderModel> = {}): ProviderModel => ({ ...newModel(id), ...extra });
  const sources: ProviderView[] = [
    { id: 'deepseek', name: 'DeepSeek', preset: 'deepseek', format: 'openai-chat', baseUrl: 'https://api.deepseek.com/v1', fullUrl: false, enabled: true, hasKey: true,
      models: [model('deepseek-v4-flash', { name: 'DeepSeek V4 Flash', context: 1_000_000, output: 393_216, efforts: ['low', 'high', 'max'], estimated: ['output'] })] },
    { id: 'openrouter', name: 'OpenRouter', preset: 'openrouter', format: 'openai-chat', baseUrl: 'https://openrouter.ai/api/v1', fullUrl: false, enabled: false, hasKey: false,
      models: [model('moonshotai/kimi-k3', { input: ['text', 'image'], context: 262_144 })] },
  ];
  const state: ProvidersState = { view: { providers: sources, families: ['generic', 'deepseek'] } };
  const builtin: AgentInfo = { id: 'acpira', name: 'Acpira', available: true };
  const render = () => renderToStaticMarkup(createElement(AgentPage, {
    agent: builtin, accounts: [], settings: DEFAULT_SETTINGS, env: { home: '/preview', cwd: '/preview/project' }, on: { ...handlers, providers: vi.fn(), providerProbe: vi.fn() },
    providers: state, controls: [],
  }));

  it('groups the models under their sources instead of the generic model list', () => {
    const markup = render();
    expect(markup).toContain('>DeepSeek<');
    expect(markup).toContain('>OpenRouter<');
    expect(markup).toContain('https://api.deepseek.com/v1');
    // The status dot carries the source's state as its label: a stored key, or switched off (with Turn on offered)
    expect(markup).toContain(`aria-label="${t('settings.providers.keySaved')}"`);
    expect(markup).toContain(`aria-label="${t('settings.providers.off')}"`);
    expect(markup).toContain(`>${t('settings.providers.turnOn')}<`);
    expect(markup).toContain('DeepSeek V4 Flash');
    // Compact rows: icon badges labelled in words, the full summary in the name's tooltip
    expect(markup).toContain('aria-label="Context 1M"');
    expect(markup).toContain(`aria-label="${t('settings.providers.model.image')}"`);
    const sep = t('common.metaSep');
    expect(markup).toContain(['Context 1M', 'Output 384k', 'Levels: 3'].join(sep));
    expect(markup).toContain(['Context 256k', 'Images'].join(sep));
    // Guessed values are named only in the editor, not as a marker on the row
    expect(markup).not.toContain('not reported by the endpoint');
    expect(markup).not.toContain(t('settings.models.desc', { agent: 'Acpira' }));
  });

  it('fills the model editor with the model\'s values', () => {
    // The editor is a dialog, which portals and so renders nothing on the server: render its contents directly
    const markup = renderToStaticMarkup(createElement(ModelEditor, {
      model: sources[0]!.models[0]!, title: 'Edit DeepSeek V4 Flash',
      onSave: vi.fn(), onCancel: vi.fn(), onRemove: vi.fn(), onTest: vi.fn(),
      test: { pending: false, outcome: { kind: 'failed', error: 'HTTP 401: Invalid API key' } },
    }));
    expect(markup).toContain('value="DeepSeek V4 Flash"');
    // 1000000 is not a whole 1M (1048576): the field shows the exact number, so saving keeps it
    expect(markup).toContain('value="1000000"');
    expect(markup).toContain('value="384k"');
    // The model's levels are lit toggles; the other common levels are offered unlit
    const lit = (level: string) => new RegExp(`aria-pressed="true"[^>]*>${level}<`).test(markup);
    expect(['Low', 'High', 'Max'].every(lit)).toBe(true);
    expect(['Minimal', 'Medium', 'XHigh'].some(lit)).toBe(false);
    // The fields the endpoint did not report are named at the top of the editor
    expect(markup).toContain('not reported by the endpoint: output.');
    expect(markup).toContain('HTTP 401: Invalid API key');
    expect(markup).toContain(t('settings.providers.model.test.hint'));
  });

  it('writes and reads token counts the way the quick values do', () => {
    expect([131_072, 1_048_576, 1_000_000, 393_216, 128_000, 512].map(fmtTokens)).toEqual(['128k', '1M', '1M', '384k', '128k', '512']);
    expect(['128k', '1M', '131072', ' 32K ', '1.5m'].map(parseTokens)).toEqual([131_072, 1_048_576, 131_072, 32_768, 1_572_864]);
    expect(['', 'abc', '0', '-5'].map(parseTokens)).toEqual([undefined, undefined, undefined, undefined]);
    // The editor's fields come from editTokens: saving an untouched field keeps the exact number
    const limits = [128_000, 64_000, 1_000_000, 131_072, 1_048_576, 393_216, 200_001];
    expect(limits.map(n => parseTokens(editTokens(n)))).toEqual(limits);
    expect([131_072, 128_000, 1_048_576].map(editTokens)).toEqual(['128k', '128000', '1M']);
  });
});
