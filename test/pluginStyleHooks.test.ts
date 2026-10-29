import { readFileSync, readdirSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

// The selectors the plugin guide promises a theme's css (acpira_shared::plugins::STYLE_HOOKS) must stay on the page:
// renaming one silently breaks every skin styling through it
const root = fileURLToPath(new URL('..', import.meta.url));
const rust = readFileSync(join(root, 'rust/crates/acpira-shared/src/plugins.rs'), 'utf8');
const list = /pub const STYLE_HOOKS: &\[&str\] = &\[([\s\S]*?)\];/.exec(rust)?.[1] ?? '';
const hooks = [...list.matchAll(/"((?:[^"\\]|\\.)*)"/g)].map(m => m[1]!.replace(/\\"/g, '"'));

function sources(dir: string): string[] {
  return readdirSync(dir).flatMap(name => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return name === 'node_modules' || name === 'test' ? [] : sources(path);
    return /\.tsx?$/.test(name) && !/\.test\.tsx?$/.test(name) ? [path] : [];
  });
}
const page = sources(join(root, 'packages/ui')).map(f => readFileSync(f, 'utf8')).join('\n');

// What the markup must contain for a selector: an attribute (any value, or the stated one) and a class
function needles(selector: string): RegExp[] {
  const out: RegExp[] = [];
  for (const [, name, value] of selector.matchAll(/\[([\w-]+)(?:="([^"]*)")?\]/g)) {
    out.push(value === undefined ? new RegExp(`\\b${name}[=\\s>{]`) : new RegExp(`${name}(?:="${value}"|=\\{[^}]*'${value}')`));
  }
  for (const [, cls] of selector.matchAll(/\.([\w-]+)/g)) out.push(new RegExp(`\\b${cls}\\b`));
  return out;
}

describe('plugin style hooks', () => {
  it('lists some hooks', () => expect(hooks.length).toBeGreaterThan(5));
  it.each(hooks)('the page still carries %s', hook => {
    for (const needle of needles(hook)) expect(page, `${hook}: ${needle}`).toMatch(needle);
  });
});
