import { describe, expect, it } from 'vitest';
import { spacedPinyinCommit } from '../ui/ime';

describe('spacedPinyinCommit', () => {
  it('drops the syllable spaces of a raw commit of the shown pinyin', () => {
    expect(spacedPinyinCommit('de v in p ro', 'de v in p ro')).toBe('devinpro');
    expect(spacedPinyinCommit("xi'an ren", "xi'an ren")).toBe("xi'anren");
  });

  it('leaves candidates, correct raw commits and anything else alone', () => {
    expect(spacedPinyinCommit('的v in旁容', 'de v in p ro')).toBeUndefined();
    expect(spacedPinyinCommit('devinpro', 'de v in p ro')).toBeUndefined();
    expect(spacedPinyinCommit('hello world', undefined)).toBeUndefined();
    expect(spacedPinyinCommit('ni hao 1', 'ni hao 1')).toBeUndefined();
  });
});
