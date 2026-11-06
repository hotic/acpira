import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import type { UserTurn } from '@acpira/shared/transcript';
import { setLocale } from '../i18n';
import { HistoryContext, HistoryMessage } from './HistoryMessage';
import { parkDraftText } from './useComposerDraft';

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

describe('a prompt card', () => {
  it('closed over an unsent edit shows the edited text, not the original', () => {
    const turn: UserTurn = { role: 'user', id: 't1', text: 'open this, it seems built for windows' };
    const context = { sessionId: 's1', edit: async () => {}, select: () => {}, editable: true };
    const card = () => renderToStaticMarkup(createElement(HistoryContext.Provider, { value: context },
      createElement(HistoryMessage, { turn, index: 0, turnIndex: 0 })));
    expect(card()).toContain('built for windows');
    parkDraftText('edit:s1:t1', 'open this');
    expect(card()).toContain('open this');
    expect(card()).not.toContain('built for windows');
  });
});
