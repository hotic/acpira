import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import { AGENT_EXT } from '../src/host/agentExt';
import { expandPath, parseFrontmatter, parseJsonLoose, parseJsonMcp, parseOpencodeMcp, parseTomlMcp, scanInventory, scopeOf } from '../src/host/inventory';

// A fake home + workspace laid out the way the three CLIs expect, so the scan is checked against real file shapes without touching the machine
let root: string;
let home: string;
let cwd: string;

beforeAll(async () => {
  root = await mkdtemp(join(tmpdir(), 'acpira-inv-'));
  home = join(root, 'home');
  cwd = join(root, 'work');
  const put = async (p: string, text: string) => { await mkdir(join(p, '..'), { recursive: true }); await writeFile(p, text); };

  // Grok: toml tables, a Claude-compat .mcp.json in the project, user + project skills
  await put(join(home, '.grok/config.toml'), [
    '[models]', 'default = "asgard"', '',
    '[mcp_servers.linear]', 'url = "https://mcp.linear.app/mcp"', '',
    '# a stdio one', '[mcp_servers.filesystem]', 'command = "npx"', 'args = ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]', 'enabled = false', '',
    '[mcp_servers."quoted name"]', 'command = "/usr/local/bin/tool"', 'type = "stdio"',
  ].join('\n'));
  await put(join(cwd, '.mcp.json'), JSON.stringify({ mcpServers: { hilfa: { command: 'hilfa', args: ['serve'] } } }));
  await put(join(home, '.grok/skills/dig/SKILL.md'), '---\nname: dig\ndescription: 挖历史会话\n---\n# Dig\n');
  await put(join(cwd, '.grok/skills/local-only/SKILL.md'), '---\ndescription: >\n  folded\n  description\n---\n');
  await put(join(cwd, 'AGENTS.md'), '# rules\n');

  // Devin: JSONC mcp_config with a comment and trailing comma, rules directory, shared ~/.agents/skills
  await put(join(home, '.config/devin/mcp_config.json'), '{\n  // user-wide\n  "mcpServers": {\n    "jina": { "url": "https://mcp.jina.ai/sse", "transport": "sse" },\n  },\n}\n');
  await put(join(home, '.config/devin/config.json'), '{ "version": 1 }');
  await put(join(home, '.agents/skills/hallmark/SKILL.md'), '---\nname: hallmark\ndescription: "Anti-slop design"\n---\n');
  await put(join(cwd, '.devin/rules/style.md'), 'no italics\n');
  await put(join(cwd, '.devin/rules/notes.txt'), 'ignored\n');

  // Kimi: mcp.json with a disabled entry
  await put(join(home, '.kimi-code/mcp.json'), JSON.stringify({ mcpServers: { legacy: { transport: 'sse', url: 'https://x/sse', enabled: false }, fs: { command: 'npx', args: ['-y', 'fs'] } } }));
  await put(join(home, '.kimi-code/AGENTS.md'), 'global\n');

  // OpenCode: the `mcp` object lives in opencode.json (JSONC) next to the project
  await put(join(cwd, 'opencode.json'), [
    '{',
    '  // project MCP servers',
    '  "mcp": {',
    '    "fs": { "type": "local", "command": ["npx", "-y", "@mcp/fs"], "environment": { "K": "v" } },',
    '    "web": { "type": "remote", "url": "https://mcp.example.com/x", "enabled": false },',
    '    "bare": { "url": "https://bare.example.com/mcp" },',
    '  },',
    '}',
  ].join('\n'));

  // DSH: a <name>/SKILL.md bundle, a flat <name>.md with a frontmatter name, and a flat file without one
  await put(join(home, '.dsh/skills/bundled/SKILL.md'), '---\nname: bundled\ndescription: bundle\n---\n');
  await put(join(home, '.dsh/skills/flat-file.md'), '---\nname: flat-skill\ndescription: flat\n---\n# Flat\n');
  await put(join(home, '.dsh/skills/plain.md'), '# no frontmatter at all\n');

  // Codex: toml MCP tables in config.toml like Grok's, ~/.codex/skills + AGENTS.md rules
  await put(join(home, '.codex/config.toml'), '[mcp_servers.context7]\nurl = "https://mcp.context7.example/mcp"\n');
  await put(join(home, '.codex/skills/shipit/SKILL.md'), '---\nname: shipit\ndescription: release it\n---\n');

  // Claude: settings.json config, ~/.claude.json MCP, ~/.claude/skills and CLAUDE.md rules
  await put(join(home, '.claude/settings.json'), '{ "model": "opus" }');
  await put(join(home, '.claude.json'), JSON.stringify({ mcpServers: { remote: { url: 'https://claude-mcp.example/x' } } }));
  await put(join(home, '.claude/skills/audit/SKILL.md'), '---\nname: audit\n---\n');
  await put(join(home, '.claude/CLAUDE.md'), 'global rules\n');
});

