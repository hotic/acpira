import { PassThrough } from 'node:stream';
import { describe, expect, it } from 'vitest';
import { onNdjsonLines } from '../ndjson';

describe('onNdjsonLines', () => {
  // A read can end anywhere: inside a line, between '\r' and '\n', or inside a multi-byte character
  it('yields the same lines however the stream is cut into reads', async () => {
    const long = 'z'.repeat(50);
    const text = `{"a":1}\r\n{"b":"x y"}\n\n{"c":"${long}"}\ntail`;
    const bytes = Buffer.from(text);
    for (let size = 1; size <= bytes.length; size++) {
      const lines: string[] = [];
      const input = new PassThrough();
      onNdjsonLines(input, line => lines.push(line));
      const ended = new Promise(resolve => input.on('end', resolve));
      for (let i = 0; i < bytes.length; i += size) input.write(bytes.subarray(i, i + size));
      input.end();
      await ended;
      expect(lines, `reads of ${size} bytes`).toEqual(['{"a":1}', '{"b":"x y"}', '', `{"c":"${long}"}`, 'tail']);
    }
  });
});
