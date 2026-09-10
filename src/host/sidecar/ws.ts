import { createHash } from 'node:crypto';
import { createReadStream, statSync } from 'node:fs';
import { createServer, type IncomingMessage, type Server, type ServerResponse } from 'node:http';
import type { AddressInfo } from 'node:net';
import { extname, join, normalize, resolve, sep } from 'node:path';
import type { Duplex } from 'node:stream';
import type { Wire } from './SidecarServer';

// A dependency-free HTTP + WebSocket (RFC 6455, text frames) server for the browser harness: serves the harness page, the real webview
// bundle and attachment blobs, and turns each /ws connection into a Wire. Development only — the IDE shell speaks stdio
export interface HarnessOpts {
  port: number;
  // Repository root: test/host-preview/* and dist/webview/* are served from it
  root: string;
  // Attachment blobs are served from here as /blobs/<sessionId>/<blob>
  sessionsDir: () => string | undefined;
  onWire: (wire: Wire) => void;
  log: (line: string) => void;
  // Required on /ws?token=; printed in the listen URL. Absent in unit tests that do not exercise auth
  token?: string;
}

const MIME: Record<string, string> = {
  '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8', '.mjs': 'text/javascript; charset=utf-8', '.css': 'text/css; charset=utf-8',
  '.json': 'application/json', '.svg': 'image/svg+xml', '.png': 'image/png', '.jpg': 'image/jpeg', '.jpeg': 'image/jpeg', '.gif': 'image/gif', '.webp': 'image/webp',
  '.woff2': 'font/woff2', '.woff': 'font/woff', '.txt': 'text/plain; charset=utf-8', '.md': 'text/markdown; charset=utf-8', '.wasm': 'application/wasm',
};

export function harnessOriginAllowed(origin: string | undefined, port: number): boolean {
  if (!origin) return false;
  let u: URL;
  try { u = new URL(origin); } catch { return false; }
  if (u.protocol !== 'http:' && u.protocol !== 'https:') return false;
  if (u.hostname !== '127.0.0.1' && u.hostname !== 'localhost') return false;
  const p = u.port ? Number(u.port) : (u.protocol === 'https:' ? 443 : 80);
  return p === port;
}

export function startHarness(opts: HarnessOpts): Server {
  const server = createServer((req, res) => serve(req, res, opts));
  const listenPort = () => {
    const addr = server.address();
    return addr && typeof addr === 'object' ? (addr as AddressInfo).port : opts.port;
  };
  server.on('upgrade', (req, socket) => {
    let url: URL;
    try { url = new URL(req.url ?? '/', 'http://127.0.0.1'); } catch { socket.destroy(); return; }
    if (url.pathname !== '/ws') { socket.destroy(); return; }
    if (opts.token && url.searchParams.get('token') !== opts.token) { socket.destroy(); return; }
    if (opts.token && !harnessOriginAllowed(req.headers.origin, listenPort())) { socket.destroy(); return; }
    const key = req.headers['sec-websocket-key'];
    if (typeof key !== 'string') { socket.destroy(); return; }
    const accept = createHash('sha1').update(`${key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11`).digest('base64');
    socket.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${accept}\r\n\r\n`);
    opts.onWire(wsWire(socket));
  });
  server.listen(opts.port, '127.0.0.1', () => {
    const q = opts.token ? `/?token=${opts.token}` : '/';
    opts.log(`harness listening on http://127.0.0.1:${listenPort()}${q}`);
  });
  return server;
}

