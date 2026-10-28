// Writes the iOS app's web runtimes: `pnpm exec tsx scripts/export/ios-web.ts`.
// The phone draws what needs a browser engine in WKWebView: mermaid diagrams its native flowchart port does not read
// (mermaid's own browser build, the version the webview's streamdown plugin renders with) and HTML widgets (Tailwind's
// browser build, inlined into each widget as the webview does). Both are copied as they ship; test/ios-web.test.ts fails
// when the committed copies drift from the installed packages.
import { existsSync, mkdirSync, readFileSync, realpathSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../..', import.meta.url));
export const IOS_WEB_DIR = join(root, 'apps/ios/Packages/AcpiraKit/Sources/AcpiraUI/Resources/web');

// Found the way Node looks a package up from `from` (a package.json): packages/ui's dependencies, then theirs; the root
// workspace depends on neither. By path, since these export maps resolve neither `package.json` nor a CommonJS entry
function packageDir(from: string, name: string): string {
  for (let dir = dirname(from); ; dir = dirname(dir)) {
    const candidate = join(dir, 'node_modules', name);
    if (existsSync(join(candidate, 'package.json'))) return realpathSync(candidate);
    if (dirname(dir) === dir) throw new Error(`${name} is not installed under ${dirname(from)}`);
  }
}

export function iosWebFiles(): Map<string, string> {
  const ui = join(root, 'packages/ui/package.json');
  const streamdown = join(packageDir(ui, '@streamdown/mermaid'), 'package.json');
  return new Map([
    ['mermaid.min.js', readFileSync(join(packageDir(streamdown, 'mermaid'), 'dist/mermaid.min.js'), 'utf8')],
    ['tailwind.js', readFileSync(join(packageDir(ui, '@tailwindcss/browser'), 'dist/index.global.js'), 'utf8')],
  ]);
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  mkdirSync(IOS_WEB_DIR, { recursive: true });
  for (const [name, text] of iosWebFiles()) writeFileSync(join(IOS_WEB_DIR, name), text);
}
