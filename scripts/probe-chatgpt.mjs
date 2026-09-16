// Isolated browser acceptance test. No real ChatGPT conversation or user profile is read.
import assert from 'node:assert/strict';
import { execFileSync, spawn } from 'node:child_process';
import { randomBytes } from 'node:crypto';
import { existsSync } from 'node:fs';
import { mkdir, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';

const root = process.cwd();
const fixture = await mkdtemp(join(tmpdir(), 'acpira-chatgpt-ui-'));
const home = join(fixture, 'profile');
const browserProfile = join(fixture, 'browser');
const cli = join(root, 'dist/chatgpt-bridge.cjs');
const token = randomBytes(24).toString('hex');
const chrome = process.env.CHROME_PATH ?? '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome';
if (!existsSync(chrome)) throw new Error('Set CHROME_PATH to a Chromium executable for this UI probe');
const env = { ...process.env, HOME: fixture, XDG_CONFIG_HOME: join(fixture, 'config') };
const invoke = (args, input) => execFileSync(process.execPath, [cli, ...args, '--home', home], { env, encoding: 'utf8', input });
const opened = JSON.parse(invoke(['open', '--key', 'isolated-ui-fixture', '--cwd', fixture, '--title', 'ChatGPT · UI acceptance fixture']));
const scope = ['--session', opened.sessionId, '--turn', 'ui-test'];
invoke(['prompt', ...scope, '--text', 'Isolated UI test: observe a real local command. This is not a user conversation.']);
invoke(['message', ...scope, '--message', 'progress', '--text', 'The local command will stream output into this mirror.']);

const socket = createServer();
await new Promise(resolve => socket.listen(0, '127.0.0.1', resolve));
const port = socket.address().port; await new Promise(resolve => socket.close(resolve));
const server = spawn(process.execPath, ['dist/host-server.cjs', '--ws', String(port), '--home', home, '--token', token], { cwd: root, env, stdio: ['ignore', 'pipe', 'pipe'] });
const browser = spawn(chrome, ['--headless=new', '--no-first-run', '--no-default-browser-check', '--disable-background-networking', '--disable-default-apps',
  '--remote-debugging-port=0', `--user-data-dir=${browserProfile}`, 'about:blank'], { stdio: ['ignore', 'pipe', 'pipe'] });
let ws; let command;
let browserErrors = [];
try {
  const debuggerUrl = await new Promise((resolve, reject) => {
    let buffer = ''; const timer = setTimeout(() => reject(new Error('Chromium debugger did not start')), 15_000);
    browser.stderr.on('data', data => {
      buffer += String(data); const match = buffer.match(/DevTools listening on (ws:\/\/\S+)/);
      if (match) { clearTimeout(timer); resolve(match[1]); }
    });
    browser.once('error', reject);
  });
  ws = new WebSocket(debuggerUrl);
  await new Promise((resolve, reject) => { ws.onopen = resolve; ws.onerror = reject; });
  let seq = 0; const pending = new Map();
  ws.onmessage = e => {
    const message = JSON.parse(e.data);
    if (message.method === 'Runtime.exceptionThrown') browserErrors.push(message.params.exceptionDetails.text);
    const p = pending.get(message.id);
    if (!p) return;
    pending.delete(message.id); clearTimeout(p.timer);
    if (message.error) p.reject(new Error(JSON.stringify(message.error))); else p.resolve(message.result);
  };
  const rpc = (method, params = {}, sessionId) => new Promise((resolve, reject) => {
    const id = ++seq; const timer = setTimeout(() => { pending.delete(id); reject(new Error(`CDP timeout: ${method}`)); }, 10_000);
    pending.set(id, { resolve, reject, timer }); ws.send(JSON.stringify({ id, method, params, sessionId }));
  });
  const { targetId } = await rpc('Target.createTarget', { url: 'about:blank' });
  const { sessionId } = await rpc('Target.attachToTarget', { targetId, flatten: true });
  const call = (method, params) => rpc(method, params, sessionId);
  const evaluate = async expression => {
    const result = await call('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true, userGesture: true });
    if (result.exceptionDetails) throw new Error(JSON.stringify(result.exceptionDetails));
    return result.result.value;
  };
  await call('Page.enable');
  await call('Runtime.enable');
  await call('Page.addScriptToEvaluateOnNewDocument', { source: `localStorage.setItem('acpira-host-preview.settings', JSON.stringify({ defaultAgent: 'chatgpt', language: 'zh-CN', sessionListPosition: 'left', 'appearance.motion': 'none' }));` });
  await call('Emulation.setDeviceMetricsOverride', { width: 1200, height: 850, deviceScaleFactor: 1, mobile: false });
  const url = `http://127.0.0.1:${port}/?token=${token}&host=editor&agent=chatgpt&cwd=${encodeURIComponent(fixture)}&session=${opened.sessionId}`;
  const deadline = Date.now() + 10_000;
  while (true) { try { await fetch(url); break; } catch { if (Date.now() > deadline) throw new Error('Harness did not start'); await new Promise(r => setTimeout(r, 50)); } }
  await call('Page.navigate', { url });
  const until = async (expression, ms = 10_000) => {
    const end = Date.now() + ms;
    while (Date.now() < end) { if (await evaluate(`Boolean(${expression})`)) return; await new Promise(r => setTimeout(r, 50)); }
    console.error(await evaluate('document.body.innerText'));
    throw new Error(`UI condition not reached: ${expression}`);
  };
  await until(`document.querySelector('[data-chatgpt-mirror]') && document.body.innerText.includes('Isolated UI test')`);
  assert.equal(await evaluate(`document.querySelectorAll('[contenteditable="true"]').length`), 0, 'Mirror must not offer a normal composer');
  await evaluate(`Object.defineProperty(navigator, 'clipboard', { value: { writeText: async text => { window.__probeClipboard = text; } }, configurable: true });`);
  await evaluate(`Array.from(document.querySelectorAll('button')).find(b => b.textContent.includes('复制连接指令')).click()`);
  await until(`window.__probeClipboard && window.__probeClipboard.includes(${JSON.stringify(opened.sessionId)})`);
  command = spawn(process.execPath, [cli, 'exec', ...scope, '--home', home, '--command', `${JSON.stringify(process.execPath)} -e "console.log('UI_LIVE_OUTPUT'); setTimeout(() => console.log('UI_FINISHED'), 8000)"`], { env, stdio: ['ignore', 'pipe', 'pipe'] });
  let commandError = '';
  command.stderr.on('data', data => { commandError += String(data); });
  command.stdout.resume();
  const finished = new Promise((resolve, reject) => command.on('exit', code => code === 0 ? resolve() : reject(new Error(`Test command exited ${code}: ${commandError}`))));
  void finished.catch(error => console.error(String(error)));
  await until(`document.querySelector('[data-chatgpt-mirror="receiving"]') && document.querySelector('[data-thread] button[aria-expanded]')`);
  // Verbs are localized. Observe the actual output card, not the English tool name.
  // Open the normal process disclosure so the actual streaming output is visible in the screenshot.
  await evaluate(`for (const b of document.querySelectorAll('button[aria-expanded="false"]')) { if (!b.closest('[data-session-panel]')) b.click(); }`);
  await until(`Array.from(document.querySelectorAll('.terminal-scroll')).some(el => el.innerText.includes('UI_LIVE_OUTPUT') && el.getBoundingClientRect().height > 0 && !el.closest('[inert]'))`);
  assert.equal(command.exitCode, null, 'Output must appear before the process exits');
  await mkdir(join(root, 'output'), { recursive: true });
  const shot = await call('Page.captureScreenshot', { format: 'png' });
  await writeFile(join(root, 'output/chatgpt-bridge-live.png'), Buffer.from(shot.data, 'base64'));
  await finished;
  invoke(['message', ...scope, '--message', 'final', '--phase', 'final', '--text', 'The isolated command completed successfully.']);
  invoke(['finish', ...scope]);
  await until(`document.querySelector('[data-chatgpt-mirror="idle"]') && document.body.innerText.includes('The isolated command completed successfully.')`);
  const finalShot = await call('Page.captureScreenshot', { format: 'png' });
  await writeFile(join(root, 'output/chatgpt-bridge-idle.png'), Buffer.from(finalShot.data, 'base64'));
  await call('Page.reload');
  await until(`document.querySelector('[data-chatgpt-mirror="idle"]') && document.body.innerText.includes('The isolated command completed successfully.')`);
  // The plus menu lists launchable agents only, while settings keep the external channel.
  await evaluate(`document.querySelector('button[aria-label="新会话"]').click()`);
  await until(`document.querySelector('[role="menu"]')`);
  assert.equal(await evaluate(`document.querySelector('[role="menu"]').innerText.includes('ChatGPT')`), false);
  await call('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Escape', code: 'Escape' });
  await call('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Escape', code: 'Escape' });
  await evaluate(`document.querySelector('button[aria-label="设置"]').click()`);
  await until(`document.querySelector('nav button[aria-label="ChatGPT"]')`);
  await evaluate(`document.querySelector('nav button[aria-label="ChatGPT"]').click()`);
  await until(`document.querySelector('[data-chatgpt-settings]') && document.body.innerText.includes('Desktop Commander') && document.body.innerText.includes('检测时间')`);
  assert.equal(await evaluate(`document.body.innerText.includes('云端配对未验证')`), true);
  assert.equal(await evaluate(`Array.from(document.querySelectorAll('[data-chatgpt-settings] button')).some(b => b.textContent.includes('打开已有会话'))`), true, 'The canonical project path must retain its received mirror');
  const settingsShot = await call('Page.captureScreenshot', { format: 'png' });
  await writeFile(join(root, 'output/chatgpt-settings.png'), Buffer.from(settingsShot.data, 'base64'));
  assert.deepEqual(browserErrors, [], 'No uncaught webview errors');
  console.log(JSON.stringify({ passed: true, checks: ['real webview', 'separate ChatGPT channel', 'no composer', 'connection instructions', 'live tool event', 'finish event', 'reload persistence', 'plus excludes ChatGPT', 'settings Desktop Commander', 'unknown pairing is explicit', 'canonical project binding', 'no browser exceptions'],
    screenshots: [resolve('output/chatgpt-bridge-live.png'), resolve('output/chatgpt-bridge-idle.png'), resolve('output/chatgpt-settings.png')] }, null, 2));
} finally {
  ws?.close(); command?.kill('SIGTERM'); server.kill('SIGTERM'); browser.kill('SIGTERM');
  await new Promise(r => setTimeout(r, 300));
  await rm(fixture, { recursive: true, force: true, maxRetries: 5, retryDelay: 150 });
}
