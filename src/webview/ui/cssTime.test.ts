import { describe, expect, it } from 'vitest';
import { parseCssTime } from './cssTime';

describe('parseCssTime', () => {
  it('reads the minified seconds form the shipped stylesheet uses', () => {
    expect(parseCssTime('.32s')).toBe(320);
    expect(parseCssTime(' 0.22s')).toBe(220);
    expect(parseCssTime('1s')).toBe(1000);
  });
  it('reads milliseconds as written in the source tokens', () => {
    expect(parseCssTime('320ms')).toBe(320);
    expect(parseCssTime(' 150ms ')).toBe(150);
    expect(parseCssTime('0ms')).toBe(0);
  });
  it('rejects values that are not a single time', () => {
    expect(parseCssTime('')).toBeUndefined();
    expect(parseCssTime('auto')).toBeUndefined();
    expect(parseCssTime('320')).toBeUndefined();
    expect(parseCssTime('calc(1s / 2)')).toBeUndefined();
  });
});
