import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

// A command button's `icon` is checked against the manifest's list (plugins/manifest.rs BUTTON_ICONS) and drawn from the
// page's (plugins/CommandButtons.tsx): one name missing on the page would pass the check and draw the fallback
const root = fileURLToPath(new URL('..', import.meta.url));
const names = (text: string) => [...text.matchAll(/'([\w-]+)'|"([\w-]+)"|\b([a-z]+):/g)].map(m => m[1] ?? m[2] ?? m[3]).sort();
const rust = /pub const BUTTON_ICONS: &\[&str\] = &\[([\s\S]*?)\];/.exec(readFileSync(join(root, 'rust/crates/acpira-host/src/plugins/manifest.rs'), 'utf8'))?.[1] ?? '';
const page = /export const BUTTON_ICONS: Record<string, LucideIcon> = \{([\s\S]*?)\};/.exec(readFileSync(join(root, 'packages/ui/plugins/CommandButtons.tsx'), 'utf8'))?.[1] ?? '';

describe('command button icons', () => {
  it('are the same on both sides', () => {
    const fromRust = [...rust.matchAll(/"([\w-]+)"/g)].map(m => m[1]).sort();
    expect(fromRust.length).toBeGreaterThan(10);
    expect(names(page)).toEqual(fromRust);
  });
});
