import type { Readable } from 'node:stream';
import { StringDecoder } from 'node:string_decoder';

// Newline-delimited JSON framing: one record per '\n' (a trailing '\r' is dropped). node:readline is not usable here, since it
// also ends a line at U+2028 / U+2029, which serde_json writes unescaped inside strings: a transcript holding either character
// would arrive as broken halves and every envelope carrying it would be dropped
export function onNdjsonLines(input: Readable, onLine: (line: string) => void) {
  const decoder = new StringDecoder('utf8');
  // The unfinished line, in the pieces it arrived in: only new text is searched for '\n', and the pieces are joined once
  // when the line ends. A multi-megabyte envelope (a long session's view) comes in 8 KB reads from a Unix socket on macOS,
  // and appending to one string and searching it from the start on every read made a 5 MB line take ~330 ms to assemble
  let parts: string[] = [];
  const flush = (text: string) => {
    let start = 0;
    let nl: number;
    while ((nl = text.indexOf('\n', start)) >= 0) {
      const tail = text.slice(start, nl);
      const line = parts.length ? parts.join('') + tail : tail;
      parts = [];
      start = nl + 1;
      onLine(line.endsWith('\r') ? line.slice(0, -1) : line);
    }
    if (start < text.length) parts.push(start ? text.slice(start) : text);
  };
  input.on('data', (chunk: Buffer | string) => flush(typeof chunk === 'string' ? chunk : decoder.write(chunk)));
  input.on('end', () => {
    flush(decoder.end());
    const rest = parts.join('');
    parts = [];
    if (rest) onLine(rest);
  });
}
