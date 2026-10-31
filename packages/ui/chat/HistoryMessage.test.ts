import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import type { UserTurn } from '@acpira/shared/transcript';
import { setLocale } from '../i18n';
import { HistoryMessage } from './HistoryMessage';

setLocale('en');

const render = (turn: UserTurn) => renderToStaticMarkup(createElement(HistoryMessage, { turn, index: 0, turnIndex: 0 }));

describe('an automatic prompt', () => {
  it('delivering summoned agents\' replies shows each reporter handing back its first line, not the message', () => {
    const text = '(Acpira) The 2 summoned agents you sent off have finished. Their replies:\n\n## Codex Review (codex)\n\nLooks fine.\n\nMore detail.\n\n## Claude Planner (claude) asks\n\nShip it?';
    const html = render({ role: 'user', text, auto: true, autoReason: 'subagents' });
    expect(html).toContain('Codex Review');
    expect(html).toContain('Handed back');
    expect(html).toContain('Looks fine.');
    expect(html).not.toContain('More detail');
    expect(html).toContain('asks');
  });

  it('delivering only the sidekick\'s reply renders nothing: its relay already shows the handback', () => {
    const text = '(Acpira) A summoned agent you sent off has finished. Its reply:\n\n## Sidekick (acpira)\n\nHello.';
    expect(render({ role: 'user', text, auto: true, autoReason: 'subagents' })).toBe('');
  });

  it('of any other kind renders nothing', () => {
    expect(render({ role: 'user', text: 'continue', auto: true, autoReason: 'gate' })).toBe('');
  });
});