afterAll(async () => { await rm(root, { recursive: true, force: true }); });

const env = () => ({ home, cwd, platform: 'darwin' as const, env: {} });

describe('path templates', () => {
  it('expands ~ / $CONFIG / relative, and scopes user vs project', () => {
    expect(expandPath('~/.grok/skills', env())).toBe(join(home, '.grok/skills'));
    expect(expandPath('$CONFIG/devin/config.json', env())).toBe(join(home, '.config/devin/config.json'));
    expect(expandPath('.devin/rules', env())).toBe(join(cwd, '.devin/rules'));
    expect(scopeOf('~/.grok/skills')).toBe('user');
    expect(scopeOf('$CONFIG/devin/skills')).toBe('user');
    expect(scopeOf('.agents/skills')).toBe('project');
  });
  it('honors XDG_CONFIG_HOME and APPDATA on Windows', () => {
    expect(expandPath('$CONFIG/devin/x', { home, cwd, platform: 'linux', env: { XDG_CONFIG_HOME: '/xdg' } })).toBe('/xdg/devin/x');
    expect(expandPath('$CONFIG/devin/x', { home, cwd, platform: 'win32', env: { APPDATA: 'C:\\Users\\u\\AppData\\Roaming' } })).toBe(join('C:\\Users\\u\\AppData\\Roaming', 'devin/x'));
  });
});

describe('parsers', () => {
  it('toml: [mcp_servers.*] tables with quoted names, args arrays, enabled, comments', () => {
    const got = parseTomlMcp('[mcp_servers.a]\nurl = "https://a" # trailing\n[other]\ncommand = "nope"\n[mcp_servers."b c"]\ncommand = "bin"\nargs = ["x", "y z"]\nenabled = false\n');
    expect(got).toEqual([
      { name: 'a', transport: 'http', target: 'https://a', enabled: true },
      { name: 'b c', transport: 'stdio', target: 'bin x y z', enabled: false },
    ]);
  });
  it('json: mcpServers with type / transport / serverUrl variants; loose JSON tolerates comments and trailing commas', () => {
    expect(parseJsonMcp('{"mcpServers":{"h":{"serverUrl":"https://h","type":"http"},"s":{"command":"s","args":["--x"],"disabled":true}}}')).toEqual([
      { name: 'h', transport: 'http', target: 'https://h', enabled: true },
      { name: 's', transport: 'stdio', target: 's --x', enabled: false },
    ]);
    expect(parseJsonLoose('{ /* c */ "a": [1, 2,], // tail\n "s": "// not a comment" }')).toEqual({ a: [1, 2], s: '// not a comment' });
    expect(parseJsonMcp('not json')).toEqual([]);
  });
  it('opencode: local/remote entries, array and string commands, inferred transport, enabled flag, JSONC', () => {
    const got = parseOpencodeMcp([
      '{',
      '  // a comment',
      '  "mcp": {',
      '    "arr": { "type": "local", "command": ["npx", "serve it"], "environment": { "K": "v" }, },',
      '    "str": { "type": "local", "command": "tool run" },',
      '    "rem": { "type": "remote", "url": "https://r/x", "enabled": false },',
      '    "inf": { "url": "https://i/y" },',
      '    "junk": "not an object",',
      '  },',
      '}',
    ].join('\n'));
    expect(got).toEqual([
      { name: 'arr', transport: 'stdio', target: 'npx serve it', enabled: true },
      { name: 'str', transport: 'stdio', target: 'tool run', enabled: true },
      { name: 'rem', transport: 'http', target: 'https://r/x', enabled: false },
      { name: 'inf', transport: 'http', target: 'https://i/y', enabled: true },
    ]);
    expect(parseOpencodeMcp('{"other": {}}')).toEqual([]);
    expect(parseOpencodeMcp('not json')).toEqual([]);
  });
  it('frontmatter: plain, quoted and folded values', () => {
    expect(parseFrontmatter('---\nname: x\ndescription: "quoted"\n---\nbody')).toEqual({ name: 'x', description: 'quoted' });
    expect(parseFrontmatter('---\ndescription: >-\n  one\n  two\n---\n')).toEqual({ name: undefined, description: 'one two' });
    expect(parseFrontmatter('no frontmatter')).toEqual({});
  });
});

