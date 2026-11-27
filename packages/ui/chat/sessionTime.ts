import { dateFormat } from '../i18n';

export function startOfDay(d: Date) { return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime(); }

// The last path segment of a session's cwd (posix or Windows); a bare root or home shows as the path itself
export function projectName(cwd: string): string {
  const parts = cwd.split(/[\\/]+/).filter(Boolean);
  return parts[parts.length - 1] ?? cwd;
}

// An upper bound of a formatted time's width in ch: tabular digits are 1ch, separators and spaces narrower, letters (AM / PM, CJK
// date suffixes) wider; rounded up to a tenth so the column never clips its longest entry
export function timeWidth(text: string): number {
  let w = 0;
  for (const c of text) w += /\d/.test(c) ? 1 : /[\s:/.,-]/.test(c) ? 0.6 : /[\u3000-\u9fff]/.test(c) ? 2 : 1.6;
  return Math.ceil(w * 10) / 10;
}

// Today shows the time of day, earlier shows month/day — both follow the UI locale
export function fmtTime(iso: string, dayAgo: number, locale: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return '';
  if (dayAgo <= 0) return dateFormat(locale, { hour: 'numeric', minute: '2-digit' }).format(d);
  return dateFormat(locale, { month: 'numeric', day: 'numeric' }).format(d);
}
