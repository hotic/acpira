// End-to-end smoke of the desktop app (docs/dev/desktop.md): the real app on a temp ACPIRA_HOME, userData and HOME, with
// test/fake-agent.ts as its only agent, driven over the DevTools protocol. A fresh home starts no session; two projects
// each get a session and one round trip; @ search lists only the shown session's project; a session link brings a session
// back; after the app quits and the engine's idle grace runs out no agent process is left.
//
//   node scripts/smoke.mjs [--app <Acpira.app or its executable>] [--engine <acpira binary>] [--keep] [--shots DIR]
//
// Without --app it runs dist/main.cjs (built first) with the workspace's debug engine (`cargo build` first; --engine or
// ACPIRA_SIDECAR_BIN replace it); --app runs a packaged build on its bundled engine unless --engine is given. Needs the
// repository's bundles (`pnpm build`) and a desktop session: the app opens a real window for a few seconds, then the run
// waits out the engine's 30 s idle grace. --keep leaves the temp folder (logs, records) in place; a failure always does
import { execFileSync, spawn } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, realpathSync, rmSync, writeFileSync, appendFileSync } from 'node:fs';
import { createServer } from 'node:http';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { basename, delimiter, dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { buildSync } from 'esbuild';

const DESKTOP = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const REPO = resolve(DESKTOP, '..');
const EXE = process.platform === 'win32' ? '.exe' : '';

const argv = process.argv.slice(2);
const option = name => { const i = argv.indexOf(name); return i >= 0 ? argv[i + 1] : undefined; };
const appArg = option('--app');
const keep = argv.includes('--keep');
// --shots DIR: screenshots of the steps worth a look (the tool panel), for a run checked by eye
const shotDir = option('--shots') && resolve(option('--shots'));
let engine = option('--engine') ?? process.env.ACPIRA_SIDECAR_BIN?.trim();

const sleep = ms => new Promise(r => setTimeout(r, ms));
const say = line => console.log(`smoke: ${line}`);
class SmokeError extends Error {}
const fail = message => { throw new SmokeError(message); };

// A short base on Unix: the engine's socket path under the temp home has to stay below 100 bytes
const root = realpathSync(mkdtempSync(join(process.platform === 'win32' ? tmpdir() : '/tmp', 'acpira-smoke-')));
const home = join(root, 'home');
const data = join(root, 'data');
const user = join(root, 'user');
const alpha = join(root, 'smoke-alpha');
const beta = join(root, 'smoke-beta');
const fakeAgent = join(root, 'fake-agent.mjs');
const logFile = join(root, 'app.log');
let appLog = '';

let app;
let page;
let passed = false;

async function main() {
  try {
    for (const f of ['dist/webview/main.js', 'dist/picker.js']) if (!existsSync(join(REPO, f))) fail(`${f} is missing: run pnpm build in the repository root`);

    // The app under test, and the engine it runs on
    let bin;
    if (appArg) {
      bin = appArg.endsWith('.app') ? join(appArg, 'Contents', 'MacOS', basename(appArg, '.app')) : appArg;
      if (!existsSync(bin)) fail(`no executable at ${bin}`);
    } else {
      execFileSync(process.execPath, ['build.mjs'], { cwd: DESKTOP, stdio: 'inherit' });
      bin = createRequire(join(DESKTOP, 'package.json'))('electron');
      if (!engine) {
        execFileSync('cargo', ['build', '--quiet', '--bin', 'acpira'], { cwd: join(REPO, 'rust'), stdio: 'inherit' });
        engine = join(REPO, 'rust', 'target', 'debug', `acpira${EXE}`);
      }
    }
    const agentsBin = engine ?? join(dirname(dirname(bin)), 'Resources', 'bin', `acpira${EXE}`);

    // Two projects with a file each, the fake agent as its own copy (only this run's processes match its path), and the
    // app's settings as a first launch would leave them plus the agent: no VS Code import, English copy to match
    for (const d of [home, data, user, alpha, beta, ...(shotDir ? [shotDir] : [])]) mkdirSync(d, { recursive: true });
    for (const [dir, file] of [[alpha, 'alpha-notes.md'], [beta, 'beta-notes.md']]) {
      execFileSync('git', ['init', '-q', dir]);
      writeFileSync(join(dir, file), `# ${file}\n`);
    }
    mkdirSync(join(alpha, 'docs'), { recursive: true });
    writeFileSync(join(alpha, 'docs', 'guide.md'), '# guide\n');
    buildSync({ entryPoints: [join(REPO, 'test', 'fake-agent.ts')], outfile: fakeAgent, bundle: true, platform: 'node', format: 'esm', logLevel: 'warning' });

    const env = { ...process.env, HOME: home, ACPIRA_HOME: data, ACPIRA_DESKTOP_USER_DATA: user, ACPIRA_DESKTOP_PICK: [alpha, beta].join(delimiter) };
    // Run as an app (an editor's terminal exports ELECTRON_RUN_AS_NODE), on the persistent engine with its usual grace (an
    // SSH login would stretch it to 3 h), without the screenshot hook
    for (const k of ['ELECTRON_RUN_AS_NODE', 'ACPIRA_DESKTOP_SHOT', 'ACPIRA_DESKTOP_SHOT_DELAY_MS', 'ACPIRA_ENGINE', 'ACPIRA_SIDECAR_BIN', 'SSH_CONNECTION', 'SSH_CLIENT', 'SSH_TTY']) delete env[k];
    if (engine) env.ACPIRA_SIDECAR_BIN = engine;

    // The built-in ids, read on a home of their own: the data home stays untouched until the app's first hello seeds the
    // agent list from the settings below
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

    // Port 0: Chromium picks one and writes it to DevToolsActivePort in userData. Occluded windows keep rendering, since
    // the page's transitions wait for frames
    const launch = extra => spawn(bin, [...(appArg ? [] : [DESKTOP]), ...extra], { env, stdio: ['ignore', 'pipe', 'pipe'] });
    app = launch(['--remote-debugging-port=0', '--use-mock-keychain', '--disable-backgrounding-occluded-windows', '--disable-renderer-backgrounding', '--disable-background-timer-throttling']);
    const collect = chunk => { const s = chunk.toString(); appLog += s; appendFileSync(logFile, s); };
    app.stdout.on('data', collect);
    app.stderr.on('data', collect);
    const appExit = new Promise(r => app.once('exit', r));
    say(`app ${appArg ?? 'dist/main.cjs'} on ${engine ?? 'its bundled engine'}, temp ${root}`);

    page = await Page.connect(user);

    await page.waitFor('the empty state', `document.body.innerText.includes('Open a folder to start')`);
    await sleep(1000);
    if (await page.eval(`document.querySelectorAll('[data-session]').length`)) fail('a session row showed on a fresh home');
    if (records().length) fail(`a fresh home wrote session records: ${records().join(', ')}`);
    say('fresh home shows the empty state and starts no session');

    // The empty state's button, then the column's: each takes the next scripted folder, adds the project and opens a
    // session there
    await page.click('the empty state button', `[...document.querySelectorAll('button')].find(b => b.innerText.includes('Open folder'))`);
    await page.waitFor('the first project', `document.body.innerText.includes('smoke-alpha')`);
    const a = await roundTrip('smoke alpha');
    say(`project smoke-alpha: session ${a}, one round trip`);

    // The Projects label's "+" menu holds "Add project" beside "New section"
    await page.mouse('the add menu', `document.querySelector('button[aria-label="New"]')`);
    await page.click('the add project item', `[...document.querySelectorAll('[role="menuitem"]')].find(e => e.textContent.trim() === 'Add project')`);
    await page.waitFor('the second project\'s start page', `document.body.innerText.includes('smoke-beta') && !document.body.innerText.includes('hello world')`);
    const inBeta = await mentions();
    if (!inBeta.some(n => n.includes('beta-notes.md')) || inBeta.some(n => n.includes('alpha-notes.md'))) fail(`@ search in smoke-beta listed: ${inBeta.join(' | ')}`);
    say('@ search in smoke-beta lists only its own file');
    const b = await roundTrip('smoke beta');
    if (b === a) fail('both projects show the same session');
    say(`project smoke-beta: session ${b}, one round trip`);

    // A second launch with the link hands it to this instance (second-instance), which brings session A back
    await waitUntil('session A\'s record', () => existsSync(join(data, 'sessions', `${a}.json`)), 10_000);
    const second = launch([`acpira://session/${a}`]);
    second.stdout.on('data', collect);
    second.stderr.on('data', collect);
    await within('the second instance to hand its link over and exit', new Promise(r => second.once('exit', r)), 15_000);
    await page.waitFor('session A after its link', `document.querySelector('[data-session][aria-selected="true"]')?.getAttribute('data-session') === ${JSON.stringify(a)} && document.body.innerText.includes('smoke alpha')`);
    say('acpira://session link brought session A back');
    const inAlpha = await mentions();
    if (!inAlpha.some(n => n.includes('alpha-notes.md')) || inAlpha.some(n => n.includes('beta-notes.md'))) fail(`@ search in smoke-alpha listed: ${inAlpha.join(' | ')}`);
    say('@ search in smoke-alpha lists only its own file');

    // The tool panel starts closed; its toggle sits clear of the chat header's buttons, opted out of the window drag region.
    // It opens on the project's tree, a folder eases open on click, a file opens in the viewer beside it, and "+" then Browser
    // runs the browser page in a frame that talks through this page (no preload in a frame)
    const panelShown = `!!document.querySelector('[data-tool-panel-wrap][data-shown]')`;
    if (await page.eval(panelShown)) fail('the tool panel was open in a fresh window');
    const toggleClear = await page.eval(`(() => {
      const t = document.querySelector('[data-panel-toggle] button'), m = document.querySelector('[data-sheet-header] button:last-of-type');
      return t && m && getComputedStyle(t).webkitAppRegion === 'no-drag' && m.getBoundingClientRect().right <= t.getBoundingClientRect().left;
    })()`);
    if (!toggleClear) fail('the tool panel toggle overlapped the chat header or stayed in the drag region');
    await page.mouse('the tool panel toggle', `document.querySelector('[data-panel-toggle] button')`);
    const dirRow = `document.querySelector('[data-tree] [data-path="docs"]')`;
    const treeRow = `[...document.querySelectorAll('[data-tree] [role="treeitem"]')].find(e => e.textContent.includes('alpha-notes.md'))`;
    // Clicked once the panel has eased open: rows still sliding in sit off the page
    await page.waitFor('the project tree', `!!document.querySelector('[data-tool-panel-wrap][data-shown]:not([data-moving])') && !!(${dirRow}) && !!(${treeRow})`);
    const dirAligned = await page.eval(`(() => {
      const r = ${dirRow}, [lead, chev] = r.querySelectorAll('svg');
      const cy = e => { const b = e.getBoundingClientRect(); return b.top + b.height / 2; };
      return Math.abs(cy(lead) - cy(chev)) <= 1 && Math.abs(cy(lead) - cy(r)) <= 1;
    })()`);
    if (!dirAligned) fail('a folder row in the tree did not centre its chevron with its icon');
    await page.mouse('docs folder in the tree', dirRow);
    await page.waitFor('docs folder easing open', `(() => {
      const fold = (${dirRow})?.nextElementSibling;
      return !!document.querySelector('[data-tree] [data-path="docs/guide.md"]') && !!fold?.getAnimations().length;
    })()`);
    await page.mouse('alpha-notes.md in the tree', treeRow);
    await page.waitFor('the file in the viewer', `document.querySelector('[data-viewer]')?.innerText.includes('# alpha-notes.md')`);
    if (shotDir) await page.shot(join(shotDir, 'panel-files.png'));
    // A session switch remounts the panel: back on a project shown before, the tree paints its last listing at once and
    // refreshes behind it (it used to blank for a frame, then show ignored rows undimmed). Sampled every frame
    await page.mouse('session B in the column', `document.querySelector('[data-session="${b}"]')`);
    await page.waitFor('smoke-beta\'s tree', `!!document.querySelector('[data-tree] [data-path="beta-notes.md"]')`);
    await page.eval(`(() => {
      const samples = window.__treeRows = [];
      const end = performance.now() + 1500;
      const step = () => { samples.push(document.querySelectorAll('[data-tree] [data-path]').length); if (performance.now() < end) requestAnimationFrame(step); };
      requestAnimationFrame(step);
    })()`);
    await page.mouse('session A in the column', `document.querySelector('[data-session="${a}"]')`);
    await page.waitFor('session A shown', `document.querySelector('[data-session][aria-selected="true"]')?.getAttribute('data-session') === ${JSON.stringify(a)}`);
    await sleep(1600);
    const rows = await page.eval('window.__treeRows');
    // The first frames still show beta's tree (2 rows), then alpha's: docs, guide.md under it, alpha-notes.md
    if (!rows.length || rows.some(n => n === 0)) fail(`the tree blanked on a switch back to its project: rows per frame ${rows.join(',')}`);
    say(`a switch back to a project shown before keeps its tree painted (${rows.length} frames, never empty)`);
    await page.mouse('the new tab button', `document.querySelector('[data-panel-tabs] button[aria-label="New tab"]')`);
    await page.mouse('the browser tile', `[...document.querySelectorAll('[data-new-tab] button')].find(b => b.textContent.trim() === 'Browser')`);
    await page.waitFor('the browser page in its tab', `(() => { const f = document.querySelector('iframe[src*="host=browser"]'); return !!f?.contentDocument?.querySelector('[data-browser-toolbar] input, input'); })()`);
    // A loopback page through the tab's relay and the engine's preview proxy, shown in the tab's inset sheet
    const site = createServer((_q, r) => { r.writeHead(200, { 'content-type': 'text/html' }); r.end('<title>Smoke site</title><h1>smoke site</h1>'); });
    await new Promise(r => site.listen(0, '127.0.0.1', r));
    try {
      const tabId = await page.eval(`new URLSearchParams(document.querySelector('iframe[src*="host=browser"]').src.split('?')[1]).get('tab')`);
      await page.eval(`window.__acpiraApi.postMessage({ type: 'panelBrowserOpen', tab: ${JSON.stringify(tabId)}, url: 'http://127.0.0.1:${site.address().port}/' })`);
      await page.waitFor('the site in the browser tab', `(() => { const d = document.querySelector('iframe[src*="host=browser"]')?.contentDocument; return !!d?.querySelector('[data-sheet] iframe') && [...document.querySelectorAll('[data-panel-tabs] [role="tab"]')].some(t => t.textContent.includes('Smoke site')); })()`);
      if (shotDir) { await sleep(500); await page.shot(join(shotDir, 'panel-browser.png')); }
    } finally {
      site.close();
    }
    say('the tool panel opens on the tree, shows a file, and runs a browser tab');

    // "+" offers Files, Terminal and Browser in that order. Terminal is a login shell the engine runs in the project folder,
    // drawn by xterm in the tab: it takes the focus, a typed line runs there, the tab takes the shell's name, and closing
    // the tab ends the shell
    await page.mouse('the new tab button', `document.querySelector('[data-panel-tabs] button[aria-label="New tab"]')`);
    const tiles = await page.waitFor('the new tab\'s tiles', `(() => { const t = [...document.querySelectorAll('[data-new-tab] .grid button')].map(b => b.textContent.trim()); return t.length && t.join(','); })()`);
    if (tiles !== 'Files,Terminal,Browser') fail(`the new tab's tiles read ${tiles}`);
    await page.mouse('the terminal tile', `[...document.querySelectorAll('[data-new-tab] button')].find(b => b.textContent.trim() === 'Terminal')`);
    await page.waitFor('the terminal taking the focus', `!!document.activeElement?.closest('[data-terminal] .xterm')`);
    // The shell's own pid, written where the shell runs
    await page.type('printf %s $$ > terminal.txt');
    await page.key('Enter');
    const marker = join(alpha, 'terminal.txt');
    await waitUntil('the typed line to run in the project folder', () => existsSync(marker) && /^\d+$/.test(readFileSync(marker, 'utf8')), 15_000);
    const shellPid = Number(readFileSync(marker, 'utf8'));
    const terminalTab = `[...document.querySelectorAll('[data-panel-tabs] [role="tab"]')].find(t => /^(zsh|bash|sh|fish)$/.test(t.textContent.trim()))`;
    await page.waitFor('the terminal tab named after its shell', `!!(${terminalTab})`);
    if (shotDir) { await sleep(300); await page.shot(join(shotDir, 'panel-terminal.png')); }
    await page.mouse('the terminal tab\'s close button', `(${terminalTab})?.querySelector('[aria-label="Close tab"]')`);
    await waitUntil('the shell to end with its tab', () => !alive(shellPid), 5_000);
    say('"+" offers Files, Terminal and Browser; a terminal tab runs a typed line in the project and its shell ends with the tab');

    await page.mouse('the tool panel toggle', `document.querySelector('[data-panel-toggle] button')`);
    await page.waitFor('the tool panel closed', `!${panelShown}`);

    // A language change re-renders in place: the column and the selected row stay the same nodes (a remount replayed
    // every mount animation and closed open menus), and the transcript's memo rows take the new strings too
    await page.eval(`(window.__smokeNodes = [document.querySelector('[data-session-panel]'), document.querySelector('[data-session][aria-selected="true"]')]).every(Boolean)`);
    await page.eval(`window.__acpiraApi.postMessage({ type: 'setSetting', key: 'language', value: 'zh-CN' })`);
    await page.waitFor('the page in Chinese', `document.body.innerText.includes('新会话')`);
    // The finished turn's row ("Done") and the plan ("To-do plan") are memo rows
    const sheet = await page.eval(`document.querySelector('[data-sheet]').innerText`);
    if (!sheet.includes('已完成') || !sheet.includes('待办计划') || sheet.includes('To-do plan')) fail(`transcript rows kept their English after a language change: ${sheet.slice(0, 300)}`);
    if (!(await page.eval(`window.__smokeNodes.every(n => n.isConnected)`))) fail('a language change remounted the page');
    await page.eval(`window.__acpiraApi.postMessage({ type: 'setSetting', key: 'language', value: 'en' })`);
    await page.waitFor('the page in English again', `document.body.innerText.includes('New session')`);
    say('a language change re-renders in place');

    // The gear menu's language switch: the pick re-renders the page under the open menus, which stay open with the tick
    // moved, so another pick is one click away (a remount used to close them)
    const item = text => `[...document.querySelectorAll('[role="menuitem"]')].find(e => e.textContent.trim() === ${JSON.stringify(text)})`;
    const itemWith = text => `[...document.querySelectorAll('[role="menuitem"]')].find(e => e.textContent.includes(${JSON.stringify(text)}))`;
    await page.mouse('the gear', `document.querySelector('button[aria-label="Settings and help"]')`);
    await page.openSubmenu('the language list', itemWith('Language'), `!!(${item('简体中文')})`);
    await page.mouse('Simplified Chinese in the language list', item('简体中文'));
    await page.waitFor('the page in Chinese from the menu', `document.body.innerText.includes('新会话')`);
    await page.waitFor('both menus open after the pick', `!!(${itemWith('语言')}) && !!(${item('English')})`);
    await page.mouse('English in the language list', item('English'));
    await page.waitFor('the page in English again from the menu', `document.body.innerText.includes('New session') && !!(${itemWith('Language')})`);
    await page.key('Escape');
    await page.key('Escape');
    await page.waitFor('the menus closed', `!document.querySelector('[role="menu"]')`);
    say('a language picked in the gear menu leaves the menu open');

    // The gear's layout switch, at the window's default size: files first lays the project's explorer and the stage (the
    // file opened above as a tab) left of the chat and docks the column at the window's right edge, and the corner toggle
    // folds that column instead of the tool panel. Switched back, the conversation layout returns with the panel closed
    const filesLayout = `!!document.querySelector('.acp-shell[data-layout="files"]')`;
    await page.mouse('the gear', `document.querySelector('button[aria-label="Settings and help"]')`);
    await page.openSubmenu('the layout list', itemWith('Layout'), `!!(${item('Files first')})`);
    await page.mouse('Files first in the layout list', item('Files first'));
    await page.waitFor('the files layout', `${filesLayout} && !!document.querySelector('[data-explorer] [data-tree]')`);
    // The gear moves to the explorer's foot, a new mount, so the switch closes its menus. No Esc for them: a key the page
    // leaves alone goes to the macOS menu bar, where a synthetic Esc crashed the app in HIToolbox (2026-10-24, as before)
    await page.waitFor('the menus closed with the switch', `!document.querySelector('[role="menu"]')`);
    const columns = await page.eval(`['[data-explorer]', '[data-stage]', '[data-sheet]', '[data-session-panel]'].map(s => Math.round(document.querySelector(s)?.getBoundingClientRect().left ?? -1))`);
    if (columns.some((x, i) => x < 0 || (i > 0 && x <= columns[i - 1]))) fail(`the files layout's columns are out of order: ${columns.join(' ')}`);
    await page.waitFor('alpha-notes.md as a tab on the stage', `[...document.querySelectorAll('[data-stage-tabs] [role="tab"]')].some(t => t.textContent.includes('alpha-notes.md'))`);
    if (shotDir) await page.shot(join(shotDir, 'layout-files.png'));
    const columnFolded = `!!document.querySelector('[data-session-panel][data-column-hidden]')`;
    await page.mouse('the session list toggle', `document.querySelector('[data-panel-toggle] button')`);
    await page.waitFor('the session column folded', columnFolded);
    await page.mouse('the session list toggle', `document.querySelector('[data-panel-toggle] button')`);
    await page.waitFor('the session column back', `!${columnFolded}`);
    await page.eval(`window.__acpiraApi.postMessage({ type: 'setSetting', key: 'desktopLayout', value: 'chat' })`);
    await page.waitFor('the conversation layout again', `!${filesLayout} && !${panelShown}`);
    say(`files first lays out explorer, stage, chat and column (x ${columns.join(' / ')}); the corner toggle folds the column`);

    // A narrow window (an emulated 480 px viewport): the title row keeps only the list's toggle, the header centres its title
    // over the agent and the project, and the list is a page over the whole window. Esc closes it with the focus outside it,
    // a pick closes it on that session
    const height = await page.eval('innerHeight');
    await page.send('Emulation.setDeviceMetricsOverride', { width: 480, height, deviceScaleFactor: 0, mobile: false });
    await page.waitFor('the narrow layout', `!!document.querySelector('.acp-shell[data-narrow] [data-header-sub]') && !document.querySelector('[data-title-row] button[aria-label="Back"]')`);
    const toggle = `document.querySelector('[data-title-row] [data-session-toggle]')`;
    const pageOpen = `(() => { const a = document.querySelector('[data-session-panel]'); return !!document.querySelector('.acp-shell[data-drawer-open]') && getComputedStyle(a).transform === 'none' && Math.round(a.getBoundingClientRect().width) === innerWidth; })()`;
    const pageClosed = `!document.querySelector('.acp-shell[data-drawer-open]') && getComputedStyle(document.querySelector('[data-session-panel]')).visibility === 'hidden'`;
    const shown = id => `document.querySelector('[data-session][aria-selected="true"]')?.getAttribute('data-session') === ${JSON.stringify(id)}`;
    await page.mouse('the list toggle', toggle);
    await page.waitFor('the list page over the window', pageOpen);
    await page.eval('document.activeElement?.blur()');
    await page.key('Escape');
    await page.waitFor('Esc to close the list page', pageClosed);
    await page.mouse('the list toggle', toggle);
    await page.waitFor('the list page again', pageOpen);
    await page.mouse('session B\'s row', `document.querySelector('[data-session="${b}"]')`);
    await page.waitFor('the list page closed on session B', `${pageClosed} && ${shown(b)}`);
    await page.mouse('the list toggle', toggle);
    await page.waitFor('the list page once more', pageOpen);
    await page.mouse('session A\'s row', `document.querySelector('[data-session="${a}"]')`);
    await page.waitFor('the list page closed on session A', `${pageClosed} && ${shown(a)}`);
    // Widened right after that close, the window docks the column in place: a slide in from the left still running here
    // put the next right-click off the page (2026-10-23)
    await page.send('Emulation.clearDeviceMetricsOverride');
    await page.waitFor('the docked column back', `!document.querySelector('.acp-shell[data-narrow]') && !!document.querySelector('[data-title-row] button[aria-label="Back"]')`);
    const slid = await page.eval(`document.querySelector('[data-session-panel]').getAnimations().filter(a => a.transitionProperty === 'transform').length`);
    if (slid) fail('the column slid in as the window crossed the width tier');
    say('a narrow window shows the list as a page; Esc and a pick close it; widened, the column docks in place');

    // "Remove from list" on a project's right-click menu takes it off the column although its session stays (the
    // removal is remembered, so the session's folder is not listed again for it)
    await waitUntil('session B\'s record', () => existsSync(join(data, 'sessions', `${b}.json`)), 10_000);
    const betaRow = `[...document.querySelectorAll('[data-project-row]')].find(e => e.title.endsWith('smoke-beta'))`;
    await page.mouse('smoke-beta\'s project row', betaRow, 'right');
    await page.mouse('the remove item', `[...document.querySelectorAll('[role="menuitem"]')].find(e => e.textContent.includes('Remove from list'))`);
    await page.waitFor('smoke-beta to leave the column', `!${betaRow} && !!document.querySelector('[data-project-row]')`);
    if (!existsSync(join(data, 'sessions', `${b}.json`))) fail('removing a project deleted its session');
    say('a removed project leaves the column, its session stays');

    // Quit; the engine outlives the app by design (sessions outlive windows) and ends after its idle grace, taking the
    // agent processes with it
    page.close();
    page = undefined;
    const enginePid = Number(/sidecar ready: pid (\d+)/.exec(appLog)?.[1]);
    if (!enginePid) fail('the app log names no engine pid');
    app.kill('SIGTERM');
    await within('the app to quit on SIGTERM', appExit, 15_000);
    app = undefined;
    await waitUntil('the engine to end after its idle grace', () => !alive(enginePid), 90_000);
    const left = processes().filter(p => p.command.includes(fakeAgent));
    if (left.length) fail(`agent processes left after the engine ended:\n${left.map(p => `${p.pid} ${p.ppid} ${p.command}`).join('\n')}`);
    say(`engine ${enginePid} ended, no agent process left`);
    const orphans = processes().filter(p => p.ppid === 1 && p.command.includes('fake-agent'));
    if (orphans.length) say(`note: fake-agent orphans from other runs:\n${orphans.map(p => `  ${p.pid} ${p.command}`).join('\n')}`);
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
    if (passed && !keep) rmSync(root, { recursive: true, force: true });
    else say(`kept ${root}${passed ? '' : ' (its engine ends by itself within its 30 s idle grace)'}`);
  }
}

// Session records the engine wrote under the temp home
function records() {
  const dir = join(data, 'sessions');
  return existsSync(dir) ? readdirSync(dir).filter(f => /^[0-9a-f-]{36}\.json$/.test(f)) : [];
}

// One prompt from the shown session's composer: the reply ("hello world", the fake agent's default) and the turn's end
// (the send button back), then the session's row, which is the selected one
async function roundTrip(text) {
  await page.waitFor('an enabled composer', `!!document.querySelector('[data-start-dock] textarea:not([disabled])')`);
  await page.eval(`document.querySelector('[data-start-dock] textarea').focus()`);
  await page.type(text);
  await page.key('Enter');
  await page.waitFor(`the reply to "${text}"`, `document.body.innerText.includes(${JSON.stringify(text)}) && document.body.innerText.includes('hello world')`);
  await page.waitFor('the turn to end', `!!document.querySelector('[data-start-dock] button[aria-label="Send"]')`);
  return page.waitFor('the session row', `document.querySelector('[data-session][aria-selected="true"]')?.getAttribute('data-session')`);
}

// The file suggestions for "@notes" in the shown session's composer, then the query erased again
async function mentions() {
  const query = '@notes';
  await page.waitFor('an enabled composer', `!!document.querySelector('[data-start-dock] textarea:not([disabled])')`);
  await page.eval(`document.querySelector('[data-start-dock] textarea').focus()`);
  await page.type(query);
  const names = await page.waitFor('file suggestions', `(() => {
    const names = [...document.querySelectorAll('[role=listbox]:not([aria-label]) [role=option]')].map(o => o.innerText.trim());
    return names.some(n => n.includes('notes.md')) ? names : undefined;
  })()`);
  for (let i = 0; i < query.length; i++) await page.key('Backspace');
  await page.waitFor('an empty composer', `document.querySelector('[data-start-dock] textarea').value === ''`);
  return names;
}

async function within(what, promise, timeout) {
  let timer;
  const late = new Promise((_, reject) => { timer = setTimeout(() => reject(new SmokeError(`timed out waiting for ${what}`)), timeout); });
  try { return await Promise.race([promise, late]); } finally { clearTimeout(timer); }
}

async function waitUntil(what, check, timeout) {
  const end = Date.now() + timeout;
  while (!check()) {
    if (Date.now() > end) fail(`timed out waiting for ${what}`);
    await sleep(250);
  }
}

function alive(pid) {
  try { process.kill(pid, 0); return true; } catch (e) { return e.code === 'EPERM'; }
}

function processes() {
  return execFileSync('ps', ['-eo', 'pid,ppid,command'], { encoding: 'utf8' }).split('\n').slice(1).map(line => {
    const m = /^\s*(\d+)\s+(\d+)\s+(.*)$/.exec(line);
    return m ? { pid: Number(m[1]), ppid: Number(m[2]), command: m[3] } : undefined;
  }).filter(Boolean);
}

// The chat window's page over the DevTools protocol
class Page {
  static async connect(userData) {
    const portFile = join(userData, 'DevToolsActivePort');
    await waitUntil('the DevTools port', () => existsSync(portFile) && readFileSync(portFile, 'utf8').trim() !== '', 30_000);
    const port = Number(readFileSync(portFile, 'utf8').split('\n')[0]);
    let target;
    const end = Date.now() + 30_000;
    while (!target) {
      if (Date.now() > end) fail('no chat page target');
      const list = await fetch(`http://127.0.0.1:${port}/json`).then(r => r.json()).catch(() => []);
      target = list.find(t => t.type === 'page' && t.url.includes('host=editor'));
      if (!target) await sleep(200);
    }
    const ws = new WebSocket(target.webSocketDebuggerUrl);
    await new Promise((res, rej) => { ws.addEventListener('open', res, { once: true }); ws.addEventListener('error', rej, { once: true }); });
    const p = new Page(ws);
    // The window may sit behind others: the page still behaves as focused (focus, caret, key handling)
    await p.send('Emulation.setFocusEmulationEnabled', { enabled: true });
    return p;
  }

  constructor(ws) {
    this.ws = ws;
    this.seq = 0;
    this.pending = new Map();
    ws.addEventListener('message', e => {
      const m = JSON.parse(e.data);
      const done = m.id && this.pending.get(m.id);
      if (!done) return;
      this.pending.delete(m.id);
      done(m);
    });
    // A page or app that goes away fails the call in flight: left pending, it emptied Node's event loop and the run ended
    // as an unsettled top-level await with no word of what happened
    ws.addEventListener('close', () => {
      for (const done of this.pending.values()) done({ error: { message: 'the DevTools connection closed (did the window or the app go away?)' } });
      this.pending.clear();
    });
  }

  send(method, params = {}) {
    return new Promise((res, rej) => {
      if (this.ws.readyState !== WebSocket.OPEN) { rej(new Error(`${method}: the DevTools connection is closed`)); return; }
      const id = ++this.seq;
      this.pending.set(id, m => (m.error ? rej(new Error(`${method}: ${m.error.message}`)) : res(m.result)));
      this.ws.send(JSON.stringify({ id, method, params }));
    });
  }

  async eval(expression) {
    const r = await this.send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
    if (r.exceptionDetails) throw new Error(r.exceptionDetails.exception?.description ?? r.exceptionDetails.text);
    return r.result.value;
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

  // A real click at the element's centre: Base UI opens menus on pointer and contextmenu events, which element.click()
  // does not send
  async mouse(what, element, button = 'left') {
    const at = await this.waitFor(what, `(() => { const e = ${element}; if (!e) return; const r = e.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })()`);
    // The point has to land on the element itself: something laid over it would take the press instead, and a point off
    // the page (an element still sliding in) hits nothing at all
    const hit = await this.eval(`(() => { const e = ${element}; const h = document.elementFromPoint(${at.x}, ${at.y}); return h && e.contains(h) ? '' : h ? 'covered by ' + h.outerHTML.slice(0, 160) : 'off the page'; })()`);
    if (hit) fail(`${what} at ${Math.round(at.x)},${Math.round(at.y)} is ${hit}`);
    await this.send('Input.dispatchMouseEvent', { type: 'mouseMoved', ...at });
    await this.send('Input.dispatchMouseEvent', { type: 'mousePressed', ...at, button, buttons: button === 'right' ? 2 : 1, clickCount: 1 });
    await this.send('Input.dispatchMouseEvent', { type: 'mouseReleased', ...at, button, buttons: 0, clickCount: 1 });
  }
  // A submenu from its trigger row: Base UI opens one on hover after a short delay and toggles it on a press, so a press
  // that lands after the hover opened it closes it again (the language step timed out that way once, 2026-10-23). Hover
  // first; press only when nothing opened
  async openSubmenu(what, trigger, opened) {
    const at = await this.waitFor(what, `(() => { const e = ${trigger}; if (!e) return; const r = e.getBoundingClientRect(); return { x: r.x + r.width / 2, y: r.y + r.height / 2 }; })()`);
    await this.send('Input.dispatchMouseEvent', { type: 'mouseMoved', ...at });
    try {
      await this.waitFor(`${what} on hover`, opened, 1500);
    } catch {
      await this.mouse(what, trigger);
      await this.waitFor(`${what} after a press`, opened);
    }
  }

  type(text) {
    return this.send('Input.insertText', { text });
  }

  // Keys as a keyboard sends them: Enter carries its text, Backspace is a raw key down. On macOS the native code is the
  // Mac's own (kVK_Escape 53): Electron hands a key the page leaves alone to the menu bar, and an Esc carrying 27 (the
  // minus key there) crashed the app in the input method code (2026-10-23)
  async key(name) {
    const spec = { Enter: { code: 'Enter', keyCode: 13, text: '\r' }, Backspace: { code: 'Backspace', keyCode: 8 }, Escape: { code: 'Escape', keyCode: 27, mac: 53 } }[name];
    const native = process.platform === 'darwin' && spec.mac ? spec.mac : spec.keyCode;
    const base = { key: name, code: spec.code, windowsVirtualKeyCode: spec.keyCode, nativeVirtualKeyCode: native };
    await this.send('Input.dispatchKeyEvent', { type: spec.text ? 'keyDown' : 'rawKeyDown', ...base, ...(spec.text ? { text: spec.text, unmodifiedText: spec.text } : {}) });
    await this.send('Input.dispatchKeyEvent', { type: 'keyUp', ...base });
  }

  async shot(file) {
    const r = await this.send('Page.captureScreenshot', { format: 'png' });
    writeFileSync(file, Buffer.from(r.data, 'base64'));
  }

  close() {
    try { this.ws.close(); } catch { /* already closed */ }
  }
}

await main();
