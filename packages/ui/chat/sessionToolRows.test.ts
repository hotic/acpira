import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ToolCallBlock } from '@acpira/shared/transcript';
import { sessionRead, splitSessionTarget } from '@acpira/shared/sessionLinks';
import { setLocale } from '../i18n';
import { ToolCall } from './ToolCall';
import { SessionTitleContext } from './fileLinks';
import { foldActivity, sessionReadLabel } from './folding';
import { ToolGroup } from './ToolGroup';
import { groupProcess } from './processGroups';
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
    expect(r).toContain('turns 2, 3');
    expect(r).not.toContain('acpira://session/abc');
    expect(render(search!)).toContain('lexer speed');
    // A session the list does not know shows its id
    expect(render({ ...read!, target: 'acpira://session/zzz' })).toContain('zzz');
  });

  it('name the turns in the fold heading while the read runs', () => {
    setLocale('zh-CN');
    expect(foldActivity({ role: 'agent', blocks: [{ ...read!, status: 'in_progress' }] })).toMatchObject({ label: '正在读取会话', target: '第 2、3 段对话' });
  });

  it('group a burst of reads of one session under one head that counts the turns, one entry per turn', () => {
    setLocale('zh-CN');
    const at = (id: string, target: string) => ({ ...read!, id, target });
    const reads = [at('r0', 'acpira://session/abc 3-3'), at('r1', 'acpira://session/abc 1-3'), at('r2', 'acpira://session/abc 3-3'), at('r3', 'acpira://session/abc')];
    // A thought ends a run, and so does another session
    const items = groupProcess([reads[0]!, reads[1]!, { type: 'thought', text: 'x' }, reads[2]!, reads[3]!, at('r4', 'acpira://session/zzz 1-2'), at('r5', 'acpira://session/zzz 4-4')]);
    expect(items.map(item => item.type === 'group' ? `${item.kind}:${item.blocks.length}` : item.type)).toEqual(['session:2', 'block', 'session:2', 'session:2']);
    const r = renderToStaticMarkup(createElement(SessionTitleContext.Provider, { value: titles }, createElement(ToolGroup, { kind: 'session', blocks: reads })));
    expect(r.match(/Fix lexer speed/g)).toHaveLength(1);
    expect(r).toContain(' 中的 3 段对话');
    // The outline first, then each turn once in order; a re-read adds no line
    expect([...r.matchAll(/(大纲|第 \d+ 段对话)/g)].map(m => m[1])).toEqual(['大纲', '第 1 段对话', '第 2 段对话', '第 3 段对话']);
    // An open range leaves the count out
    const open = renderToStaticMarkup(createElement(ToolGroup, { kind: 'session', blocks: [reads[0]!, at('r6', 'acpira://session/abc 5-')] }));
    expect(open).toContain('第 5 段对话起');
    expect(open).not.toContain('2 段对话');
  });

  it('split targets back into session and rest', () => {
    expect(splitSessionTarget('acpira://session/abc parser speed')).toEqual({ id: 'abc', rest: 'parser speed' });
    expect(splitSessionTarget('parser speed')).toEqual({ rest: 'parser speed' });
    expect(splitSessionTarget(undefined)).toEqual({});
    expect([sessionRead('3-5'), sessionRead('-2'), sessionRead('3-'), sessionRead(undefined), sessionRead('1-40')]).toEqual([
      { type: 'turns', turns: [3, 4, 5] }, { type: 'turns', turns: [1, 2] }, { type: 'from', from: 3 }, { type: 'outline' }, { type: 'span', from: 1, to: 40 }]);
    // A row lists a short run and spans a longer one
    expect([sessionReadLabel(sessionRead('3-5')), sessionReadLabel(sessionRead('1-9'))]).toEqual(['turns 3, 4, 5', 'turns 1–9']);
  });
});
