import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ToolCallBlock } from '@acpira/shared/transcript';
import { setLocale } from '../i18n';
import { ToolCall } from './ToolCall';
import { formatScheduleMeta } from './scheduleTime';

vi.mock('./autoFold', async importOriginal => ({ ...await importOriginal<typeof import('./autoFold')>(), useAutoFold: () => undefined }));

afterEach(() => setLocale('en'));

describe('schedule tool rows', () => {
  it('formats durations and common cron expressions in the active locale', () => {
    setLocale('zh-CN');
    expect(formatScheduleMeta('45s')).toEqual({ text: '45 秒后', raw: '45s' });
    expect(formatScheduleMeta('300s')).toEqual({ text: '5 分钟后', raw: '300s' });
    expect(formatScheduleMeta('90s')).toEqual({ text: '1 分钟 30 秒后', raw: '90s' });
    expect(formatScheduleMeta('5400s')).toEqual({ text: '1 小时 30 分钟后', raw: '5400s' });
    expect(formatScheduleMeta('*/5 * * * *')).toEqual({ text: '每 5 分钟', raw: '*/5 * * * *' });
    expect(formatScheduleMeta('0 * * * *')).toEqual({ text: '每小时', raw: '0 * * * *' });
    expect(formatScheduleMeta('0 */2 * * *')).toEqual({ text: '每 2 小时', raw: '0 */2 * * *' });
    expect(formatScheduleMeta('0 9 * * *')).toEqual({ text: '每天 09:00', raw: '0 9 * * *' });
    expect(formatScheduleMeta('0 9 * * 1-5')).toEqual({ text: '0 9 * * 1-5', mono: true, raw: '0 9 * * 1-5' });

    setLocale('en');
    expect(formatScheduleMeta('300s').text).toBe('in 5m');
    expect(formatScheduleMeta('*/5 * * * *').text).toBe('Every 5m');
    expect(formatScheduleMeta('0 9 * * *').text).toBe('Daily 09:00');
  });

  it('places the formatted schedule time in the row trailing slot', () => {
    setLocale('zh-CN');
    const timer: ToolCallBlock = {
      type: 'tool_call', id: 's1', kind: 'other', verb: '定时唤醒', verbKey: 'verb.schedule',
      target: '检查 v2 语料生成进度', meta: '300s', status: 'completed',
    };
    const html = renderToStaticMarkup(createElement(ToolCall, { block: timer }));
    expect(html).toContain('已定时唤醒');
    expect(html).toContain('检查 v2 语料生成进度');
    expect(html).toContain('row-trailing');
    expect(html).toContain('5 分钟后');

    const cron: ToolCallBlock = {
      type: 'tool_call', id: 's2', kind: 'other', verb: '设置定时', verbKey: 'verb.scheduleCron',
      target: '轮询 GPU 训练进度', meta: '*/5 * * * *', status: 'in_progress',
    };
    const cronHtml = renderToStaticMarkup(createElement(ToolCall, { block: cron }));
    expect(cronHtml).toContain('正在设置定时');
    expect(cronHtml).toContain('轮询 GPU 训练进度');
    expect(cronHtml).toContain('每 5 分钟');
    expect(cronHtml).toContain('title="*/5 * * * *"');
  });
});
