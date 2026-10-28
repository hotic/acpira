import type { DeviceStatus } from '@acpira/shared/devices';
import { t } from '../i18n';

const MIN = 60_000;

/** An offline device's second line: when it was last connected, or that it never was */
export function seenLine(d: DeviceStatus, now = Date.now()): string {
  if (!d.lastSeen) return t('devices.seen.never');
  const m = Math.max(0, Math.round((now - d.lastSeen) / MIN));
  if (m < 1) return t('devices.seen.now');
  if (m < 60) return t('devices.seen.minutes', { n: m });
  if (m < 1440) return t('devices.seen.hours', { n: Math.round(m / 60) });
  return t('devices.seen.days', { n: Math.round(m / 1440) });
}
