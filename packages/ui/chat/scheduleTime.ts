import type { ToolCallBlock } from '@acpira/shared/transcript';
import { t } from '../i18n';

export function isScheduleTool(block: Pick<ToolCallBlock, 'verbKey'>): boolean {
  return block.verbKey === 'verb.schedule' || block.verbKey === 'verb.scheduleCron';
}

export interface ScheduleMetaDisplay {
  text: string;
  mono?: boolean;
  raw: string;
}

const pad2 = (n: number) => String(n).padStart(2, '0');
const intIn = (s: string, min: number, max: number): number | undefined => {
  if (!/^\d{1,2}$/.test(s)) return undefined;
  const n = Number(s);
  return n >= min && n <= max ? n : undefined;
};

/** Format Antigravity `schedule` metadata (`300s` or a 5-field cron) for a row's right-aligned trailing slot. */
export function formatScheduleMeta(meta: string): ScheduleMetaDisplay {
  const trimmed = meta.trim();
  const secsMatch = /^(\d+)s$/.exec(trimmed);
  if (secsMatch) {
    const total = Number(secsMatch[1]);
    const h = Math.floor(total / 3600);
    const rem = total % 3600;
    const m = Math.floor(rem / 60);
    const s = rem % 60;
    const time = h > 0
      ? (m > 0 ? t('schedule.hm', { h, m }) : t('schedule.h', { h }))
      : m > 0
        ? (s > 0 ? t('turns.elapsed.ms', { m, s }) : t('turns.elapsed.m', { m }))
        : t('turns.elapsed.s', { s });
    return { text: t('schedule.in', { time }), raw: trimmed };
  }

  const fields = trimmed.split(/\s+/);
  if (fields.length === 5) {
    const [min, hour, dom, mon, dow] = fields as [string, string, string, string, string];
    if (dom === '*' && mon === '*' && dow === '*') {
      const everyM = /^\*\/(\d+)$/.exec(min);
      if (everyM && hour === '*') {
        const n = Number(everyM[1]);
        if (n > 0) return { text: t('schedule.everyM', { m: n }), raw: trimmed };
      }
      if (min === '0' && hour === '*') {
        return { text: t('schedule.hourly'), raw: trimmed };
      }
      const everyH = /^\*\/(\d+)$/.exec(hour);
      if (min === '0' && everyH) {
        const n = Number(everyH[1]);
        if (n === 1) return { text: t('schedule.hourly'), raw: trimmed };
        if (n > 1) return { text: t('schedule.everyH', { h: n }), raw: trimmed };
      }
      const mVal = intIn(min, 0, 59);
      if (mVal !== undefined && hour === '*') {
        return { text: t('schedule.hourlyAt', { mm: pad2(mVal) }), raw: trimmed };
      }
      const hVal = intIn(hour, 0, 23);
      if (mVal !== undefined && hVal !== undefined) {
        return { text: t('schedule.daily', { time: `${pad2(hVal)}:${pad2(mVal)}` }), raw: trimmed };
      }
    }
    return { text: trimmed, mono: true, raw: trimmed };
  }

  return { text: trimmed, raw: trimmed };
}
