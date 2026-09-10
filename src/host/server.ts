import { randomBytes } from 'node:crypto';
import { createInterface } from 'node:readline';
import { join, resolve } from 'node:path';
import { SidecarServer, type Wire } from './sidecar/SidecarServer';
import { startHarness } from './sidecar/ws';
import { acpiraHome } from './store/dataDir';
import { VERSION } from './version';

// The Node sidecar: `node dist/host-server.cjs` speaks the envelope protocol (shared/sidecar.ts) over stdio to a shell such as the
// IntelliJ plugin. stdout carries envelopes only; everything else (our log, stray console.log calls) goes to stderr.
// `--ws [port]` instead runs the browser harness (test/host-preview) with one sidecar per WebSocket connection. That mode
// defaults to `~/.acpira/harness`, requires a handshake token (printed in the listen URL; `--token` to pin it), and ignores
// client-supplied `agents`. `--home DIR` overrides ACPIRA_HOME for both modes

const args = process.argv.slice(2);
const flag = (name: string) => { const i = args.indexOf(name); return i >= 0 ? (args[i + 1] ?? '') : undefined; };
const explicitHome = flag('--home') ? resolve(flag('--home')!) : undefined;
const stderr = (line: string) => { process.stderr.write(`[acpira] ${line}\n`); };

// Anything in the dependency graph that prints must not land on the envelope channel
console.log = console.info = console.debug = (...a: unknown[]) => { console.error(...a); };

if (args.includes('--ws')) {
  const port = Number(flag('--ws')) || 7357;
  const home = explicitHome ?? join(acpiraHome(), 'harness');
  const token = flag('--token') || randomBytes(16).toString('hex');
  let sessionsDir: string | undefined;
  stderr(`harness home ${home}`);
  startHarness({
    port, root: process.cwd(), sessionsDir: () => sessionsDir, log: stderr, token,
    onWire: wire => {
      const server = new SidecarServer(wire, {
        version: VERSION, home, log: stderr, ignoreClientAgents: true,
        onExit: code => stderr(`harness connection closed (${code})`),
      });
      // The blob route needs the sessions directory the runtime settled on; read it back from the first helloOk
      const write = wire.write.bind(wire);
      wire.write = line => { if (!sessionsDir && line.includes('"helloOk"')) sessionsDir = (JSON.parse(line) as { sessionsDir: string }).sessionsDir; write(line); };
      server.start();
    },
  });
} else {
  const { wire, close } = stdioWire();
  const server = new SidecarServer(wire, {
    version: VERSION, home: explicitHome, log: stderr,
    // Let stdout drain before exiting so the last envelope (shutdownOk) reaches the shell
    onExit: code => { process.stdout.write('', () => process.exit(code)); },
  });
  server.start();
  for (const sig of ['SIGINT', 'SIGTERM'] as const) process.on(sig, close);
}

function stdioWire(): { wire: Wire; close: () => void } {
  const rl = createInterface({ input: process.stdin, crlfDelay: Infinity });
  let onClose: () => void = () => {};
  let closed = false;
  rl.on('close', () => { if (!closed) { closed = true; onClose(); } });
  const wire: Wire = {
    write: line => { process.stdout.write(`${line}\n`); },
    onLine: fn => { rl.on('line', fn); },
    onClose: fn => { onClose = fn; },
    end: () => { rl.close(); },
  };
  return { wire, close: () => rl.close() };
}
