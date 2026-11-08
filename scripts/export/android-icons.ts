// Writes the lucide icons the webview imports and the vendor marks as one JSON asset for the Android app:
// `pnpm exec tsx scripts/export/android-icons.ts`. Every element becomes SVG path data in lucide's 24-unit box, which
// the app parses with Compose's PathParser (stroked for lucide, filled even-odd for marks); `brands` maps a model name to its mark and `colors` a brand to
// its own colour, so icons match the webview
// and the iOS app. test/android-icons.test.ts fails when the committed file drifts from the imports.
import { readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { type Attrs, type IconNode, brandColors, brandRules, lucideNodes, vendorMarks } from './ios-icons';

const root = fileURLToPath(new URL('../..', import.meta.url));
export const ANDROID_ICONS_FILE = join(root, 'apps/android/app/src/main/assets/icons.json');

const n = (v: string | number | undefined) => Number(v ?? 0);
const pts = (v: string | number | undefined) => String(v ?? '').trim().split(/[\s,]+/).map(Number);

/** One lucide element as path data */
function pathData([tag, a]: [string, Attrs]): string {
  switch (tag) {
    case 'path': return String(a.d);
    case 'circle': {
      const [cx, cy, r] = [n(a.cx), n(a.cy), n(a.r)];
      return `M${cx - r} ${cy}a${r} ${r} 0 1 0 ${2 * r} 0a${r} ${r} 0 1 0 ${-2 * r} 0z`;
    }
    case 'ellipse': {
      const [cx, cy, rx, ry] = [n(a.cx), n(a.cy), n(a.rx), n(a.ry)];
      return `M${cx - rx} ${cy}a${rx} ${ry} 0 1 0 ${2 * rx} 0a${rx} ${ry} 0 1 0 ${-2 * rx} 0z`;
    }
    case 'rect': {
      const [x, y, w, h] = [n(a.x), n(a.y), n(a.width), n(a.height)];
      const r = Math.min(n(a.rx ?? a.ry), w / 2, h / 2);
      if (!r) return `M${x} ${y}h${w}v${h}h${-w}z`;
      return `M${x + r} ${y}h${w - 2 * r}a${r} ${r} 0 0 1 ${r} ${r}v${h - 2 * r}a${r} ${r} 0 0 1 ${-r} ${r}h${-(w - 2 * r)}a${r} ${r} 0 0 1 ${-r} ${-r}v${-(h - 2 * r)}a${r} ${r} 0 0 1 ${r} ${-r}z`;
    }
    case 'line': return `M${n(a.x1)} ${n(a.y1)}L${n(a.x2)} ${n(a.y2)}`;
    case 'polyline':
    case 'polygon': {
      const p = pts(a.points);
      const pairs: string[] = [];
      for (let i = 0; i + 1 < p.length; i += 2) pairs.push(`${p[i]} ${p[i + 1]}`);
      return `M${pairs.join('L')}${tag === 'polygon' ? 'z' : ''}`;
    }
    default: throw new Error(`unsupported lucide element <${tag}>`);
  }
}

const camel = (pascal: string) => pascal[0]!.toLowerCase() + pascal.slice(1);

export async function androidIconsJson(): Promise<string> {
  const lucide: Record<string, string[]> = {};
  for (const [name, node] of await lucideNodes() as [string, IconNode][]) lucide[camel(name)] = node.map(pathData);
  return `${JSON.stringify({ lucide, marks: vendorMarks(), brands: brandRules(), colors: brandColors() }, null, 1)}\n`;
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  writeFileSync(ANDROID_ICONS_FILE, await androidIconsJson());
}

export const readAndroidIcons = () => readFileSync(ANDROID_ICONS_FILE, 'utf8');
