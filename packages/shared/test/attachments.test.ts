import { describe, expect, it } from 'vitest';
import { stagedFileBlob } from '../attachments';

describe('stagedFileBlob', () => {
  it('finds session attachments independently of the local or remote data root', () => {
    const blob = 'c429caa26e072b19.mp4';
    expect(stagedFileBlob(`file:///Users/remote/My%20Data/sessions/s-one/${blob}`, 's-one')).toBe(blob);
    expect(stagedFileBlob(`/home/remote/.acpira/sessions/s-one/${blob}`, 's-one')).toBe(blob);
    expect(stagedFileBlob(`C:\\Data\\sessions\\s-one\\${blob}`, 's-one')).toBe(blob);
    for (const path of [`/repo/${blob}`, `/data/sessions/s-other/${blob}`, '/data/sessions/s-one/clip.mp4',
      `https://example.com/sessions/s-one/${blob}`, 'file:///data/sessions/s-one/%zz.mp4']) {
      expect(stagedFileBlob(path, 's-one')).toBeUndefined();
    }
  });
});
