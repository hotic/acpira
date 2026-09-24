// Cold-start and idle-memory comparison of the two sidecar engines over the real envelope protocol.
// Usage: node scripts/bench-sidecar.mjs [runs]   (needs `pnpm build:host` and `cargo build --release` in rust/)
// No agent process is involved: the configured agent command does not exist, so only the host itself is measured
import { spawn, execFileSync } from 'node:child_process';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { createInterface } from 'node:readline';

const runs = Number(process.argv[2]) || 5;
const engines = {
  node: ['node', ['dist/host-server.cjs']],
  rust: ['rust/target/release/acpira', []],
};

function once(cmd, args) {
  return new Promise((resolve, reject) => {
    const home = mkdtempSync(join(tmpdir(), 'acpira-bench-'));
    const t0 = performance.now();
    const p = spawn(cmd, [...args, '--home', home], { stdio: 'pipe', env: { ...process.env, ACPIRA_HOME: '' } });
    const send = m => p.stdin.write(`${JSON.stringify(m)}\n`);
    const r = { hello: 0, init: 0, rssKb: 0 };
    let tAttach = 0;
    createInterface({ input: p.stdout }).on('line', line => {
      const m = JSON.parse(line);
      if (m.type === 'helloOk') {
        r.hello = performance.now() - t0;
        tAttach = performance.now();
        send({ type: 'attachView', viewId: 'V', host: 'sidebar' });
        send({ type: 'webviewMessage', viewId: 'V', message: { type: 'ready' } });
      } else if (m.type === 'hostMessage' && m.message.type === 'init') {
        r.init = performance.now() - tAttach;
        setTimeout(() => {
          r.rssKb = Number(execFileSync('ps', ['-o', 'rss=', '-p', String(p.pid)]).toString().trim());
          send({ type: 'shutdown' });
        }, 1000);
      } else if (m.type === 'helloReject') reject(new Error(m.reason));
    });
    p.on('exit', () => { rmSync(home, { recursive: true, force: true }); resolve(r); });
    send({
      type: 'hello', protocolVersion: 1, requestId: 'h',
      client: { name: 'bench', version: '0', capabilities: [] },
      env: { hostLanguage: 'en', cwd: process.cwd() },
      settings: { defaultAgent: 'none', agents: { none: { name: 'None', command: '/nonexistent/agent' } } },
    });
  });
}

const median = xs => [...xs].sort((a, b) => a - b)[Math.floor(xs.length / 2)];
for (const [name, [cmd, args]] of Object.entries(engines)) {
  const rs = [];
  for (let i = 0; i < runs; i++) rs.push(await once(cmd, args));
  console.log(`${name.padEnd(5)} hello ${median(rs.map(r => r.hello)).toFixed(0).padStart(5)} ms  init ${median(rs.map(r => r.init)).toFixed(0).padStart(5)} ms  idle rss ${(median(rs.map(r => r.rssKb)) / 1024).toFixed(1).padStart(6)} MB  (median of ${runs})`);
}
