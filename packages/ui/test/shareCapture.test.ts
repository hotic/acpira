import { describe, expect, it } from 'vitest';
import { shareFileName } from '../chat/shareCapture';

describe('shareFileName', () => {
  it('turns session titles into clean PNG file names while keeping CJK characters', () => {
    expect(shareFileName('parseArgs 支持 --dry-run')).toBe('parseArgs-支持---dry-run.png');
    expect(shareFileName('  Fix: path/to*file? <test>  ')).toBe('Fix-path-to-file-test.png');
  });

  it('falls back when the title is blank or punctuation-only', () => {
    expect(shareFileName('   ')).toBe('acpira-share.png');
    expect(shareFileName('///:::***')).toBe('acpira-share.png');
  });
});