describe('scanInventory', () => {
  it('grok: toml + .mcp.json servers, user + project skills, rule files with exists flags', async () => {
    const inv = await scanInventory({ agent: 'grok', ext: AGENT_EXT.grok, binary: '/usr/local/bin/grok' }, env());
    expect(inv.steer).toBe(false);
    expect(inv.mcp.map(m => `${m.name}:${m.transport}:${m.scope}:${m.enabled}`)).toEqual([
      'linear:http:user:true', 'filesystem:stdio:user:false', 'quoted name:stdio:user:true', 'hilfa:stdio:project:true', 'remote:http:user:true',
    ]);
    expect(inv.skills.map(s => [s.name, s.scope, s.description])).toEqual([
      ['dig', 'user', '挖历史会话'],
      ['local-only', 'project', 'folded description'],
      ['audit', 'user', undefined],
    ]);
    expect(inv.rules.find(r => r.path.endsWith('AGENTS.md'))).toMatchObject({ exists: true, scope: 'project' });
    expect(inv.rules.find(r => r.path.endsWith('CLAUDE.md'))).toMatchObject({ exists: false });
    expect(inv.config.map(c => c.exists)).toEqual([true, false]);
  });
  it('devin: JSONC mcp_config, shared ~/.agents skills, rules directory lists only markdown', async () => {
    const inv = await scanInventory({ agent: 'devin', ext: AGENT_EXT.devin, binary: null }, env());
    expect(inv.steer).toBe(true);
    expect(inv.binary).toBeNull();
    expect(inv.mcp).toEqual([{ name: 'jina', transport: 'sse', target: 'https://mcp.jina.ai/sse', source: join(home, '.config/devin/mcp_config.json'), scope: 'user', enabled: true }]);
    expect(inv.skills.map(s => s.name)).toEqual(['hallmark']);
    const dirRules = inv.rules.filter(r => r.path.includes('.devin/rules'));
    expect(dirRules.map(r => r.path.split('/').pop())).toEqual(['style.md']);
  });
  it('kimi: disabled entries keep their flag; global AGENTS.md is user scope', async () => {
    const inv = await scanInventory({ agent: 'kimi', ext: AGENT_EXT.kimi, binary: '/x/kimi' }, env());
    expect(inv.mcp.map(m => `${m.name}:${m.enabled}`)).toEqual(['legacy:false', 'fs:true', 'hilfa:true']);
    expect(inv.rules.find(r => r.scope === 'user')).toMatchObject({ exists: true });
  });
  it('opencode: the mcp object in project opencode.json parses as JSONC', async () => {
    const inv = await scanInventory({ agent: 'opencode', ext: AGENT_EXT.opencode, binary: '/x/opencode' }, env());
    expect(inv.mcp.map(m => `${m.name}:${m.transport}:${m.scope}:${m.enabled}`)).toEqual([
      'fs:stdio:project:true', 'web:http:project:false', 'bare:http:project:true',
    ]);
  });
  it('dsh: bundle directories and named flat <name>.md files are skills; frontmatter-less markdown is ignored', async () => {
    const inv = await scanInventory({ agent: 'dsh', ext: AGENT_EXT.dsh, binary: null }, env());
    const dsh = inv.skills.filter(s => s.path.includes('.dsh/skills'));
    expect(dsh.map(s => s.name)).toEqual(['bundled', 'flat-skill']);
    expect(dsh.find(s => s.name === 'flat-skill')?.path).toBe(join(home, '.dsh/skills/flat-file.md'));
  });
  it('codex: toml config + shared ~/.agents skill, AGENTS.md rules', async () => {
    const inv = await scanInventory({ agent: 'codex', ext: AGENT_EXT.codex, binary: '/x/codex-acp' }, env());
    expect(inv.mcp.map(m => `${m.name}:${m.transport}:${m.scope}`)).toEqual(['context7:http:user']);
    expect(inv.skills.map(s => `${s.name}:${s.scope}`).sort()).toEqual(['hallmark:user', 'shipit:user']);
    expect(inv.rules.find(r => r.path === join(cwd, 'AGENTS.md'))).toMatchObject({ exists: true, scope: 'project' });
    expect(inv.rules.find(r => r.path.endsWith('AGENTS.override.md'))).toMatchObject({ exists: false });
  });
  it('claude: ~/.claude.json + project .mcp.json servers, settings and CLAUDE.md rules', async () => {
    const inv = await scanInventory({ agent: 'claude', ext: AGENT_EXT.claude, binary: '/x/claude-agent-acp' }, env());
    expect(inv.mcp.map(m => `${m.name}:${m.scope}`).sort()).toEqual(['hilfa:project', 'remote:user']);
    expect(inv.skills.map(s => s.name)).toEqual(['audit']);
    expect(inv.config.map(c => c.exists)).toEqual([true, false, false]);
    expect(inv.rules.find(r => r.path === join(home, '.claude/CLAUDE.md'))).toMatchObject({ exists: true, scope: 'user' });
    expect(inv.rules.find(r => r.path === join(cwd, 'CLAUDE.md'))).toMatchObject({ exists: false, scope: 'project' });
  });
  it('unknown agent: binary status only', async () => {
    const inv = await scanInventory({ agent: 'custom', binary: '/opt/custom' }, env());
    expect(inv).toMatchObject({ agent: 'custom', binary: '/opt/custom', steer: false, config: [], mcp: [], skills: [], rules: [] });
  });
});
