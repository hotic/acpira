import { describe, expect, it } from 'vitest';
import { formatBytes } from '../packages/shared/storage';

describe('formatBytes', () => {
  it('uses decimal units with one decimal below 10 and none from 10 up', () => {
    expect(formatBytes(0)).toBe('0 B');
    expect(formatBytes(999)).toBe('999 B');
    expect(formatBytes(1000)).toBe('1.0 KB');
    expect(formatBytes(8_049_000)).toBe('8.0 MB');
    expect(formatBytes(393_400_000)).toBe('393 MB');
    expect(formatBytes(999_700_000)).toBe('1.0 GB');
    expect(formatBytes(1_234_000_000, 'de')).toBe('1,2 GB');
  });
});
