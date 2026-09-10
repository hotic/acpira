import { createConnection, type AddressInfo } from 'node:net';
import { describe, expect, it } from 'vitest';
import { SidecarPlatform, type Hello } from '../src/host/sidecar/SidecarPlatform';
import { startHarness, harnessOriginAllowed } from '../src/host/sidecar/ws';
import { SIDECAR_PROTOCOL_VERSION } from '../src/shared/sidecar';

describe('harnessOriginAllowed', () => {
  it('allows loopback on the listen port and nothing else', () => {
    expect(harnessOriginAllowed('http://127.0.0.1:7357', 7357)).toBe(true);
    expect(harnessOriginAllowed('http://localhost:7357', 7357)).toBe(true);
    expect(harnessOriginAllowed('https://127.0.0.1:7357', 7357)).toBe(true);
    expect(harnessOriginAllowed('http://127.0.0.1:80', 7357)).toBe(false);
    expect(harnessOriginAllowed('http://evil.example', 7357)).toBe(false);
    expect(harnessOriginAllowed('http://127.0.0.1.attacker.test:7357', 7357)).toBe(false);
    expect(harnessOriginAllowed(undefined, 7357)).toBe(false);
  });
});

describe('SidecarPlatform ignoreAgents', () => {
  it('hides hello.settings.agents so the page cannot supply a command', () => {
    const hello: Hello = {
      type: 'hello', protocolVersion: SIDECAR_PROTOCOL_VERSION, requestId: 'h',
      client: { name: 't', version: '0', capabilities: [] },
      env: { hostLanguage: 'en' },
      settings: { agents: { evil: { command: '/bin/sh', args: ['-c', 'true'] } }, defaultAgent: 'grok' },
    };
    const open = new SidecarPlatform(() => {}, hello, () => {});
    expect(open.readSetting('agents')).toEqual(hello.settings.agents);
    const locked = new SidecarPlatform(() => {}, hello, () => {}, { ignoreAgents: true });
    expect(locked.readSetting('agents')).toBeUndefined();
    expect(locked.readSetting('defaultAgent')).toBe('grok');
  });
});

describe('startHarness websocket gate', () => {
  it('refuses upgrades without the token or from a foreign origin', async () => {
    const token = 'test-token';
    let wired = 0;
    const server = startHarness({
      port: 0, root: process.cwd(), sessionsDir: () => undefined, log: () => {}, token,
      onWire: () => { wired++; },
    });
    await new Promise<void>(r => server.once('listening', () => r()));
    const port = (server.address() as AddressInfo).port;
    try {
      expect((await upgrade(port, '/ws?token=nope', origin(port))).status).not.toBe(101);
      expect((await upgrade(port, `/ws?token=${token}`, 'Origin: http://evil.example\r\n')).status).not.toBe(101);
      expect((await upgrade(port, `/ws?token=${token}`, origin(port))).status).toBe(101);
      expect(wired).toBe(1);
    } finally {
      server.close();
    }
  });
});

function origin(port: number) { return `Origin: http://127.0.0.1:${port}\r\n`; }

function upgrade(port: number, path: string, extra: string): Promise<{ status: number }> {
  return new Promise((resolve, reject) => {
    const socket = createConnection({ port, host: '127.0.0.1' }, () => {
      socket.write(
        `GET ${path} HTTP/1.1\r\nHost: 127.0.0.1:${port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n${extra}\r\n`,
      );
    });
    let buf = Buffer.alloc(0);
    const done = (status: number) => { socket.destroy(); resolve({ status }); };
    socket.on('data', chunk => {
      buf = Buffer.concat([buf, chunk]);
      const text = buf.toString('utf8');
      if (!text.includes('\r\n\r\n')) return;
      const status = Number(text.split(' ')[1]);
      done(Number.isFinite(status) ? status : 0);
    });
    socket.on('close', () => done(0));
    socket.on('error', reject);
    setTimeout(() => done(0), 2_000);
  });
}
