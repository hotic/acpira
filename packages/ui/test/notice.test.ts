import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import type { AccountAction } from '@acpira/shared/protocol';
import { Notice, type NoticeProps } from '../chat/Notice';

const noop = () => {};
const props: NoticeProps = {
  status: 'auth_required',
  agent: { id: 'claude', name: 'Claude Code', accounts: true },
  onLogin: noop, onRetry: noop, onNewSession: noop, onSelectAccount: noop, onAddAccount: noop,
};

function actions(via: AccountAction['via']) {
  const html = renderToStaticMarkup(createElement(Notice, { ...props, accountAction: { agent: 'claude', via, status: 'pending' } }));
  return html.match(/<fieldset\b[^>]*>/)?.[0];
}

describe('sign-in notice recovery', () => {
  it.each(['login', 'auto'] as const)('keeps login and import reachable while %s waits on an external terminal', via => {
    expect(actions(via)).toBeDefined();
    expect(actions(via)).not.toContain('disabled=');
  });

  it('still blocks repeated imports while the local credentials are being read', () => {
    expect(actions('import')).toContain('disabled=');
  });

  it('does not show a stale saved-account line on an auth-required notice', () => {
    const html = renderToStaticMarkup(createElement(Notice, { ...props, accountAction: { agent: 'claude', via: 'import', status: 'success' } }));
    expect(html).not.toContain('Account saved.');
  });
});
