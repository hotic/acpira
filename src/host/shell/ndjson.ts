import type { Readable } from 'node:stream';
import { StringDecoder } from 'node:string_decoder';

// Newline-delimited JSON framing: one record per '\n' (a trailing '\r' is dropped). node:readline is not usable here, since it
// also ends a line at U+2028 / U+2029, which serde_json writes unescaped inside strings: a transcript holding either character
// would arrive as broken halves and every envelope carrying it would be dropped
export function onNdjsonLines(input: Readable, onLine: (line: string) => void) {
  const decoder = new StringDecoder('utf8');
  let pending = '';
  const flush = (text: string) => {
    pending += text;
    // Walk the buffer once and keep only the unfinished tail, so a chunk of many small envelopes is not re-copied per line
    let start = 0;
    let nl: number;
    while ((nl = pending.indexOf('\n', start)) >= 0) {
      const line = pending.slice(start, nl);
      start = nl + 1;
      onLine(line.endsWith('\r') ? line.slice(0, -1) : line);
    }
    if (start) pending = pending.slice(start);
  };
  input.on('data', (chunk: Buffer | string) => flush(typeof chunk === 'string' ? chunk : decoder.write(chunk)));
  input.on('end', () => {
    flush(decoder.end());
    if (pending) onLine(pending);
    pending = '';
  });
}
