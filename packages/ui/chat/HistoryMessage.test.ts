import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import type { UserTurn } from '@shared/transcript';
import { setLocale } from '../i18n';
import { HistoryMessage } from './HistoryMessage';

setLocale('en');

const render = (turn: UserTurn) => renderToStaticMarkup(createElement(HistoryMessage, { turn, index: 0, turnIndex: 0 }));

describe('an automatic prompt', () => {
  it('delivering summoned agents\' replies shows one line naming who reported, not the message', () => {
    const text = '(Acpira) The 2 summoned agents you sent off have finished. Their replies:\n\n## Codex Review (codex)\n\nLooks fine.\n\n## Claude Planner (claude)\n\nShip it.';
    const html = render({ role: 'user', text, auto: true, autoReason: 'subagents' });
    expect(html).toContain('Replies from');
    expect(html).toContain('Codex Review, Claude Planner');
    expect(html).not.toContain('Looks fine');
  });

  it('of any other kind renders nothing', () => {
    expect(render({ role: 'user', text: 'continue', auto: true, autoReason: 'gate' })).toBe('');
  });
});
