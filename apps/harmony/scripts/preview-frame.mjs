// The last frame the OpenHarmony Previewer streams on its -lws port, written out as the JPEG inside the message
// Usage: node preview-frame.mjs <port> <out.jpg> [listen-ms]
import fs from 'node:fs';
const [port, out, waitMs] = [process.argv[2], process.argv[3], Number(process.argv[4] ?? 1500)];
const ws = new WebSocket(`ws://127.0.0.1:${port}`);
ws.binaryType = 'arraybuffer';
let last = null;
ws.onmessage = (e) => {
  if (typeof e.data === 'string') return;
  const b = Buffer.from(e.data);
  const soi = b.indexOf(Buffer.from([0xff, 0xd8, 0xff]));
  if (soi >= 0) last = b.subarray(soi);
};
ws.onerror = (e) => { console.error('ws error', e.message ?? e); process.exit(1); };
setTimeout(() => {
  if (!last) { console.error('no frame'); process.exit(2); }
  fs.writeFileSync(out, last);
  console.log('wrote', out, last.length);
  process.exit(0);
}, waitMs);
