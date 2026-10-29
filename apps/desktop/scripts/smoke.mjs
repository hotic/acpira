// End-to-end smoke of the Tauri desktop app (docs/dev/desktop.md): the real app on a temp ACPIRA_HOME, profile and HOME,
// with test/fake-agent.ts as its only agent, driven through the app's smoke hook (a Unix socket, a named pipe on Windows,
// in a build with the `smoke` feature: page scripts, window size, menu items, links). WKWebView has no DevTools protocol,
// so clicks and keys are DOM events the page script dispatches; native menus, input methods, Finder drags and system
// dialogs are on the hand-check list in docs/dev/desktop.md instead. On Windows the run takes `--app Acpira.exe` (a
// cross-built `--features smoke` exe) and skips the macOS-only steps: the drag pasteboard and screenshots. On Linux it
// runs on whatever display DISPLAY names (Xvfb with a window manager in a container: maximize needs one), with the
// debug build, an installed package's /usr/bin/Acpira or an extracted AppImage's AppRun (`--appimage-extract`, of a
// `package-linux.mjs --smoke` build) as --app, and skips the drag pasteboard and the update.
//
// The run stays in the background: the app takes no focus, shows no Dock icon and keeps its windows behind every other
// window (ACPIRA_DESKTOP_BACKGROUND), so it never interrupts whoever is using the Mac.
//
//   node scripts/smoke.mjs [--app <Acpira.app or its executable, built with --features smoke>] [--engine <acpira>]
//                          [--fake-agent <bundled fake-agent.mjs>] [--pack-tool <acpira-update-pack>] [--keep] [--shots DIR]
//
// --fake-agent takes test/fake-agent.ts already bundled (esbuild), for a machine without the repository's node_modules
// (the Windows PC runs this script beside the exe and the bundle)
//
// With --app <Acpira.app> the run also updates the app: it runs a copy of the bundle, packs a re-signed copy with a marker
// file as the next version (full package, a delta from this one, a manifest signed with a throwaway key), serves them
// on the loopback, and checks Ready → the restart toast → a new process from the replaced bundle on the same session.
// With --app <Acpira.exe> on Windows it does the same with a copy of the exe and a second copy with a marker appended
// (a PE runs with bytes after its last section), when it has the pack tool: --pack-tool, else built from rust/ here
// (a machine with only the exe and the fake agent needs a cross-built acpira-update-pack.exe; docs/dev/desktop.md).
// Without --app it builds and runs the debug binary (`cargo build --features smoke`) on the workspace's debug engine.
// Needs the repository's bundles (`pnpm build`). After the app quits the run waits out the engine's idle grace and checks
// that no agent process is left. --keep leaves the temp folder (logs, records); a failure always does
import { execFileSync, spawn, spawnSync } from 'node:child_process';
import { appendFileSync, chmodSync, copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, realpathSync, rmSync, statSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { createServer } from 'node:http';
import { connect } from 'node:net';
import { tmpdir } from 'node:os';
import { basename, delimiter, dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const SHELL = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const REPO = resolve(SHELL, '..', '..');

const argv = process.argv.slice(2);
const option = name => { const i = argv.indexOf(name); return i >= 0 ? argv[i + 1] : undefined; };
const appArg = option('--app');
const keep = argv.includes('--keep');
const shotDir = option('--shots') && resolve(option('--shots'));
const fakeAgentArg = option('--fake-agent') && resolve(option('--fake-agent'));
const packToolArg = option('--pack-tool') && resolve(option('--pack-tool'));
let engine = option('--engine') ?? process.env.ACPIRA_SIDECAR_BIN?.trim();

const WIN = process.platform === 'win32';
const MAC = process.platform === 'darwin';
const LINUX = process.platform === 'linux';
// The app protocol's origin as the page sees it (acpira_desktop::page::app_origin)
const ORIGIN = WIN ? 'http://acpira-app.localhost' : 'acpira-app://localhost';
// A one-page PDF saying `text`, its cross-reference offsets counted
function onePagePdf(text) {
  const objects = [
    '<< /Type /Catalog /Pages 2 0 R >>',
    '<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
    '<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 100] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>',
    `<< /Length ${`BT /F1 18 Tf 20 40 Td (${text}) Tj ET`.length} >>\nstream\nBT /F1 18 Tf 20 40 Td (${text}) Tj ET\nendstream`,
    '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>',
  ];
  let out = '%PDF-1.4\n';
  const at = objects.map((o, i) => { const offset = out.length; out += `${i + 1} 0 obj\n${o}\nendobj\n`; return offset; });
  const xref = out.length;
  out += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n${at.map(o => `${String(o).padStart(10, '0')} 00000 n \n`).join('')}`;
  return `${out}trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
}
const sleep = ms => new Promise(r => setTimeout(r, ms));
const say = line => console.log(`smoke: ${line}`);
class SmokeError extends Error {}
const fail = message => { throw new SmokeError(message); };

// A short base: the engine's socket path under the temp home has to stay below 100 bytes (Windows uses pipes)
const root = realpathSync(mkdtempSync(WIN ? join(tmpdir(), 'acpira-tsmoke-') : '/tmp/acpira-tsmoke-'));
const home = join(root, 'home');
const data = join(root, 'data');
const user = join(root, 'user');
const alpha = join(root, 'smoke-alpha');
const beta = join(root, 'smoke-beta');
const fakeAgent = join(root, 'fake-agent.mjs');
const hookPath = WIN ? `\\\\.\\pipe\\acpira-smoke-${process.pid}` : join(root, 'hook.sock');
const logFile = join(root, 'app.log');
const desktopLog = join(root, 'user', 'logs', 'desktop.log');
let appLog = '';

let app;
let page;
let passed = false;

async function main() {
  try {
    // The Windows exe carries the page; everything else reads it from the repository's dist/
    if (!(WIN && appArg)) for (const f of ['dist/webview/main.js', 'dist/picker.js']) if (!existsSync(join(REPO, f))) fail(`${f} is missing: run pnpm build in the repository root`);

    let bin;
    let release;
    if (appArg) {
      bin = appArg.endsWith('.app') ? join(appArg, 'Contents', 'MacOS', 'Acpira') : appArg;
      if (!existsSync(bin)) fail(`no executable at ${bin}`);
      // The Windows exe is its own engine
      if (WIN) engine ??= bin;
      if (appArg.endsWith('.app')) {
        release = await prepareUpdate(resolve(appArg));
        bin = join(release.installed, 'Contents', 'MacOS', 'Acpira');
      } else if (WIN) {
        release = await prepareExeUpdate(resolve(appArg));
        // The run's copy is the app and its own engine: after the update the same path is the new exe
        if (release) {
          bin = release.installed;
          if (!option('--engine')) engine = bin;
        }
      }
    } else {
      execFileSync('cargo', ['build', '--quiet', '--features', 'smoke'], { cwd: SHELL, stdio: 'inherit' });
      bin = join(SHELL, 'target', 'debug', WIN ? 'acpira-desktop-tauri.exe' : 'acpira-desktop-tauri');
      if (WIN) engine ??= bin;
      if (!engine) {
        execFileSync('cargo', ['build', '--quiet', '--bin', 'acpira'], { cwd: join(REPO, 'rust'), stdio: 'inherit' });
        engine = join(REPO, 'rust', 'target', 'debug', 'acpira');
      }
    }
    // The engine beside the shell (macOS bundle); a Linux package's in <prefix>/libexec/acpira, which is usr/ in an
    // extracted AppImage
    const linuxEngine = prefix => join(prefix, 'libexec', 'acpira', 'acpira-engine');
    const agentsBin = engine ?? (basename(bin) === 'AppRun' ? linuxEngine(join(dirname(bin), 'usr')) : LINUX ? linuxEngine(dirname(dirname(bin))) : join(dirname(bin), 'acpira-engine'));

    // No data folder: the run starts as a first launch, whose consent card creates it
    for (const d of [home, user, alpha, beta, ...(shotDir ? [shotDir] : [])]) mkdirSync(d, { recursive: true });
    for (const [dir, file] of [[alpha, 'alpha-notes.md'], [beta, 'beta-notes.md']]) {
      execFileSync('git', ['init', '-q', dir]);
      writeFileSync(join(dir, file), `# ${file}\n`);
    }
    mkdirSync(join(alpha, 'docs'), { recursive: true });
    writeFileSync(join(alpha, 'docs', 'guide.md'), '# guide\n');
    // Media for the viewer: a 2 × 1 PNG, an SVG, and where ffmpeg is at hand a noise clip larger than one /file chunk with its
    // index at the end (ffmpeg's default), so the player has to read it by range before it can show a frame
    writeFileSync(join(alpha, 'docs', 'dot.png'), Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAIAAAABCAIAAAB7QOjdAAAADUlEQVR4nGP4z8AARAAI/gH/xp559wAAAABJRU5ErkJggg==', 'base64'));
    writeFileSync(join(alpha, 'docs', 'logo.svg'), '<svg xmlns="http://www.w3.org/2000/svg" width="3" height="2"><rect width="3" height="2" fill="red"/></svg>');
    writeFileSync(join(alpha, 'docs', 'page.pdf'), onePagePdf('smoke pdf'));
    const clip = spawnSync('ffmpeg', ['-v', 'error', '-f', 'lavfi', '-i', 'nullsrc=s=320x240:r=30:d=5,geq=random(1)*255:128:128', '-c:v', 'libx264', '-preset', 'ultrafast', '-pix_fmt', 'yuv420p', '-y', join(alpha, 'docs', 'clip.mp4')]).status === 0;
    if (fakeAgentArg) writeFileSync(fakeAgent, readFileSync(fakeAgentArg));
    else {
      const { buildSync } = await import('esbuild');
      buildSync({ entryPoints: [join(REPO, 'test', 'fake-agent.ts')], outfile: fakeAgent, bundle: true, platform: 'node', format: 'esm', logLevel: 'warning' });
    }

    const env = {
      ...process.env, HOME: home,
      // Windows keeps per-user data under these, VS Code's settings (the shared settings' seed) among them
      ...(WIN ? { USERPROFILE: home, APPDATA: join(home, 'AppData', 'Roaming'), LOCALAPPDATA: join(home, 'AppData', 'Local') } : {}),
      // Linux: the XDG directories a session may set away from HOME (the app's dirs, WebKitGTK's, GTK's settings);
      // DISPLAY, the session bus and XDG_RUNTIME_DIR stay, they are the display and the desktop the run talks to
      ...(LINUX ? { XDG_CONFIG_HOME: join(home, '.config'), XDG_CACHE_HOME: join(home, '.cache'), XDG_DATA_HOME: join(home, '.local', 'share'), XDG_STATE_HOME: join(home, '.local', 'state') } : {}),
      ACPIRA_HOME: data, ACPIRA_DESKTOP_USER_DATA: user, ACPIRA_DESKTOP_PICK: [alpha, beta].join(delimiter),
      ACPIRA_DESKTOP_HOOK: hookPath, ACPIRA_DESKTOP_BACKGROUND: '1',
    };
    for (const k of ['ACPIRA_ENGINE', 'ACPIRA_SIDECAR_BIN', 'SSH_CONNECTION', 'SSH_CLIENT', 'SSH_TTY']) delete env[k];
    if (engine) env.ACPIRA_SIDECAR_BIN = engine;
    if (release) Object.assign(env, { ACPIRA_DESKTOP_UPDATE_URL: release.manifest, ACPIRA_DESKTOP_UPDATE_KEY: release.publicKey });

    const probeEnv = { ...env, ACPIRA_HOME: join(root, 'agents-probe') };
    const builtins = JSON.parse(execFileSync(agentsBin, ['agents', '--json'], { encoding: 'utf8', env: probeEnv })).agents.map(a => a.id);
    writeFileSync(join(user, 'settings.json'), `${JSON.stringify({
      sessionListPosition: 'left',
      sessionScope: 'all',
      language: 'en',
      defaultAgent: 'fake',
      agentOrder: ['fake'],
      disabledAgents: builtins,
      agents: { fake: { name: 'Fake', command: process.execPath, args: [fakeAgent] } },
    }, null, 2)}\n`);

    const launch = extra => spawn(bin, extra, { env, stdio: ['ignore', 'pipe', 'pipe'] });
    app = launch([]);
    const collect = chunk => { const s = chunk.toString(); appLog += s; appendFileSync(logFile, s); };
    app.stdout.on('data', collect);
    app.stderr.on('data', collect);
    const appExit = new Promise(r => app.once('exit', r));
    say(`app ${appArg ?? basename(bin)} on ${engine ?? 'its bundled engine'}, temp ${root}`);

    page = await Hook.connect(hookPath);
    await page.waitFor('the chat page', `!!document.querySelector('.acp-shell')`);
    const origin = await page.eval('location.protocol + "//" + location.host + " " + isSecureContext + " " + history.length');
    if (origin !== `${ORIGIN} true 1`) fail(`the page runs on ${origin} (expected ${ORIGIN}, a secure context, no back entry)`);

    // First launch: no engine runs (it would create the folder), the intro lists what `agents --json` found, and the
    // consent card's yes creates the folder, private to the user, then starts the engine, whose init swaps the sample
    // shell for the real one
    await page.waitFor('the first-run intro', `!!document.querySelector('.wl-intro[data-open]')`);
    const firstRun = await page.eval('JSON.stringify(window.__acpira.firstRun)');
    if (firstRun !== JSON.stringify({ home: data, locale: 'en' })) fail(`the first-run page says ${firstRun}`);
    await page.waitFor('the intro\'s agent row', `!!document.querySelector('.wl-agent, .wl-none')`);
    if (existsSync(data)) fail('the data folder exists before the consent card');
    if (shotDir) { await sleep(1600); await page.shot(join(shotDir, 'welcome-intro.png')); }
    await page.click('the intro\'s skip', `document.querySelector('.wl-intro .wl-quiet')`);
    await page.waitFor('consent after skipping', `!!document.querySelector('.wl-center[data-open]')`);
    if (existsSync(data)) fail('skipping the intro created the data folder before consent');
    await page.click('see the tour after skipping', `document.querySelector('.wl-center[data-open] button')`);
    await page.waitFor('the intro again', `!!document.querySelector('.wl-intro[data-open]')`);
    await page.click('the intro\'s start', `document.querySelector('.wl-intro[data-open] .wl-cta')`);
    const checkTour = async (step, name, target) => {
      await page.waitFor(`tour step ${step + 1}: ${name}`, `document.querySelectorAll('.wl-step')[${step}]?.classList.contains('is-now')`);
      // Wait for the hole's glide and the caption's entrance before measuring their real painted boxes.
      await sleep(1800);
      const error = await page.eval(`(() => {
        const hole = document.querySelector('.wl-hole-edge')?.getBoundingClientRect();
        const note = document.querySelector('.wl-note:not(.is-out)')?.getBoundingClientRect();
        const dock = document.querySelector('.wl-dock')?.getBoundingClientRect();
        const inside = r => r && r.width > 0 && r.height > 0 && r.left >= 0 && r.top >= 0 && r.right <= innerWidth && r.bottom <= innerHeight;
        if (!inside(hole) || !inside(note) || !inside(dock)) return 'hole, caption or controls outside the window: ' + JSON.stringify({ hole, note, dock });
        const overlaps = (a, b) => a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top;
        if (overlaps(dock, note) || overlaps(dock, hole)) return 'controls cover the caption or spotlight: ' + JSON.stringify({ hole, note, dock });
        const lead = document.querySelector('.wl-leader:not(.is-out) .wl-lead');
        if (!lead) return 'the active step has no leader';
        // Sample the painted path, not its bounding box: an elbow can safely go around the capsule.
        const length = lead.getTotalLength(), matrix = lead.getScreenCTM();
        for (let i = 0; i <= 100; i++) {
          const p = lead.getPointAtLength(length * i / 100).matrixTransform(matrix);
          if (p.x >= dock.left && p.x <= dock.right && p.y >= dock.top && p.y <= dock.bottom) return 'controls cover the leader';
        }
        ${target ? `const r = (${target})?.getBoundingClientRect();
        if (!r || r.x + r.width / 2 < hole.left || r.x + r.width / 2 > hole.right || r.y + r.height / 2 < hole.top || r.y + r.height / 2 > hole.bottom) return 'spotlight missed its visible control';
        ${step === 2 ? `if (Math.abs(hole.left - r.left) > 3 || Math.abs(hole.right - r.right) > 3 || Math.abs(hole.bottom - r.bottom) > 3) return 'composer spotlight includes the dock padding';` : ''}` : ''}
        return '';
      })()`);
      if (error) fail(`tour ${name}: ${error}`);
      if (existsSync(data)) fail('the tour created the data folder before consent');
      if (shotDir) await page.shot(join(shotDir, `welcome-${name}.png`));
    };
    await checkTour(0, 'sessions', `document.querySelector('[data-sample-shell] [data-session]')`);
    for (const [i, name] of ['approvals', 'composer', 'settings'].entries()) {
      await page.click('the tour\'s next', `document.querySelector('.wl-next')`);
      await checkTour(i + 1, name, i === 2 ? `document.querySelector('[data-sample-shell] [data-session-panel] svg.lucide-settings')` : i === 1 ? `document.querySelector('[data-sample-shell] .composer-field')` : undefined);
    }
    for (const width of [680, 480]) {
      await page.call({ op: 'resize', width, height: 760 });
      await checkTour(3, `narrow-settings-${width}`, `document.querySelector('[data-sample-shell] [data-sheet-header] svg.lucide-ellipsis')`);
      for (let i = 0; i < 3; i++) await page.eval(`window.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowLeft' }))`);
      await checkTour(0, `narrow-sessions-${width}`, `document.querySelector('[data-sample-shell] [data-title-row] [data-session-toggle]')`);
      if (width === 680) for (let i = 0; i < 3; i++) await page.click('the tour\'s next', `document.querySelector('.wl-next')`);
    }
    await page.call({ op: 'resize', width: 760, height: 760 });
    await checkTour(0, 'sessions-at-breakpoint', `document.querySelector('[data-sample-shell] [data-session]')`);
    await page.call({ op: 'resize', width: 1180, height: 820 });
    await checkTour(0, 'sessions-widened', `document.querySelector('[data-sample-shell] [data-session]')`);
    for (let i = 0; i < 4; i++) await page.click('the tour\'s next', `document.querySelector('.wl-next')`);
    await page.waitFor('the consent card', `!!document.querySelector('.wl-center[data-open]')`);
    if (!(await page.eval(`document.querySelector('.wl-center[data-open] button')?.textContent.includes('Back')`))) fail('finishing the tour after skipping kept the see-tour action');
    await page.click('back to the last tour step', `document.querySelector('.wl-center[data-open] button')`);
    await checkTour(3, 'settings-after-consent-back', `document.querySelector('[data-sample-shell] [data-session-panel] svg.lucide-settings')`);
    await page.click('finish the tour again', `document.querySelector('.wl-next')`);
    await page.waitFor('consent after returning', `!!document.querySelector('.wl-center[data-open]')`);
    if (shotDir) { await sleep(800); await page.shot(join(shotDir, 'welcome-consent.png')); }
    await page.press('the create button', `[...document.querySelectorAll('.wl-center button')].find(b => b.innerText.includes('Create and start'))`);
    await waitUntil('the data folder', () => existsSync(data), 10_000);
    if (!WIN && (statSync(data).mode & 0o777) !== 0o700) fail(`the data folder has mode ${(statSync(data).mode & 0o777).toString(8)}`);
    await page.waitFor('the first-run layer to go', `!document.querySelector('.wl-layer') && !document.querySelector('[data-sample-shell]')`);
    say('first launch: intro with no engine, the data folder created on consent, then the real shell');

    await page.waitFor('the empty state', `document.body.innerText.includes('Open a folder to start')`);
    await sleep(1000);
    if (await page.eval(`document.querySelectorAll('[data-session]').length`)) fail('a session row showed on a fresh home');
    if (records().length) fail(`a fresh home wrote session records: ${records().join(', ')}`);
    say(`fresh home on ${ORIGIN} shows the empty state and starts no session`);

    await page.click('the empty state button', `[...document.querySelectorAll('button')].find(b => b.innerText.includes('Open folder'))`);
    await page.waitFor('the first project', `document.body.innerText.includes('smoke-alpha')`);
    const a = await roundTrip('smoke alpha');
    say(`project smoke-alpha: session ${a}, one round trip`);

    await page.press('the add menu', `document.querySelector('button[aria-label="New"]')`);
    await page.press('the add project item', `[...document.querySelectorAll('[role="menuitem"]')].find(e => e.textContent.trim() === 'Add project')`);
    await page.waitFor('the second project\'s start page', `document.body.innerText.includes('smoke-beta') && !document.body.innerText.includes('hello world')`);
    const inBeta = await mentions();
    if (!inBeta.some(n => n.includes('beta-notes.md')) || inBeta.some(n => n.includes('alpha-notes.md'))) fail(`@ search in smoke-beta listed: ${inBeta.join(' | ')}`);
    say('@ search in smoke-beta lists only its own file');
    const b = await roundTrip('smoke beta');
    if (b === a) fail('both projects show the same session');
    say(`project smoke-beta: session ${b}, one round trip`);

    // A second launch with the link hands it to this instance (single instance socket), which brings session A back
    await waitUntil('session A\'s record', () => existsSync(join(data, 'sessions', `${a}.json`)), 10_000);
    const second = launch([`acpira://session/${a}`]);
    second.stdout.on('data', collect);
    second.stderr.on('data', collect);
    await within('the second instance to hand its link over and exit', new Promise(r => second.once('exit', r)), 15_000);
    await page.waitFor('session A after its link', `document.querySelector('[data-session][aria-selected="true"]')?.getAttribute('data-session') === ${JSON.stringify(a)} && document.body.innerText.includes('smoke alpha')`);
    say('acpira://session link from a second launch brought session A back');
    const inAlpha = await mentions();
    if (!inAlpha.some(n => n.includes('alpha-notes.md')) || inAlpha.some(n => n.includes('beta-notes.md'))) fail(`@ search in smoke-alpha listed: ${inAlpha.join(' | ')}`);
    say('@ search in smoke-alpha lists only its own file');

    // The packaged app keeps assets/ zstd-compressed (scripts/package.mjs): each answers under its own name with the
    // page's original bytes and type
    if (release?.kind === 'app') {
      const assets = join(release.installed, 'Contents', 'Resources', 'webview', 'assets');
      const packed = readdirSync(assets).filter(n => n.endsWith('.zst')).map(n => n.slice(0, -'.zst'.length));
      if (!packed.length) fail('the bundle has no packed assets');
      const got = await page.eval(`Promise.all(${JSON.stringify(packed)}.map(async n => {
        const r = await fetch('acpira-app://localhost/webview/assets/' + n);
        const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', await r.arrayBuffer()));
        return [n, r.status, r.headers.get('content-type'), [...digest].map(b => b.toString(16).padStart(2, '0')).join('')];
      }))`);
      for (const [name, status, type, digest] of got) {
        const want = createHash('sha256').update(readFileSync(join(REPO, 'dist', 'webview', 'assets', name))).digest('hex');
        if (status !== 200 || digest !== want) fail(`packed ${name} answered ${status} (${type}) with other bytes than dist/webview`);
      }
      const types = new Set(got.map(([, , type]) => type));
      if (!types.has('text/javascript; charset=utf-8') || !types.has('font/woff2')) fail(`packed asset types: ${[...types].join(', ')}`);
      say(`${packed.length} packed assets answer with their original bytes (${types.size} types)`);
    }

    // A link to a session this machine does not have: the page says so
    await page.call({ op: 'link', url: 'acpira://session/00000000-0000-4000-8000-000000000000' });
    await page.waitFor('the not-here toast', `document.body.innerText.includes("isn't stored here")`);
    say('a link to a session from elsewhere shows the toast');

    // A Finder drag's paths come from the system's drag pasteboard (WKWebView gives a dropped file's name, not its path).
    // The pasteboard is written only while no mouse button is down, so a drag of whoever uses the Mac is never touched.
    // Windows hands a drop's paths over in the drop itself, Linux in the drag's data as WebKitGTK receives it (hand checks)
    const wrote = !MAC ? 'not macOS' : execFileSync('osascript', ['-l', 'JavaScript', '-e', `ObjC.import('AppKit');
      if (Number($.NSEvent.pressedMouseButtons) !== 0) 'busy'; else {
        const pb = $.NSPasteboard.pasteboardWithName('Apple CFPasteboard drag');
        pb.clearContents;
        pb.writeObjects($([$.NSURL.fileURLWithPath(${JSON.stringify(alpha)})]));
        'ok';
      }`], { encoding: 'utf8' }).trim();
    if (wrote === 'ok') {
      const dragged = await page.eval(`window.__TAURI_INTERNALS__.invoke('acpira_drag_paths')`);
      if (JSON.stringify(dragged) !== JSON.stringify([alpha])) fail(`the drag pasteboard read back as ${JSON.stringify(dragged)}`);
      const resolved = await page.eval(`(async () => {
        document.dispatchEvent(Object.assign(new Event('dragenter'), { dataTransfer: { types: ['Files'] } }));
        await new Promise(r => setTimeout(r, 300));
        return window.__acpiraApi.pathForFile(new File([], ${JSON.stringify(basename(alpha))}));
      })()`);
      if (resolved !== alpha) fail(`pathForFile answered ${resolved}`);
      say('a Finder drag\'s folder path reaches the page through the drag pasteboard');
    } else if (MAC) say('note: a mouse button was down, the drag pasteboard check was skipped');

    // The tool panel: closed at first, its toggle clear of the chat header's buttons; the tree, a folder easing open, a
    // file in the viewer, and a browser tab whose frame talks through this page
    const panelShown = `!!document.querySelector('[data-tool-panel-wrap][data-shown]')`;
    if (await page.eval(panelShown)) fail('the tool panel was open in a fresh window');
    const toggleClear = await page.eval(`(() => {
      const t = document.querySelector('[data-panel-toggle] button'), m = document.querySelector('[data-sheet-header] button:last-of-type');
      return !!t && !!m && m.getBoundingClientRect().right <= t.getBoundingClientRect().left;
    })()`);
    if (!toggleClear) fail('the tool panel toggle overlapped the chat header');
    await page.press('the tool panel toggle', `document.querySelector('[data-panel-toggle] button')`);
    const dirRow = `document.querySelector('[data-tree] [data-path="docs"]')`;
    const treeRow = `[...document.querySelectorAll('[data-tree] [role="treeitem"]')].find(e => e.textContent.includes('alpha-notes.md'))`;
    await page.waitFor('the project tree', `!!document.querySelector('[data-tool-panel-wrap][data-shown]:not([data-moving])') && !!(${dirRow}) && !!(${treeRow})`);
    await page.press('docs folder in the tree', dirRow);
    await page.waitFor('docs folder open', `!!document.querySelector('[data-tree] [data-path="docs/guide.md"]')`);
    await page.press('alpha-notes.md in the tree', treeRow);
    await page.waitFor('the file in the viewer', `document.querySelector('[data-viewer]')?.innerText.includes('# alpha-notes.md')`);
    if (shotDir) await page.shot(join(shotDir, 'panel-files.png'));
    // Media in the viewer, loaded from the app's /file route: a picture at its size, a byte range, script-bearing types
    // refused there (an SVG draws from its source instead), a video that loads by range and seeks
    const media = kind => `document.querySelector('[data-viewer] [data-media="${kind}"]')`;
    // A file open in the Files tab puts the tree away; the viewer's bar brings it back
    const pick = async path => {
      const row = `document.querySelector('[data-tree] [data-path="${path}"]')`;
      if (!(await page.eval(`(${row})?.getBoundingClientRect().width > 0`))) await page.press('the tree toggle', `document.querySelector('[data-viewer] button[aria-label="File Tree"]')`);
      await page.press(`${path} in the tree`, row);
    };
    await pick('docs/dot.png');
    await page.waitFor('the picture in the viewer', `(() => { const i = ${media('image')}?.querySelector('img'); return !!i?.complete && i.naturalWidth === 2 && ${media('image')}.innerText.includes('2 × 1'); })()`);
    const ranged = await page.eval(`(async () => {
      const src = ${media('image')}.querySelector('img').src;
      const r = await fetch(src, { headers: { Range: 'bytes=1-4' } });
      const svg = await fetch(src.replace('dot.png', 'logo.svg'));
      return [r.status, r.headers.get('content-range'), (await r.arrayBuffer()).byteLength, svg.status].join(' ');
    })()`);
    if (ranged !== '206 bytes 1-4/70 4 404') fail(`the /file route answered ${ranged}`);
    await pick('docs/logo.svg');
    await page.waitFor('the SVG drawn from its source', `(() => { const i = ${media('image')}?.querySelector('img'); return !!i?.complete && i.src.startsWith('data:image/svg+xml') && i.naturalWidth === 3; })()`);
    // WebKit draws a PDF in a frame with its own plugin: a document of the PDF's type holding one embed. Other engines'
    // viewers are only seen loading
    await pick('docs/page.pdf');
    const pdf = await page.waitFor('the PDF in the viewer', `(() => { const d = ${media('pdf')}?.contentDocument; return d?.readyState === 'complete' && d.URL !== 'about:blank' && d.contentType + ' ' + d.embeds.length; })()`);
    if (MAC && pdf !== 'application/pdf 1') fail(`the PDF frame holds ${pdf}`);
    if (clip) {
      await pick('docs/clip.mp4');
      const video = `${media('video')}?.querySelector('video')`;
      await page.waitFor('the video\'s first frame', `(() => { const v = ${video}; return !!v && v.readyState >= 2 && v.videoWidth === 320 && Math.round(v.duration) === 5; })()`)
        .catch(async e => fail(`${e.message}: ${await page.eval(`(() => { const v = ${video}; return v ? JSON.stringify({ ready: v.readyState, network: v.networkState, error: v.error?.code, message: v.error?.message, duration: v.duration, width: v.videoWidth }) : document.querySelector('[data-viewer]')?.innerText; })()`)}`));
      await page.eval(`(${media('video')}.querySelector('video').currentTime = 4.5, true)`);
      await page.waitFor('the video seeking near its end', `(() => { const v = ${media('video')}.querySelector('video'); return !v.seeking && v.readyState >= 2 && v.currentTime > 4; })()`);
      if (shotDir) await page.shot(join(shotDir, 'panel-video.png'));
      say('the viewer shows a picture, an SVG from its source, a PDF and a video read by range');
    } else say('the viewer shows a picture, an SVG from its source and a PDF (no ffmpeg: the video check was skipped)');
    await page.press('the new tab button', `document.querySelector('[data-panel-tabs] button[aria-label="New tab"]')`);
    const tiles = await page.waitFor('the new tab\'s tiles', `(() => { const t = [...document.querySelectorAll('[data-new-tab] .grid button')].map(b => b.textContent.trim()); return t.length && t.join(','); })()`);
    if (tiles !== 'Files,Terminal,Browser') fail(`the new tab's tiles read ${tiles}`);
    await page.press('the browser tile', `[...document.querySelectorAll('[data-new-tab] button')].find(b => b.textContent.trim() === 'Browser')`);
    await page.waitFor('the browser page in its tab', `(() => { const f = document.querySelector('iframe[src*="host=browser"]'); return !!f?.contentDocument?.querySelector('input'); })()`);
    const site = createServer((_q, r) => { r.writeHead(200, { 'content-type': 'text/html' }); r.end('<title>Smoke site</title><h1>smoke site</h1>'); });
    await new Promise(r => site.listen(0, '127.0.0.1', r));
    try {
      const tabId = await page.eval(`new URLSearchParams(document.querySelector('iframe[src*="host=browser"]').src.split('?')[1]).get('tab')`);
      await page.eval(`window.__acpiraApi.postMessage({ type: 'panelBrowserOpen', tab: ${JSON.stringify(tabId)}, url: 'http://127.0.0.1:${site.address().port}/' })`);
      await page.waitFor('the site in the browser tab', `(() => { const d = document.querySelector('iframe[src*="host=browser"]')?.contentDocument; return !!d?.querySelector('[data-sheet] iframe') && [...document.querySelectorAll('[data-panel-tabs] [role="tab"]')].some(t => t.textContent.includes('Smoke site')); })()`);
      if (shotDir) { await sleep(500); await page.shot(join(shotDir, 'panel-browser.png')); }
      // The recent pages came back through the shell, with the site answering
      const history = JSON.parse(readFileSync(join(user, 'browser-history.json'), 'utf8'));
      if (!history.some(p => p.title === 'Smoke site')) fail(`the browser history did not record the site: ${JSON.stringify(history)}`);
    } finally {
      site.close();
    }
    say('the tool panel opens on the tree, shows a file, and runs a browser tab through the preview proxy');

    // Terminal is a login shell the engine runs in the project folder, drawn by xterm in the tab: it takes the focus, a
    // typed line runs there, the tab takes the shell's name, and closing the tab ends the shell. The line goes in as the
    // input event xterm reads from its helper textarea (no DevTools protocol here to type keys)
    await page.press('the new tab button', `document.querySelector('[data-panel-tabs] button[aria-label="New tab"]')`);
    await page.press('the terminal tile', `[...document.querySelectorAll('[data-new-tab] button')].find(b => b.textContent.trim() === 'Terminal')`);
    await page.waitFor('the terminal taking the focus', `!!document.activeElement?.closest('[data-terminal] .xterm')`);
    // PowerShell on Windows: its own pid, no newline (named parameters: 5.1 binds the positional ones the other way)
    const line = WIN ? 'Set-Content -Path terminal.txt -Value $PID -NoNewline\\r' : 'printf %s $$ > terminal.txt\\r';
    await page.eval(`(() => {
      const t = document.activeElement;
      t.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'insertText', data: '${line}' }));
      return true;
    })()`);
    const marker = join(alpha, 'terminal.txt');
    await waitUntil('the typed line to run in the project folder', () => existsSync(marker) && /^\d+$/.test(readFileSync(marker, 'utf8')), 15_000);
    const shellPid = Number(readFileSync(marker, 'utf8'));
    const terminalTab = `[...document.querySelectorAll('[data-panel-tabs] [role="tab"]')].find(t => /^(zsh|bash|sh|fish|powershell|pwsh|cmd)$/i.test(t.textContent.trim()))`;
    await page.waitFor('the terminal tab named after its shell', `!!(${terminalTab})`);
    if (shotDir) { await sleep(300); await page.shot(join(shotDir, 'panel-terminal.png')); }
    await page.press('the terminal tab\'s close button', `(${terminalTab})?.querySelector('[aria-label="Close tab"]')`);
    await waitUntil('the shell to end with its tab', () => !alive(shellPid), 5_000);
    say('a terminal tab runs a typed line in the project and its shell ends with the tab');

    // A plugin's tab piece: enabled as the plugin page does, it joins the new tab's tiles after the built-in ones and runs
    // in a sandboxed frame that reaches its own process, hears what the process posts, gets the page's tokens and has
    // neither the page nor the network (the frame reports what it saw through a method of its plugin)
    const plugin = join(data, 'plugins', 'smoke-board');
    const report = join(root, 'frame-report.json');
    mkdirSync(plugin, { recursive: true });
    writeFileSync(join(plugin, 'board.mjs'), `
import { createInterface } from 'node:readline';
import { writeFileSync } from 'node:fs';
const send = m => process.stdout.write(JSON.stringify(m) + '\\n');
createInterface({ input: process.stdin }).on('close', () => process.exit(0)).on('line', line => {
  const m = JSON.parse(line);
  if (m.method === 'initialize') return send({ jsonrpc: '2.0', id: m.id, result: { protocolVersion: 1 } });
  if (m.method === 'rpc/board.hello') {
    send({ jsonrpc: '2.0', id: 'post', method: 'ui/post', params: { message: { pushed: true } } });
    return send({ jsonrpc: '2.0', id: m.id, result: { hello: 'board' } });
  }
  if (m.method === 'rpc/board.report') { writeFileSync(${JSON.stringify(report)}, JSON.stringify(m.params.params)); return send({ jsonrpc: '2.0', id: m.id, result: {} }); }
  if (m.id !== undefined && m.method) send({ jsonrpc: '2.0', id: m.id, error: { code: -32601, message: 'no ' + m.method } });
});
`);
    writeFileSync(join(plugin, 'board.html'), `<!doctype html><p>Smoke board</p><script>
const seen = {};
acpira.on(m => { seen.pushed = !!(m && m.pushed); });
document.addEventListener('securitypolicyviolation', () => { seen.blocked = true; });
(async () => {
  try { void parent.document.body; seen.page = 'reachable'; } catch { seen.page = 'opaque'; }
  seen.hello = (await acpira.call('board.hello')).hello;
  await fetch('http://127.0.0.1:9/').then(() => { seen.network = 'open'; }, () => { seen.network = 'closed'; });
  seen.fg = getComputedStyle(document.documentElement).getPropertyValue('--fg-1').trim();
  await new Promise(r => setTimeout(r, 300));
  await acpira.call('board.report', seen);
})();
</script>`);
    writeFileSync(join(plugin, 'plugin.json'), JSON.stringify({
      schema: 1, id: 'smoke-board', version: '1.0.0', description: 'Smoke board',
      contributes: {
        // A command is a file of the plugin or one on PATH, never an absolute path
        runtime: { command: ['node', 'board.mjs'], methods: ['board.hello', 'board.report'] },
        ui: [{ id: 'board', slot: 'panel.tab', title: 'Board', html: 'board.html' }],
      },
    }));
    await page.eval(`window.__acpiraApi.postMessage({ type: 'pluginAction', action: { op: 'enable', id: 'smoke-board' } })`);
    await page.press('the new tab button', `document.querySelector('[data-panel-tabs] button[aria-label="New tab"]')`);
    const withBoard = await page.waitFor('the plugin\'s tile', `(() => { const t = [...document.querySelectorAll('[data-new-tab] .grid button')].map(b => b.textContent.trim()); return t.includes('Board') && t.join(','); })()`);
    if (withBoard !== 'Files,Terminal,Browser,Board') fail(`the new tab's tiles read ${withBoard} with the plugin on`);
    await page.press('the plugin\'s tile', `[...document.querySelectorAll('[data-new-tab] button')].find(b => b.textContent.trim() === 'Board')`);
    await page.waitFor('the plugin\'s tab', `[...document.querySelectorAll('[data-panel-tabs] [role="tab"]')].some(t => t.textContent.trim() === 'Board') && !!document.querySelector('iframe[src$="/plugin-frame.html"][sandbox="allow-scripts"]')`);
    await waitUntil('the frame\'s report', () => existsSync(report), 15_000);
    const seen = JSON.parse(readFileSync(report, 'utf8'));
    if (seen.hello !== 'board' || !seen.pushed || seen.page !== 'opaque' || seen.network !== 'closed' || !seen.blocked || !seen.fg) fail(`the plugin frame saw ${JSON.stringify(seen)}`);
    if (shotDir) { await sleep(300); await page.shot(join(shotDir, 'panel-plugin.png')); }
    await page.eval(`window.__acpiraApi.postMessage({ type: 'pluginAction', action: { op: 'disable', id: 'smoke-board' } })`);
    await page.waitFor('the plugin\'s tab gone', `![...document.querySelectorAll('[data-panel-tabs] [role="tab"]')].some(t => t.textContent.trim() === 'Board') && !document.querySelector('iframe[src$="/plugin-frame.html"]')`);
    say('a plugin\'s tab piece joins the new tab, runs sandboxed against its own process, and goes with its plugin');
    await page.press('the tool panel toggle', `document.querySelector('[data-panel-toggle] button')`);
    await page.waitFor('the tool panel closed', `!${panelShown}`);

    // A skin from the scaffold (a deep-sea backdrop and the cat on the composer): enabled as the plugin page does, its
    // tokens and scheme take the page, its backdrop runs behind the shell in the frame document, the cat sits on the
    // composer; switched off, all of it goes
    execFileSync(agentsBin, ['plugin', 'new', 'smoke-skin', '--kind', 'theme,ui', '--scene', 'deepsea', '--home', data], { stdio: 'ignore' });
    // The same piece in the toolbar, the header and the column's foot, each at the box its slot asks for
    const skinManifest = join(data, 'plugins', 'smoke-skin', 'plugin.json');
    const manifest = JSON.parse(readFileSync(skinManifest, 'utf8'));
    manifest.contributes.ui.push(
      { id: 'chip', slot: 'composer.toolbar', title: 'Chip', html: 'cat.html', width: 60, height: 20 },
      { id: 'status', slot: 'statusbar', title: 'Status', html: 'cat.html', width: 40, height: 20 },
      { id: 'foot', slot: 'sidebar', title: 'Foot', html: 'cat.html', height: 40 },
    );
    writeFileSync(skinManifest, JSON.stringify(manifest, null, 2));
    await page.eval(`window.__acpiraApi.postMessage({ type: 'pluginAction', action: { op: 'enable', id: 'smoke-skin' } })`);
    const skinned = `document.querySelector('.acp-shell[data-skin="smoke-skin"][data-backdrop][data-theme="dark"]')`;
    const slotFrames = `['composer.toolbar', 'statusbar', 'sidebar'].every(s => !!document.querySelector('[data-plugin-slot="' + s + '"] iframe'))`;
    await page.waitFor('the skin', `!!${skinned} && !!document.querySelector('[data-skin-backdrop] iframe[src$="/plugin-frame.html"]') && !!document.querySelector('[data-plugin-above] iframe') && ${slotFrames} && /#031420/.test(document.getElementById('acpira-plugin-theme')?.textContent ?? '')`);
    if (shotDir) { await sleep(800); await page.shot(join(shotDir, 'skin.png')); }
    // The skin picker's action: none keeps the plugin's pieces and drops its look, picking it again brings it back
    await page.eval(`window.__acpiraApi.postMessage({ type: 'pluginAction', action: { op: 'skin' } })`);
    await page.waitFor('no skin', `!document.querySelector('.acp-shell[data-skin]') && !document.querySelector('[data-skin-backdrop]') && ${slotFrames}`);
    await page.eval(`window.__acpiraApi.postMessage({ type: 'pluginAction', action: { op: 'skin', id: 'smoke-skin' } })`);
    await page.waitFor('the skin picked again', `!!${skinned}`);
    await page.eval(`window.__acpiraApi.postMessage({ type: 'pluginAction', action: { op: 'disable', id: 'smoke-skin' } })`);
    await page.waitFor('the skin gone', `!document.querySelector('.acp-shell[data-skin]') && !document.querySelector('[data-skin-backdrop], [data-plugin-above], [data-plugin-slot]') && !document.getElementById('acpira-plugin-theme')`);
    say('a skin takes the page with its backdrop and pieces in four slots, can be set aside and picked again, and goes with its plugin');

    // A language change re-renders in place, and the menu bar follows the shared settings file
    await page.eval(`(window.__smokeNodes = [document.querySelector('[data-session-panel]'), document.querySelector('[data-session][aria-selected="true"]')]).every(Boolean)`);
    const english = await page.call({ op: 'menuBar' });
    if (!english.includes('File') || !english.includes('Edit')) fail(`the menu bar read ${JSON.stringify(english)} in English`);
    await page.eval(`window.__acpiraApi.postMessage({ type: 'setSetting', key: 'language', value: 'zh-CN' })`);
    await page.waitFor('the page in Chinese', `document.body.innerText.includes('新会话')`);
    if (!(await page.eval(`window.__smokeNodes.every(n => n.isConnected)`))) fail('a language change remounted the page');
    // The native menu bar follows (rebuilt from the shared settings file within its one-second poll)
    let chinese = [];
    await waitUntil('the menu bar in Chinese', async () => {
      chinese = await page.call({ op: 'menuBar' });
      return chinese.length === english.length && chinese.slice(1).every(t => /[\u4e00-\u9fff]/.test(t));
    }, 5_000).catch(() => fail(`the menu bar read ${JSON.stringify(chinese)} after the switch to zh-CN`));
    await page.eval(`window.__acpiraApi.postMessage({ type: 'setSetting', key: 'language', value: 'en' })`);
    await page.waitFor('the page in English again', `document.body.innerText.includes('New session')`);
    await waitUntil('the menu bar in English again', async () => JSON.stringify(await page.call({ op: 'menuBar' })) === JSON.stringify(english), 5_000);
    say(`a language change re-renders in place and the menu bar follows (${chinese.slice(1).join(' ')})`);

    // The View menu's layout item goes to the page as a shell command; files first lays out explorer, stage, chat, column
    const filesLayout = `!!document.querySelector('.acp-shell[data-layout="files"]')`;
    await page.call({ op: 'menu', item: 'layout.files' });
    await page.waitFor('the files layout', `${filesLayout} && !!document.querySelector('[data-explorer] [data-tree]')`);
    const columns = await page.eval(`['[data-explorer]', '[data-stage]', '[data-sheet]', '[data-session-panel]'].map(s => Math.round(document.querySelector(s)?.getBoundingClientRect().left ?? -1))`);
    if (columns.some((x, i) => x < 0 || (i > 0 && x <= columns[i - 1]))) fail(`the files layout's columns are out of order: ${columns.join(' ')}`);
    if (shotDir) await page.shot(join(shotDir, 'layout-files.png'));
    await page.call({ op: 'menu', item: 'layout.chat' });
    await page.waitFor('the conversation layout again', `!${filesLayout} && !${panelShown}`);
    say(`View › Layout switches to files first and back (x ${columns.join(' / ')})`);

    // A narrow window, really resized: the list becomes a page over the window, a pick closes it on that session
    await page.call({ op: 'resize', width: 480, height: 760 });
    await page.waitFor('the narrow layout', `!!document.querySelector('.acp-shell[data-narrow] [data-header-sub]')`);
    const toggle = `document.querySelector('[data-title-row] [data-session-toggle]')`;
    const pageOpen = `(() => { const a = document.querySelector('[data-session-panel]'); return !!document.querySelector('.acp-shell[data-drawer-open]') && getComputedStyle(a).transform === 'none' && Math.round(a.getBoundingClientRect().width) === innerWidth; })()`;
    const pageClosed = `!document.querySelector('.acp-shell[data-drawer-open]') && getComputedStyle(document.querySelector('[data-session-panel]')).visibility === 'hidden'`;
    const shown = id => `document.querySelector('[data-session][aria-selected="true"]')?.getAttribute('data-session') === ${JSON.stringify(id)}`;
    await page.press('the list toggle', toggle);
    await page.waitFor('the list page over the window', pageOpen);
    await page.press('session B\'s row', `document.querySelector('[data-session="${b}"]')`);
    await page.waitFor('the list page closed on session B', `${pageClosed} && ${shown(b)}`);
    if (shotDir) await page.shot(join(shotDir, 'narrow.png'));
    await page.call({ op: 'resize', width: 1180, height: 820 });
    await page.waitFor('the docked column back', `!document.querySelector('.acp-shell[data-narrow]')`);
    say('a narrow window shows the list as a page; a pick closes it; widened, the column docks again');

    // "Remove from list" on a project's right-click menu takes it off the column although its session stays
    await waitUntil('session B\'s record', () => existsSync(join(data, 'sessions', `${b}.json`)), 10_000);
    const betaRow = `[...document.querySelectorAll('[data-project-row]')].find(e => e.title.endsWith('smoke-beta'))`;
    await page.press('smoke-beta\'s project row', betaRow, 2);
    await page.press('the remove item', `[...document.querySelectorAll('[role="menuitem"]')].find(e => e.textContent.includes('Remove from list'))`);
    await page.waitFor('smoke-beta to leave the column', `!${betaRow} && !!document.querySelector('[data-project-row]')`);
    if (!existsSync(join(data, 'sessions', `${b}.json`))) fail('removing a project deleted its session');
    say('a removed project leaves the column, its session stays');

    // Windows and Linux: no system title bar and no menu bar; the page draws the caption (maximize turns into restore and
    // back, its label with the glyph) and the menu, whose items go back to the shell as menuCommand
    if (!MAC) {
      const max = `document.querySelector('[data-window-caption] [data-op="maximize"]')`;
      await page.waitFor('the caption', `document.querySelectorAll('[data-window-caption] button').length === 3`);
      const label = await page.eval(`${max}.title`);
      await page.press('maximize', max);
      await page.waitFor('the restore button', `${max}.title !== ${JSON.stringify(label)}`);
      if (shotDir) { await sleep(300); await page.shot(join(shotDir, 'maximized.png')); }
      await page.press('restore', max);
      await page.waitFor('the maximize button again', `${max}.title === ${JSON.stringify(label)}`);
      const menus = `['File', 'Edit', 'View', 'Help'].every(l => [...document.querySelectorAll('[role="menuitem"]')].some(e => e.textContent.trim() === l))`;
      await page.press('the menu button', `document.querySelector('[data-app-menus]')`);
      await page.waitFor('the four menus', menus);
      await page.press('the menu button again', `document.querySelector('[data-app-menus]')`);
      await page.waitFor('the menu closed', `!document.querySelector('[role="menuitem"]')`);
      say('the page draws the caption (maximize, restore) and the menu (File, Edit, View, Help)');
    }

    // Pick real list options, then check them again after quitting the app: sessionStorage alone loses them.
    const viewButton = `document.querySelector('[data-view-menu]')`;
    const viewItem = text => `[...document.querySelectorAll('[role="menuitem"]')].find(e => e.textContent.trim() === ${JSON.stringify(text)})`;
    await page.press('the list view menu', viewButton);
    await page.waitFor('the current list count', `!!(${viewItem('ShowLatest 5')})`);
    await page.press('the pinned placement submenu', viewItem('PinnedAt the top'));
    await page.press('pins inside their project', viewItem('In their project'));
    await page.press('close the pinned placement menu', viewButton);
    await page.press('reopen the list view menu', viewButton);
    await page.press('the show submenu', viewItem('ShowLatest 5'));
    if (await page.eval(`document.body.innerText.includes('Per project')`)) fail('the show submenu still has the redundant per-project label');
    await page.press('the latest ten option', viewItem('Latest 10'));
    await page.press('the model name option', viewItem('Model at the row end'));
    await page.waitFor('the updated list count', `!!(${viewItem('ShowLatest 10')})`);
    const listView = await page.eval(`window.__acpiraApi.getState().sessionListView`);
    if (listView.pinned !== 'project' || listView.per !== 10 || !listView.model) fail(`the list options did not apply: ${JSON.stringify(listView)}`);
    await page.press('close the list view menu', viewButton);
    say('list menu shows its current count and applies pin placement, count and model display');

    // A second window from the menu bar (the page's menu on Windows and Linux), then closed: the window list follows
    if (!MAC) await page.eval(`window.__acpiraApi.postMessage({ type: 'menuCommand', id: 'NewWindow' })`);
    else await page.call({ op: 'menu', item: 'NewWindow' });
    let windows = [];
    await waitUntil('two windows, both shown', async () => {
      windows = await page.call({ op: 'windows' });
      return windows.length === 2 && windows.every(w => w.ready && w.visible);
    }, 15_000).catch(e => fail(`${e.message}: ${JSON.stringify(windows)}`));
    await page.call({ op: 'close', window: windows[1].label });
    await waitUntil('the second window to close', async () => (await page.call({ op: 'windows' })).length === 1, 10_000);
    say('File › New Window opens a second chat window, and it closes');

    // Quit through the hook (the Quit item's path): window places are saved, the engine is only disconnected and ends
    // after its idle grace, taking the agent processes with it
    const enginePid = Number(/sidecar ready: pid (\d+)/.exec(appLog)?.[1]);
    if (!enginePid) fail('the app log names no engine pid');
    await page.call({ op: 'quit' }).catch(() => {});
    page.close();
    page = undefined;
    await within('the app to quit', appExit, 15_000);
    app = undefined;
    const saved = JSON.parse(readFileSync(join(user, 'windows.json'), 'utf8'));
    if (saved.windows?.length !== 1 || !saved.windows[0].bounds) fail(`windows.json kept no window: ${JSON.stringify(saved)}`);
    say(`quit saved ${saved.windows.length} window place(s)`);

    // A relaunch within the engine's idle grace: the saved window comes back on its session, on the same engine process
    const before = appLog.length;
    if (!WIN) rmSync(hookPath, { force: true });
    app = launch([]);
    app.stdout.on('data', collect);
    app.stderr.on('data', collect);
    const relaunchExit = new Promise(r => app.once('exit', r));
    page = await Hook.connect(hookPath);
    // By the window's session, not a selected row: the removed project's session has no row in the column
    await waitUntil('the restored window on its session', async () => (await page.call({ op: 'windows' }).catch(() => []))[0]?.session === saved.windows[0].session, 20_000);
    await page.waitFor('its conversation', `document.body.innerText.includes('hello world')`);
    const again = Number(/sidecar ready: pid (\d+)/.exec(appLog.slice(before))?.[1]);
    if (again !== enginePid) fail(`the relaunch reached engine ${again}, not the running ${enginePid}`);
    say('a relaunch restores the window on its session and reuses the running engine');
    const restoredView = await page.eval(`window.__acpiraApi.getState().sessionListView`);
    if (JSON.stringify(restoredView) !== JSON.stringify(listView)) fail(`relaunch lost list options: ${JSON.stringify(restoredView)}`);
    await page.press('the restored list view menu', viewButton);
    await page.waitFor('the restored list choices', `!!(${viewItem('ShowLatest 10')}) && !!(${viewItem('PinnedIn their project')})`);
    await page.press('close the restored list view menu', viewButton);
    say('a full app relaunch preserves list preferences and displays the restored choices');
    const engines = [enginePid];
    if (release) {
      // The app updates itself: checked, the delta downloaded and applied, the staged bundle verified, then the restart
      // toast's button swaps it in and a new process starts from the replaced bundle on the same session
      const session = saved.windows[0].session;
      await page.eval(`window.__acpiraApi.postMessage({ type: 'updateCheck', download: true })`);
      let state;
      await waitUntil('the update to be staged', async () => {
        state = await page.call({ op: 'update' }).catch(() => undefined);
        if (state?.phase === 'failed') fail(`the update failed: ${state.error}`);
        return state?.phase === 'ready';
      }, 120_000).catch(e => fail(`${e.message}: ${JSON.stringify(state)}`));
      if (state.version !== release.version) fail(`staged ${state.version}, expected ${release.version}`);
      if (/did not apply/.test(appLog)) fail('the delta did not apply; the full package was taken');
      const restart = `[...document.querySelectorAll('[role="status"] button')].find(b => b.textContent.trim() === 'Restart')`;
      await page.waitFor('the restart toast', `document.body.innerText.includes('Acpira ${release.version} is ready to install') && !!(${restart})`);
      if (shotDir) await page.shot(join(shotDir, 'update-ready.png'));
      const logBefore = existsSync(desktopLog) ? readFileSync(desktopLog, 'utf8').length : 0;
      const oldPid = app.pid;
      await page.press('the restart button', restart);
      await within('the old process to quit for the restart', relaunchExit, 15_000);
      app = undefined;
      page.close();
      if (!WIN) rmSync(hookPath, { force: true });
      if (!release.replaced()) fail(`the installed ${release.kind} was not replaced`);
      page = await Hook.connect(hookPath);
      await waitUntil('the updated app on the same session', async () => (await page.call({ op: 'windows' }).catch(() => []))[0]?.session === session, 30_000);
      await page.waitFor('its conversation after the update', `document.body.innerText.includes('hello world')`);
      // Windows quotes the program in the command line it records
      const updatedPid = processes().find(p => p.command.replace(/^"/, '').startsWith(bin) && !/ (serve|stdio)( |$)/.test(p.command))?.pid;
      if (!updatedPid || updatedPid === oldPid) fail(`no new process runs ${bin}`);
      const fresh = readFileSync(desktopLog, 'utf8').slice(logBefore);
      const newEngine = Number(/sidecar ready: pid (\d+)/.exec(fresh)?.[1]);
      if (newEngine) engines.push(newEngine);
      say(`updated to ${release.version} through the delta (${release.deltaKb} KB of ${release.fullKb} KB): process ${oldPid} → ${updatedPid} on session ${session}, engine ${newEngine || '?'}`);
      if (release.kind === 'exe') {
        // The old exe went aside; the engine it started may still run from it, so it is deleted on a later start
        say(`the previous exe ${existsSync(`${bin}.old`) ? 'waits beside it as Acpira.exe.old' : 'is already gone'}`);
      }
      await page.call({ op: 'quit' }).catch(() => {});
      page.close();
      page = undefined;
      await waitUntil('the updated app to quit', () => !alive(updatedPid), 15_000);
    } else {
      await page.call({ op: 'quit' }).catch(() => {});
      page.close();
      page = undefined;
      await within('the relaunched app to quit', relaunchExit, 15_000);
      app = undefined;
    }
    for (const pid of engines) await waitUntil(`engine ${pid} to end after its idle grace`, () => !alive(pid), 90_000);
    const left = processes().filter(p => p.command.includes(fakeAgent));
    if (left.length) fail(`agent processes left after the engine ended:\n${left.map(p => `${p.pid} ${p.ppid} ${p.command}`).join('\n')}`);
    say(`engine${engines.length > 1 ? "s" : ""} ${engines.join(" and ")} ended, no agent process left`);
    passed = true;
    say('passed');
  } catch (e) {
    console.error(`smoke: FAILED: ${e instanceof SmokeError ? e.message : e?.stack ?? e}`);
    if (page) {
      try { await page.shot(join(root, 'failure.png')); console.error(`smoke: screenshot ${join(root, 'failure.png')}`); } catch { /* the page may be gone */ }
    }
    const tail = appLog.trim().split('\n').slice(-25).join('\n');
    if (tail) console.error(`smoke: app log tail (${logFile}):\n${tail}`);
    process.exitCode = 1;
  } finally {
    page?.close();
    if (app) { app.kill('SIGTERM'); await Promise.race([new Promise(r => app.once('exit', r)), sleep(5000)]); }
    if (passed && !keep) {
      // A confirmed plugin's frozen copy is read-only, folders included
      writable(join(data, 'plugins'));
      rmSync(root, { recursive: true, force: true });
    }
    else say(`kept ${root}${passed ? '' : ' (its engine ends by itself within its 30 s idle grace)'}`);
  }
}

// The next version of the bundle at `appPath`, served from the loopback: the run's copy of the bundle (the updater
// replaces it), a re-signed copy carrying a marker file as the new version, and the release packed from them with the
// pack tool the release workflow uses (a throwaway key; zstd level 3 to keep the run short)
async function prepareUpdate(appPath) {
  const installed = join(root, 'Applications', 'Acpira.app');
  const next = join(root, 'next', 'Acpira.app');
  for (const d of [dirname(installed), dirname(next)]) mkdirSync(d, { recursive: true });
  execFileSync('ditto', [appPath, installed]);
  execFileSync('ditto', [appPath, next]);
  const current = execFileSync('plutil', ['-extract', 'CFBundleShortVersionString', 'raw', join(appPath, 'Contents', 'Info.plist')], { encoding: 'utf8' }).trim();
  const version = nextPatch(current);
  writeFileSync(join(next, 'Contents', 'Resources', 'smoke-update.txt'), `${version}\n`);
  execFileSync('codesign', ['--force', '--deep', '--sign', '-', next], { stdio: 'ignore' });
  const marker = join(installed, 'Contents', 'Resources', 'smoke-update.txt');
  const served = await serveRelease(packTool(), { installed, next, current, version, platform: 'darwin-aarch64', kind: 'app' });
  return { ...served, kind: 'app', replaced: () => existsSync(marker) };
}

// The same for Windows' single exe: the run's copy (under the temp folder, which a smoke build does not refuse; see
// smoke.rs update_config) and a copy with a marker appended as the next version; the exe carries no signature yet
async function prepareExeUpdate(exePath) {
  const pack = packTool();
  if (!pack) {
    say('no --pack-tool and no rust/ checkout to build one: the update step is skipped');
    return undefined;
  }
  const installed = join(root, 'Programs', 'Acpira.exe');
  const next = join(root, 'next', 'Acpira.exe');
  for (const d of [dirname(installed), dirname(next)]) mkdirSync(d, { recursive: true });
  copyFileSync(exePath, installed);
  // The engine role answers --version with the release's version, the shell's too
  const current = execFileSync(exePath, ['--version'], { encoding: 'utf8' }).trim();
  const version = nextPatch(current);
  writeFileSync(next, Buffer.concat([readFileSync(exePath), Buffer.from(`\nacpira-smoke-update ${version}\n`)]));
  const want = sha256(next);
  const served = await serveRelease(pack, { installed, next, current, version, platform: 'windows-x86_64', kind: 'exe' });
  return { ...served, kind: 'exe', replaced: () => sha256(installed) === want };
}

function nextPatch(version) {
  const parts = version.split('.').map(Number);
  parts[parts.length - 1] += 1;
  return parts.join('.');
}

function sha256(file) {
  return createHash('sha256').update(readFileSync(file)).digest('hex');
}

// The update pack tool: --pack-tool, else the debug build from this checkout's rust/ (none without one)
function packTool() {
  if (packToolArg) return packToolArg;
  if (!existsSync(join(REPO, 'rust', 'Cargo.toml'))) return undefined;
  execFileSync('cargo', ['build', '--quiet', '-p', 'acpira-desktop', '--bin', 'acpira-update-pack'], { cwd: join(REPO, 'rust'), stdio: 'inherit' });
  return join(REPO, 'rust', 'target', 'debug', WIN ? 'acpira-update-pack.exe' : 'acpira-update-pack');
}

// Packs `installed` as version `current` and `next` as `version` (full package and a delta from `current`), signs the
// manifest with a throwaway key and serves it all on the loopback
async function serveRelease(pack, { installed, next, current, version, platform, kind }) {
  const keys = spawnSync(pack, ['keygen'], { encoding: 'utf8' });
  const seed = keys.stdout.trim();
  const publicKey = /([0-9a-f]{64})/.exec(keys.stderr)?.[1];
  if (!seed || !publicKey) fail(`keygen printed no key: ${keys.stderr}`);
  const files = join(root, 'release');
  const server = createServer((q, r) => {
    const file = join(files, 'out', basename(decodeURIComponent(q.url.split('?')[0])));
    if (!existsSync(file)) { r.writeHead(404); r.end(); return; }
    r.writeHead(200, { 'content-length': statSync(file).size });
    r.end(readFileSync(file));
  });
  await new Promise(r => server.listen(0, '127.0.0.1', r));
  server.unref();
  const base = `http://127.0.0.1:${server.address().port}`;
  const common = ['--base-url', base, '--level', '3', '--platform', platform];
  execFileSync(pack, ['pack', '--app', installed, '--version', current, '--out', join(files, 'base'), ...common, '--unsigned'], { stdio: 'ignore' });
  const packed = execFileSync(pack, ['pack', '--app', next, '--version', version, '--out', join(files, 'out'), ...common,
    '--prev', `${current}=${join(files, 'base', `Acpira-${current}-${platform}.${kind}.tar.zst`)}`], { env: { ...process.env, DESKTOP_UPDATE_KEY: seed }, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] });
  const manifest = JSON.parse(readFileSync(join(files, 'out', 'desktop-update.json'), 'utf8')).platforms[platform];
  const delta = manifest.deltas[current];
  if (!delta) fail(`the pack tool listed no delta from ${current}: ${packed}`);
  say(`release ${version} packed: full ${Math.round(manifest.full.size / 1024)} KB, delta ${Math.round(delta.size / 1024)} KB, served on ${base}`);
  return { installed, version, publicKey, manifest: `${base}/desktop-update.json`, deltaKb: Math.round(delta.size / 1024), fullKb: Math.round(manifest.full.size / 1024) };
}

function records() {
  const dir = join(data, 'sessions');
  return existsSync(dir) ? readdirSync(dir).filter(f => /^[0-9a-f-]{36}\.json$/.test(f)) : [];
}

async function roundTrip(text) {
  await page.waitFor('an enabled composer', `!!document.querySelector('[data-start-dock] textarea:not([disabled])')`);
  await page.type('[data-start-dock] textarea', text);
  await page.key('[data-start-dock] textarea', 'Enter');
  await page.waitFor(`the reply to "${text}"`, `document.body.innerText.includes(${JSON.stringify(text)}) && document.body.innerText.includes('hello world')`);
  await page.waitFor('the turn to end', `!!document.querySelector('[data-start-dock] button[aria-label="Send"]')`);
  return page.waitFor('the session row', `document.querySelector('[data-session][aria-selected="true"]')?.getAttribute('data-session')`);
}

async function mentions() {
  const query = '@notes';
  await page.waitFor('an enabled composer', `!!document.querySelector('[data-start-dock] textarea:not([disabled])')`);
  await page.type('[data-start-dock] textarea', query);
  const names = await page.waitFor('file suggestions', `(() => {
    const names = [...document.querySelectorAll('[role=listbox]:not([aria-label]) [role=option]')].map(o => o.innerText.trim());
    return names.some(n => n.includes('notes.md')) ? names : undefined;
  })()`);
  await page.type('[data-start-dock] textarea', '', true);
  await page.waitFor('an empty composer', `document.querySelector('[data-start-dock] textarea').value === ''`);
  return names;
}

function writable(dir) {
  if (!existsSync(dir)) return;
  chmodSync(dir, 0o755);
  for (const e of readdirSync(dir, { withFileTypes: true })) {
    const p = join(dir, e.name);
    if (e.isDirectory()) writable(p);
    else if (e.isFile()) chmodSync(p, 0o644);
  }
}

async function within(what, promise, timeout) {
  let timer;
  const late = new Promise((_, reject) => { timer = setTimeout(() => reject(new SmokeError(`timed out waiting for ${what}`)), timeout); });
  try { return await Promise.race([promise, late]); } finally { clearTimeout(timer); }
}

async function waitUntil(what, check, timeout) {
  const end = Date.now() + timeout;
  while (!(await check())) {
    if (Date.now() > end) fail(`timed out waiting for ${what}`);
    await sleep(250);
  }
}

function alive(pid) {
  try { process.kill(pid, 0); return true; } catch (e) { return e.code === 'EPERM'; }
}

function processes() {
  if (WIN) {
    const raw = execFileSync('powershell.exe', ['-NoProfile', '-Command', 'Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId,CommandLine | ConvertTo-Json -Compress'], { encoding: 'utf8', maxBuffer: 64 << 20 });
    return JSON.parse(raw).map(p => ({ pid: p.ProcessId, ppid: p.ParentProcessId, command: p.CommandLine ?? '' }));
  }
  return execFileSync('ps', ['-eo', 'pid,ppid,command'], { encoding: 'utf8' }).split('\n').slice(1).map(line => {
    const m = /^\s*(\d+)\s+(\d+)\s+(.*)$/.exec(line);
    return m ? { pid: Number(m[1]), ppid: Number(m[2]), command: m[3] } : undefined;
  }).filter(Boolean);
}

// The app's smoke hook: one JSON command per line, one answer per command
class Hook {
  static async connect(path) {
    // A socket file shows up when the app listens; a pipe is only known by connecting to it
    if (!WIN) await waitUntil('the smoke hook', () => existsSync(path), 30_000);
    const end = Date.now() + 30_000;
    for (;;) {
      const sock = connect(path);
      const ok = await new Promise(res => { sock.once('connect', () => res(true)); sock.once('error', () => res(false)); });
      if (ok) return new Hook(sock);
      if (Date.now() > end) fail('timed out waiting for the smoke hook');
      await sleep(200);
    }
  }

  constructor(sock) {
    this.sock = sock;
    this.seq = 0;
    this.pending = new Map();
    let buf = '';
    sock.on('data', d => {
      buf += d.toString();
      for (let i = buf.indexOf('\n'); i >= 0; i = buf.indexOf('\n')) {
        const line = buf.slice(0, i);
        buf = buf.slice(i + 1);
        const m = JSON.parse(line);
        const done = this.pending.get(m.id);
        this.pending.delete(m.id);
        done?.(m);
      }
    });
    sock.on('close', () => {
      for (const done of this.pending.values()) done({ ok: false, value: 'the smoke hook closed (did the app go away?)' });
      this.pending.clear();
    });
  }

  call(cmd) {
    return new Promise((res, rej) => {
      const id = ++this.seq;
      this.pending.set(id, m => (m.ok ? res(m.value) : rej(new Error(`${cmd.op}: ${m.value}`))));
      this.sock.write(`${JSON.stringify({ id, ...cmd })}\n`);
    });
  }

  eval(js) {
    return this.call({ op: 'eval', js });
  }

  async waitFor(what, expression, timeout = 20_000) {
    const end = Date.now() + timeout;
    for (;;) {
      const v = await this.eval(expression).catch(() => undefined);
      if (v) return v;
      if (Date.now() > end) fail(`timed out waiting for ${what}`);
      await sleep(100);
    }
  }

  async click(what, element) {
    await this.waitFor(what, `!!(${element})`);
    await this.eval(`(${element}).click()`);
  }

  // A press at the element's centre as pointer and mouse events (Base UI opens menus on pointer events, which
  // element.click() does not send); the point has to land on the element itself
  async press(what, element, button = 0) {
    const at = await this.waitFor(what, `(() => { const e = ${element}; if (!e) return; const r = e.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })()`);
    const hit = await this.eval(`(() => { const e = ${element}; const h = document.elementFromPoint(${at.x}, ${at.y}); return h && e.contains(h) ? '' : h ? 'covered by ' + h.outerHTML.slice(0, 160) : 'off the page'; })()`);
    if (hit) fail(`${what} at ${Math.round(at.x)},${Math.round(at.y)} is ${hit}`);
    await this.eval(`(() => {
      const t = document.elementFromPoint(${at.x}, ${at.y});
      const o = { bubbles: true, cancelable: true, composed: true, clientX: ${at.x}, clientY: ${at.y}, button: ${button}, buttons: ${button === 2 ? 2 : 1}, pointerId: 1, pointerType: 'mouse', isPrimary: true, view: window };
      t.dispatchEvent(new PointerEvent('pointerover', o)); t.dispatchEvent(new PointerEvent('pointerenter', o)); t.dispatchEvent(new MouseEvent('mouseover', o));
      t.dispatchEvent(new PointerEvent('pointermove', o)); t.dispatchEvent(new MouseEvent('mousemove', o));
      t.dispatchEvent(new PointerEvent('pointerdown', o)); t.dispatchEvent(new MouseEvent('mousedown', o));
      if (t.focus) t.focus();
      const u = { ...o, buttons: 0 };
      t.dispatchEvent(new PointerEvent('pointerup', u)); t.dispatchEvent(new MouseEvent('mouseup', u));
      if (${button} === 2) t.dispatchEvent(new MouseEvent('contextmenu', o)); else t.dispatchEvent(new MouseEvent('click', u));
      return true;
    })()`);
  }

  // Text into a field as typing would leave it: React reads the native value setter and the input event
  type(selector, text, replace = false) {
    return this.eval(`(() => {
      const el = document.querySelector(${JSON.stringify(selector)});
      el.focus();
      const value = ${replace ? JSON.stringify(text) : `el.value + ${JSON.stringify(text)}`};
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value').set.call(el, value);
      el.selectionStart = el.selectionEnd = value.length;
      el.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'insertText', data: ${JSON.stringify(text)} }));
      return true;
    })()`);
  }

  key(selector, key) {
    return this.eval(`(() => {
      const el = document.querySelector(${JSON.stringify(selector)});
      const o = { key: ${JSON.stringify(key)}, code: ${JSON.stringify(key)}, bubbles: true, cancelable: true };
      el.dispatchEvent(new KeyboardEvent('keydown', o));
      el.dispatchEvent(new KeyboardEvent('keyup', o));
      return true;
    })()`);
  }

  shot(file) {
    return this.call({ op: 'screenshot', path: file });
  }

  close() {
    try { this.sock.end(); } catch { /* already closed */ }
  }
}

await main();
