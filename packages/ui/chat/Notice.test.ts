import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { agentsForFolder, pickFolderAgent } from '@acpira/shared/agentOrder';
import { SSH_LOCAL_AGENTS } from '@acpira/shared/remote';
import type { AgentInfo, AuthMethodInfo } from '@acpira/shared/transcript';
import { setLocale } from '../i18n';
import { Notice, type NoticeProps } from './Notice';

// DropdownMenu portals into the shell's overlay layer on the client; render its children inline so static markup includes
// every "Other ways" item.
vi.mock('../ui/DropdownMenu', async () => {
  const { createElement: h, Fragment } = await import('react');
  const pass = ({ children }: { children?: unknown }) => h(Fragment, null, children as never);
  const item = ({ children }: { children?: unknown }) => h('div', { role: 'menuitem' }, children as never);
  return {
    DropdownMenu: {
      Root: pass,
      Trigger: ({ render }: { render?: unknown }) => render as never,
      Portal: pass,
      Positioner: pass,
      Popup: ({ children }: { children?: unknown }) => h('div', { role: 'menu' }, children as never),
      Item: item,
      Separator: () => h('hr'),
    },
  };
});

function renderNotice(overrides: Partial<NoticeProps>): string {
  const base: NoticeProps = {
    status: 'error',
    agent: { id: 'antigravity', name: 'Antigravity', available: false, install: { command: 'curl -fsSL https://example.com/install.sh | sh' } },
    onLogin: vi.fn(),
    onRetry: vi.fn(),
    onNewSession: vi.fn(),
    onSelectAccount: vi.fn(),
    onAddAccount: vi.fn(),
    ...overrides,
  };
  return renderToStaticMarkup(createElement(Notice, base));
}

afterEach(() => setLocale('en'));

describe('Notice missing-CLI and SSH auth UX', () => {
  it('offers in-app Install, Run in terminal, and Run on this computer when the CLI is missing', () => {
    const markup = renderNotice({
      error: 'agy not found — install the Antigravity CLI first',
      onInstall: vi.fn(),
      onCancelInstall: vi.fn(),
      onInstallInTerminal: vi.fn(),
      onRunHere: vi.fn(),
    });
    expect(markup).toContain('Install Antigravity');
    expect(markup).toContain('agy not found — install the Antigravity CLI first');
    expect(markup).toContain('>Install</button>');
    expect(markup).toContain('Other ways');
    expect(markup).toContain('Run in terminal');
    expect(markup).toContain('Run Antigravity on this computer');
    expect(markup).toContain('aria-label="Retry"');
  });

  it('switches to Installing with newest log output and Cancel while an install runs, and exposes logs on failure', () => {
    const running = renderNotice({
      onInstall: vi.fn(),
      onCancelInstall: vi.fn(),
      onInstallInTerminal: vi.fn(),
      install: { agent: 'antigravity', status: 'running', log: ['$ curl ...', 'Downloading package', ''] },
    });
    expect(running).toContain('Installing…');
    expect(running).toContain('Downloading package');
    expect(running).toContain('>Cancel</button>');
    expect(running).not.toContain('>Install</button>');
    expect(running).not.toContain('Run in terminal');

    const failed = renderNotice({
      onInstall: vi.fn(),
      install: { agent: 'antigravity', status: 'failed', error: 'Exited with code 127', log: ['$ curl ...', 'curl: command not found'] },
    });
    expect(failed).toContain('Install failed');
    expect(failed).toContain('Exited with code 127');
    expect(failed).toContain('>Install</button>');
  });

  it('promotes terminal auth methods ahead of browser OAuth when remoteAuth is set', () => {
    const authMethods: AuthMethodInfo[] = [
      { id: 'oauth', name: 'Sign in with browser' },
      { id: 'term-login', name: 'Sign in in terminal', terminal: { args: ['--login'] } },
    ];
    const localMarkup = renderNotice({
      status: 'auth_required',
      agent: { id: 'antigravity', name: 'Antigravity', available: true },
      authMethods,
    });
    expect(localMarkup).toContain('>Sign in with browser</button>');

    const remoteMarkup = renderNotice({
      status: 'auth_required',
      agent: { id: 'antigravity', name: 'Antigravity', available: true },
      authMethods,
      remoteAuth: true,
      onRunHere: vi.fn(),
    });
    expect(remoteMarkup).toContain('>Sign in in terminal</button>');
    expect(remoteMarkup).toContain('Sign in with browser');
    expect(remoteMarkup).toContain('Run Antigravity on this computer');
  });
});

describe('SSH folder agent selection', () => {
  const localAgents: AgentInfo[] = [
    { id: 'acpira', name: 'Acpira', available: true },
    { id: 'claude', name: 'Claude Code', available: true },
    { id: 'antigravity', name: 'Antigravity', available: true },
    { id: 'codex', name: 'Codex', available: false },
  ];
  const hostAgents: AgentInfo[] = [
    { id: 'acpira', name: 'Acpira', available: true },
    { id: 'claude', name: 'Claude Code', available: false },
    { id: 'antigravity', name: 'Antigravity', available: false },
    { id: 'codex', name: 'Codex', available: true },
  ];

  it('includes antigravity in SSH_LOCAL_AGENTS so opting in keeps local credentials on SSH folders', () => {
    expect(SSH_LOCAL_AGENTS).toContain('antigravity');
    const merged = agentsForFolder(localAgents, 'ssh://gen1/work', hostAgents, SSH_LOCAL_AGENTS);
    expect(merged.find(a => a.id === 'antigravity')?.available).toBe(true);
    expect(merged.find(a => a.id === 'codex')?.available).toBe(true);
    expect(pickFolderAgent(localAgents, 'antigravity', 'ssh://gen1/work', '/local/repo', undefined, SSH_LOCAL_AGENTS)).toBe('antigravity');
  });

  it('falls back when an agent is set to remote-run and missing on the SSH host, or defers to an unprobed host', () => {
    const remoteRunForAgy = ['acpira', 'claude'];
    expect(
      pickFolderAgent(localAgents, 'antigravity', 'ssh://gen1/work', '/local/repo', undefined, remoteRunForAgy),
    ).toBeUndefined();
    expect(
      pickFolderAgent(localAgents, 'antigravity', 'ssh://gen1/work', 'ssh://gen1/other', hostAgents, remoteRunForAgy),
    ).toBe('acpira');
    expect(
      pickFolderAgent(localAgents, 'codex', 'ssh://gen1/work', 'ssh://gen1/other', hostAgents, remoteRunForAgy),
    ).toBe('codex');
  });
});
