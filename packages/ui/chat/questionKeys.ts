// Ignore named keys such as Tab and Shift: indexOf('') would otherwise select A.
export function questionOptionIndex(key: string): number | undefined {
  if (/^[1-9]$/.test(key)) return Number(key) - 1;
  if (/^[a-z]$/i.test(key)) return key.toUpperCase().charCodeAt(0) - 'A'.charCodeAt(0);
  return undefined;
}
