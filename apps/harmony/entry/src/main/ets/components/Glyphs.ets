import { PathShape } from '@kit.ArkUI';
import { LUCIDE, MARKS } from '../design/Icons';
import { Metrics, Type } from '../design/Tokens';

// Lucide icons and vendor marks, drawn from their SVG path data. Path commands are px and a Shape's viewport did not
// scale them in the Previewer, so each path is scaled here from its 24 unit box to the icon's size in px.

const scaled: Map<string, string[]> = new Map<string, string[]>();

/** How many numbers each command takes */
function arity(cmd: string): number {
  switch (cmd.toUpperCase()) {
    case 'M':
    case 'L':
    case 'T':
      return 2;
    case 'H':
    case 'V':
      return 1;
    case 'S':
    case 'Q':
      return 4;
    case 'C':
      return 6;
    case 'A':
      return 7;
    default:
      return 0;
  }
}

/** One SVG path with every length multiplied by `k` (an arc's rotation and flags stay) */
export function scalePath(d: string, k: number): string {
  const out: string[] = [];
  let i = 0;
  let cmd = '';
  let index = 0;
  const number = /^[+-]?(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?/;
  while (i < d.length) {
    const c = d.charAt(i);
    if (/[a-zA-Z]/.test(c)) {
      cmd = c;
      index = 0;
      out.push(c);
      i++;
      continue;
    }
    if (c === ' ' || c === ',' || c === '\n' || c === '\t') {
      i++;
      continue;
    }
    const n = arity(cmd);
    const slot = n === 0 ? 0 : index % n;
    const upper = cmd.toUpperCase();
    if (upper === 'A' && (slot === 3 || slot === 4)) {
      // Flags are single digits that may run into the next number ("011.5")
      out.push(c);
      i++;
      index++;
      continue;
    }
    const m = d.slice(i).match(number);
    if (m === null) {
      i++;
      continue;
    }
    const value = Number(m[0]);
    const keep = upper === 'A' && slot === 2;
    out.push(`${keep ? value : value * k}`);
    i += m[0].length;
    index++;
  }
  return out.join(' ');
}

function pathsAt(table: Map<string, string[]>, name: string, extent: number): string[] {
  const key = `${name}@${extent}`;
  const hit = scaled.get(key);
  if (hit !== undefined) {
    return hit;
  }
  const k = vp2px(extent) / Metrics.iconBox;
  const paths = (table.get(name) ?? []).map((d: string) => scalePath(d, k));
  scaled.set(key, paths);
  return paths;
}

/** A lucide icon: its stroked paths on the 24 unit box, drawn at `extent` */
@ComponentV2
export struct Lucide {
  @Param name: string = 'square';
  @Param tint: ResourceColor = $r('app.color.fg1');
  @Param extent: number = Metrics.icon;

  build() {
    Stack() {
      ForEach(pathsAt(LUCIDE, this.name, this.extent), (d: string) => {
        Path()
          .commands(d)
          .fill(Color.Transparent)
          .stroke(this.tint)
          .strokeWidth(Metrics.stroke * this.extent / Metrics.iconBox)
          .strokeLineCap(LineCapStyle.Round)
          .strokeLineJoin(LineJoinStyle.Round)
          .antiAlias(true)
          .width(this.extent)
          .height(this.extent)
          .position({ x: 0, y: 0 })
      }, (d: string, i: number) => `${i}`)
    }
    .width(this.extent)
    .height(this.extent)
  }
}

/**
 * An agent's mark (chat/marks.tsx), filled in the row's ink; an agent without one shows its initial, as the other
 * apps do
 */
@ComponentV2
export struct AgentMark {
  @Param agent: string = '';
  @Param name: string = '';
  @Param tint: ResourceColor = $r('app.color.fg2');
  @Param extent: number = Metrics.rowMark;

  build() {
    if (MARKS.has(this.agent)) {
      Stack() {
        ForEach(pathsAt(MARKS, this.agent, this.extent), (d: string) => {
          // The tint fills a box clipped to the mark: a Path's own fill came out black for a resource colour
          Column()
            .width(this.extent)
            .height(this.extent)
            .backgroundColor(this.tint)
            .clipShape(new PathShape({ commands: d }))
            .position({ x: 0, y: 0 })
        }, (d: string, i: number) => `${i}`)
      }
      .width(this.extent)
      .height(this.extent)
    } else {
      Text((this.name.length > 0 ? this.name : this.agent).slice(0, 1).toUpperCase())
        .fontSize(Type.text4)
        .fontWeight(FontWeight.Medium)
        .fontColor(this.tint)
        .textAlign(TextAlign.Center)
        .width(this.extent)
        .height(this.extent)
    }
  }
}
