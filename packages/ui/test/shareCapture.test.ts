import { describe, expect, it } from 'vitest';
import { shareFileName, shareRasterSize } from '../chat/shareCapture';

describe('shareRasterSize', () => {
  it('keeps ordinary screenshots at 2x', () => {
    expect(shareRasterSize(700, 2_000)).toEqual({ width: 1_400, height: 4_000 });
  });

  it.each([[700, 40_000], [700, 100_000], [360, 1_000_000], [50_000, 50_000], [1, 1_000_000]])(
    'bounds the allocation for a %i × %i card, including scales below 1x', (width, height) => {
      const raster = shareRasterSize(width, height);
      expect(raster.width).toBeGreaterThanOrEqual(1);
      expect(raster.height).toBeGreaterThanOrEqual(1);
      expect(raster.width).toBeLessThanOrEqual(16_384);
      expect(raster.height).toBeLessThanOrEqual(16_384);
      expect(raster.width * raster.height).toBeLessThanOrEqual(16_000_000);
    },
  );
});

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
