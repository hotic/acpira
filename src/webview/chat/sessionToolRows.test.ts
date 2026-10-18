import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ToolCallBlock } from '@shared/transcript';
import { splitSessionTarget, turnRange } from '@shared/sessionLinks';
import { setLocale } from '../i18n';
import { ToolCall } from './ToolCall';
import { SessionTitleContext } from './fileLinks';
import { foldActivity } from './folding';
import { agentTurn } from '../../../test/fixtures/engine';

vi.mock('./autoFold', async importOriginal => ({ ...await importOriginal<typeof import('./autoFold')>(), useAutoFold: () => undefined }));

afterEach(() => setLocale('en'));

// search_sessions / read_session rows (engine golden `session-tools`): the verb names the action, the session shows by title
describe('session tool rows', () => {
  const [search, read] = agentTurn('session-tools').blocks as ToolCallBlock[];
  const titles = (ref: string) => (ref === 'abc' ? 'Fix lexer speed' : undefined);
  const render = (block: ToolCallBlock) => renderToStaticMarkup(createElement(SessionTitleContext.Provider, { value: titles }, createElement(ToolCall, { block })));

  it('read the session title and turns instead of the link, and the query of a search', () => {
    const r = render(read!);
    expect(r).toContain('Read session');
    expect(r).toContain('Fix lexer speed');
    expect(r).toContain('turns 2–3');
    expect(r).not.toContain('acpira://session/abc');
    expect(render(search!)).toContain('lexer speed');
    // A session the list does not know shows its id
    expect(render({ ...read!, target: 'acpira://session/zzz' })).toContain('zzz');
  });

  it('name the turns in the fold heading while the read runs', () => {
    setLocale('zh-CN');
    expect(foldActivity({ role: 'agent', blocks: [{ ...read!, status: 'in_progress' }] })).toMatchObject({ label: '正在读取会话', target: '第 2–3 段对话' });
  });

  it('split targets back into session and rest', () => {
    expect(splitSessionTarget('acpira://session/abc parser speed')).toEqual({ id: 'abc', rest: 'parser speed' });
    expect(splitSessionTarget('parser speed')).toEqual({ rest: 'parser speed' });
    expect(splitSessionTarget(undefined)).toEqual({});
    expect([turnRange('3-5'), turnRange('3-3'), turnRange('-1'), turnRange('3-'), turnRange('-5')]).toEqual(['3–5', '3', '1', '3–', '1–5']);
  });
});
