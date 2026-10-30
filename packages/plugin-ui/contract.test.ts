// The editor types a TypeScript plugin gets (`acpira plugin new --lang ts` writes rust/.../plugins/acpira.d.ts) must
// name exactly what runs: the frame kit's exports here, and `definePlugin` in the engine's SDK
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';
import * as Kit from './index';

const plugins = new URL('../../rust/crates/acpira-host/src/plugins/', import.meta.url);
const types = readFileSync(fileURLToPath(new URL('acpira.d.ts', plugins)), 'utf8');

// The value names (functions and consts) one `declare module` block exports
function declared(module: string): string[] {
  const start = types.indexOf(`declare module '${module}' {`);
  const body = types.slice(start, types.indexOf('\n}\n', start));
  return [...body.matchAll(/^ {2}export (?:function|const) (\w+)/gm)].map(m => m[1]!).sort();
}

describe('plugin type declarations', () => {
  it('list every value the frame kit exports, and nothing else', () => {
    expect(declared('@acpira/plugin-ui')).toEqual(Object.keys(Kit).sort());
  });

  it('list what a definition imports from the SDK', () => {
    const sdk = readFileSync(fileURLToPath(new URL('sdk.mjs', plugins)), 'utf8');
    for (const name of declared('@acpira/plugin')) expect(sdk).toMatch(new RegExp(`^export function ${name}\\(`, 'm'));
    expect(declared('@acpira/plugin')).toContain('definePlugin');
  });
});