// Static files under a root, confined to it; directories are refused
function serve(req: IncomingMessage, res: ServerResponse, opts: HarnessOpts) {
  const url = new URL(req.url ?? '/', 'http://localhost');
  let path: string;
  try { path = decodeURIComponent(url.pathname); } catch { res.writeHead(400); res.end(); return; }
  let base: string | undefined;
  let rel: string;
  if (path === '/' || path === '/index.html') { base = join(opts.root, 'test', 'host-preview'); rel = 'index.html'; }
  else if (path.startsWith('/host-preview/')) { base = join(opts.root, 'test', 'host-preview'); rel = path.slice('/host-preview/'.length); }
  else if (path.startsWith('/webview/')) { base = join(opts.root, 'dist', 'webview'); rel = path.slice('/webview/'.length); }
  else if (path.startsWith('/blobs/')) { base = opts.sessionsDir(); rel = path.slice('/blobs/'.length); }
  else { rel = ''; }
  if (!base || !rel) { res.writeHead(404); res.end(); return; }
  const file = resolve(base, normalize(rel));
  if (!file.startsWith(base + sep) || file === base) { res.writeHead(403); res.end(); return; }
  let size: number;
  try { const s = statSync(file); if (!s.isFile()) throw new Error('not a file'); size = s.size; } catch { res.writeHead(404); res.end(); return; }
  res.writeHead(200, { 'Content-Type': MIME[extname(file).toLowerCase()] ?? 'application/octet-stream', 'Content-Length': size, 'Cache-Control': 'no-store' });
  createReadStream(file).pipe(res);
}

// Frames from the browser are masked; ours are not. Fragmented text is reassembled; ping gets pong; close gets close
function wsWire(socket: Duplex): Wire {
  let buffer = Buffer.alloc(0);
  let fragments: Buffer[] = [];
  let onLine: (line: string) => void = () => {};
  let onClose: () => void = () => {};
  let closed = false;

  const frame = (opcode: number, payload: Buffer): Buffer => {
    const len = payload.length;
    const head = len < 126 ? Buffer.from([0x80 | opcode, len])
      : len < 65_536 ? Buffer.concat([Buffer.from([0x80 | opcode, 126]), u16(len)])
        : Buffer.concat([Buffer.from([0x80 | opcode, 127]), u64(len)]);
    return Buffer.concat([head, payload]);
  };
  const close = () => {
    if (closed) return;
    closed = true;
    socket.end();
    onClose();
  };

  socket.on('data', (chunk: Buffer) => {
    buffer = Buffer.concat([buffer, chunk]);
    for (;;) {
      if (buffer.length < 2) return;
      const fin = (buffer[0]! & 0x80) !== 0;
      const opcode = buffer[0]! & 0x0f;
      const masked = (buffer[1]! & 0x80) !== 0;
      let len = buffer[1]! & 0x7f;
      let off = 2;
      if (len === 126) { if (buffer.length < 4) return; len = buffer.readUInt16BE(2); off = 4; }
      else if (len === 127) { if (buffer.length < 10) return; len = Number(buffer.readBigUInt64BE(2)); off = 10; }
      const maskLen = masked ? 4 : 0;
      if (buffer.length < off + maskLen + len) return;
      const mask = masked ? buffer.subarray(off, off + 4) : undefined;
      const payload = Buffer.from(buffer.subarray(off + maskLen, off + maskLen + len));
      if (mask) for (let i = 0; i < payload.length; i++) payload[i] = payload[i]! ^ mask[i & 3]!;
      buffer = buffer.subarray(off + maskLen + len);
      if (opcode === 0x8) { if (!closed) socket.write(frame(0x8, payload.subarray(0, 2))); close(); return; }
      if (opcode === 0x9) { socket.write(frame(0xa, payload)); continue; }
      if (opcode === 0xa) continue;
      if (opcode === 0x1 || opcode === 0x2 || opcode === 0x0) {
        fragments.push(payload);
        if (!fin) continue;
        const text = Buffer.concat(fragments).toString('utf8');
        fragments = [];
        // A text frame may carry several lines
        for (const line of text.split('\n')) if (line.trim()) onLine(line);
      }
    }
  });
  socket.on('close', close);
  socket.on('error', close);

  return {
    write: line => { if (!closed) socket.write(frame(0x1, Buffer.from(line, 'utf8'))); },
    onLine: fn => { onLine = fn; },
    onClose: fn => { onClose = fn; },
    end: () => { if (!closed) socket.write(frame(0x8, Buffer.from([0x03, 0xe8]))); close(); },
  };
}

function u16(n: number): Buffer { const b = Buffer.alloc(2); b.writeUInt16BE(n); return b; }
function u64(n: number): Buffer { const b = Buffer.alloc(8); b.writeBigUInt64BE(BigInt(n)); return b; }
