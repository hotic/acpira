import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import { Shell } from './sidecarShell';

// The Shared tab against the Rust sidecar: the view is read from the open files, actions write them back.
// The sidecar runs with a temp HOME, so the real ~/.agents and the agents' own folders are never read or written

describe('shared config contract', () => {
  const shells: Shell[] = [];
  const dirs: string[] = [];
  afterEach(async () => {
    for (const s of shells.splice(0)) await s.kill();
    for (const d of dirs.splice(0)) rmSync(d, { recursive: true, force: true, maxRetries: 5, retryDelay: 50 });
  });

  function setup() {
    const temp = (p: string) => { const d = mkdtempSync(join(tmpdir(), p)); dirs.push(d); return d; };
    const data = temp('acpira-shared-data-');
    const home = temp('acpira-shared-home-');
    const root = temp('acpira-shared-ws-');
    mkdirSync(join(root, '.git'));
    mkdirSync(join(home, '.agents/skills/demo'), { recursive: true });
    writeFileSync(join(home, '.agents/skills/demo/SKILL.md'), '---\nname: demo\ndescription: A demo skill\n---\n\nBody\n');
    writeFileSync(join(home, '.agents/mcp.json'), JSON.stringify({ mcpServers: { files: { command: 'echo', args: ['g'] } } }));
    writeFileSync(join(root, 'AGENTS.md'), '# Project rules\n\nBe brief.\n');
    // `$CONFIG` (Devin's global AGENTS.md) is %APPDATA% on Windows and XDG_CONFIG_HOME elsewhere: both stay in the temp home
    const s = new Shell(data, root, undefined, { HOME: home, USERPROFILE: home, APPDATA: join(home, 'AppData', 'Roaming'), XDG_CONFIG_HOME: join(home, '.config') });
    shells.push(s);
    return { s, home, root };
  }

  it('reads the open files and edits the MCP list through actions', async () => {
    const { s, home, root } = setup();
    await s.hello({ client: { name: 'contract', version: '0', capabilities: [] } });
    await s.open('V');

    s.view('V', { type: 'shared' });
    const first = await s.hostMsg('V', 'shared');
    expect(first.error).toBeUndefined();
    expect(first.view.home).toBe(home);
    expect(first.view.root).toBe(root);
    expect(first.view.auto).toBe(false);
    expect(first.view.skills).toMatchObject([{ name: 'demo', description: 'A demo skill', scope: 'global' }]);
    expect(first.view.mcp).toMatchObject([{ name: 'files', scope: 'global', transport: 'stdio', enabled: true, shadowed: false }]);
    expect(first.view.prompts.find(p => p.scope === 'project')).toMatchObject({ exists: true, preview: '# Project rules\nBe brief.' });
    expect(first.view.prompts.find(p => p.scope === 'global')).toMatchObject({ exists: false });

    // A project server of the same name shadows the global one
    s.view('V', { type: 'sharedAction', action: { kind: 'addMcp', scope: 'project', json: '{"command":"echo","args":["p"]}', name: 'files' } });
    const added = await s.hostMsg('V', 'shared', m => m.view.mcp.length === 2);
    expect(added.view.mcp.find(m => m.scope === 'global')?.shadowed).toBe(true);
    expect(JSON.parse(readFileSync(join(root, '.mcp.json'), 'utf8'))).toEqual({ mcpServers: { files: { command: 'echo', args: ['p'] } } });

    s.view('V', { type: 'sharedAction', action: { kind: 'toggleMcp', scope: 'project', name: 'files', enabled: false } });
    const off = await s.hostMsg('V', 'shared', m => m.view.mcp.some(x => x.scope === 'project' && !x.enabled));
    // A disabled project entry still shadows the global one: that is how a global server is turned off for one project
    expect(off.view.mcp.find(m => m.scope === 'global')?.shadowed).toBe(true);

    // A name already taken is refused with an error and the file is left as it was
    s.view('V', { type: 'sharedAction', action: { kind: 'addMcp', scope: 'global', json: '{"command":"x"}', name: 'files' } });
    const refused = await s.hostMsg('V', 'shared', m => !!m.error);
    expect(refused.view.mcp).toHaveLength(2);

    s.view('V', { type: 'sharedAction', action: { kind: 'removeMcp', scope: 'project', name: 'files' } });
    await s.hostMsg('V', 'shared', m => m.view.mcp.length === 1 && !m.error);
  });

  it('saves a prompt verbatim and turns overwrite on and off', async () => {
    const { s, home } = setup();
    await s.hello({ client: { name: 'contract', version: '0', capabilities: [] } });
    await s.open('V');

    // The editor's text lands byte for byte, Markdown untouched
    const text = '# Rules\n\n- no **watermark**\n\n```sh\necho hi\n```';
    s.view('V', { type: 'sharedAction', action: { kind: 'savePrompt', scope: 'global', text, base: '' } });
    const saved = await s.hostMsg('V', 'shared', m => !!m.view.prompts.find(p => p.scope === 'global')?.exists);
    expect(saved.error).toBeUndefined();
    expect(readFileSync(join(home, '.agents/AGENTS.md'), 'utf8')).toBe(text);
    expect(saved.view.prompts.find(p => p.scope === 'global')?.text).toBe(text);
    // An edit that started from another text is refused and the file stays
    s.view('V', { type: 'sharedAction', action: { kind: 'savePrompt', scope: 'global', text: 'lost', base: '' } });
    expect((await s.hostMsg('V', 'shared', m => !!m.error)).view.prompts.find(p => p.scope === 'global')?.text).toBe(text);

    s.view('V', { type: 'sharedAction', action: { kind: 'overwrite', on: true, merge: [] } });
    const on = await s.hostMsg('V', 'shared', m => m.view.overwrite);
    expect(on.error).toBeUndefined();
    // Whatever agents this machine has installed, every user-level link point is in place
    expect(on.view.plan).toEqual([]);
    s.view('V', { type: 'sharedAction', action: { kind: 'overwrite', on: false } });
    await s.hostMsg('V', 'shared', m => !m.view.overwrite && !m.error);
  });

  it('creates a skill and turns links on and off', async () => {
    const { s, home } = setup();
    await s.hello({ client: { name: 'contract', version: '0', capabilities: [] } });
    await s.open('V');

    s.view('V', { type: 'sharedAction', action: { kind: 'createSkill', scope: 'global', name: 'fresh' } });
    const created = await s.hostMsg('V', 'shared', m => m.view.skills.some(x => x.name === 'fresh'));
    expect(created.error).toBeUndefined();
    expect(readFileSync(join(home, '.agents/skills/fresh/SKILL.md'), 'utf8')).toContain('name: fresh');

    // User level waits for the link panel; its skill items are what the panel would switch on
    expect(created.view.auto).toBe(false);
    const picks = created.view.plan.filter(p => p.kind === 'skill').map(p => ({ at: p.at, choice: 'link' as const }));
    s.view('V', { type: 'sharedAction', action: { kind: 'link', picks, auto: true } });
    const linked = await s.hostMsg('V', 'shared', m => m.view.auto);
    // Whatever agents this machine has installed, nothing is left missing once every link was made
    expect(linked.view.skills.flatMap(x => x.reach).filter(r => r.state === 'missing')).toEqual([]);
    expect(linked.view.plan.filter(p => p.kind === 'skill')).toEqual([]);

    s.view('V', { type: 'sharedAction', action: { kind: 'unlink' } });
    await s.hostMsg('V', 'shared', m => !m.view.auto && !m.view.userLinked);

    // Project level is on by default and can be turned off
    expect(created.view.projectAuto).toBe(true);
    s.view('V', { type: 'sharedAction', action: { kind: 'projectAuto', on: false } });
    await s.hostMsg('V', 'shared', m => !m.view.projectAuto);
  });
});
