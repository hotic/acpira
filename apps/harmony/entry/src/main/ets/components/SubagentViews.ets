import { Metrics, WebMetrics, WebType } from '../design/Tokens';
import { Block, SubagentSummary } from '../wire/Blocks';
import {
  isWaiting, rootRows, secondLine, statusLabel, subagentElapsed, subagentMeta, subagentTitle
} from '../logic/Subagents';
import { tp } from '../logic/Folding';
import { Lucide } from './Glyphs';
import { PermissionCard } from './Cards';
import { AcpRow, Orb, RowSpec, ticker } from './Rows';

// apps/ios Transcript/SubagentViews.swift (chat/subagents/SubagentGroup.tsx, the DelegatedSubagents / ChildPermissions
// parts of Turns.tsx), after apps/android ui/subagents/SubagentViews.kt: the rows a turn shows for the children it
// delegated to, their state marks, and a child's pending approval under its row.

/** A waiting child takes the warn tone; how it ended colours the rest */
function statusInk(n: SubagentSummary): ResourceColor {
  if (isWaiting(n)) {
    return $r('app.color.warn');
  }
  return n.state === 'completed' ? $r('app.color.ok') : n.state === 'failed' ? $r('app.color.danger') : $r('app.color.fg3');
}

/** A node's identity for a ForEach key: what its row would show differently */
function nodeSig(n: SubagentSummary): string {
  return `${n.id}:${n.state}:${n.endedAt ?? ''}:${(n.activity ?? '').length}:${(n.result ?? '').length}:` +
    `${(n.permissions ?? []).length}:${n.question !== undefined}`;
}

/**
 * icons.tsx stateIcon: a pending card outranks the state, as that is what the child is blocked on; a running child
 * shows the working ring
 */
@ComponentV2
struct SubagentStateIcon {
  @Param node: SubagentSummary = { id: '', turnIndex: 0, state: 'running', announcedAt: 0 };

  build() {
    Stack() {
      if ((this.node.permissions ?? []).length > 0) {
        Lucide({ name: 'hand', tint: $r('app.color.warn'), extent: WebMetrics.icon, weight: WebMetrics.strokeIcon })
      } else if (this.node.question !== undefined) {
        Lucide({ name: 'messageCircleQuestion', tint: $r('app.color.warn'), extent: WebMetrics.icon, weight: WebMetrics.strokeIcon })
      } else if (this.node.state === 'running') {
        Orb()
      } else if (this.node.state === 'completed') {
        Lucide({ name: 'check', tint: $r('app.color.fg2'), extent: WebMetrics.icon, weight: WebMetrics.strokeIcon })
      } else if (this.node.state === 'failed') {
        Lucide({ name: 'triangleAlert', tint: $r('app.color.danger'), extent: WebMetrics.icon, weight: WebMetrics.strokeIcon })
      } else if (this.node.state === 'cancelled') {
        Lucide({ name: 'x', tint: $r('app.color.fg2'), extent: WebMetrics.icon, weight: WebMetrics.strokeIcon })
      } else {
        Lucide({ name: 'unplug', tint: $r('app.color.fg2'), extent: WebMetrics.icon, weight: WebMetrics.strokeIcon })
      }
    }
    .width(WebMetrics.lead)
    .height(WebMetrics.lead)
  }
}

/**
 * An unframed row like the process folds: the state mark, the title with its faint meta, the status and elapsed time at
 * the end, then the live activity / waiting / result excerpt one step fainter. No chevron: the row opens a page
 */
@ComponentV2
export struct SubagentRow {
  @Param node: SubagentSummary = { id: '', turnIndex: 0, state: 'running', announcedAt: 0 };
  @Param all: SubagentSummary[] = [];
  @Param now: () => number = () => Date.now();

  aboutToAppear(): void {
    ticker.start();
  }

  elapsed(): string {
    // Reads the shared tick, so a running child's time follows the clock
    const tick = ticker.tick;
    return tick >= 0 ? subagentElapsed(this.node, this.now()) : '';
  }

  detail(): string {
    const line = secondLine(this.node);
    return line === statusLabel(this.node) ? '' : line;
  }

  build() {
    Row() {
      SubagentStateIcon({ node: this.node })
        .margin({ left: (WebMetrics.icon - WebMetrics.lead) / 2, right: Metrics.gap - (WebMetrics.lead - WebMetrics.icon) / 2 })
      Column({ space: Metrics.gap }) {
        Row({ space: Metrics.s2 }) {
          // Title and meta share the room left by the status; the meta gives way first
          Row({ space: Metrics.s2 }) {
            Text(subagentTitle(this.node))
              .fontSize(WebType.text2)
              .lineHeight(WebType.text2Line)
              .fontColor($r('app.color.fg1'))
              .maxLines(1)
              .textOverflow({ overflow: TextOverflow.Ellipsis })
              .flexShrink(0)
              .constraintSize({ maxWidth: '100%' })
            ForEach(subagentMeta(this.node, this.all.length > 0 ? this.all : [this.node]), (part: string) => {
              Text(part)
                .fontSize(WebType.text3)
                .lineHeight(WebType.text3Line)
                .fontColor($r('app.color.fg3'))
                .maxLines(1)
                .textOverflow({ overflow: TextOverflow.Ellipsis })
                .flexShrink(1)
            }, (part: string, i: number) => `${i}:${part}`)
          }
          .layoutWeight(1)
          .clip(true)
          Text(statusLabel(this.node))
            .fontSize(WebType.text3)
            .lineHeight(WebType.text3Line)
            .fontColor(statusInk(this.node))
            .maxLines(1)
            .flexShrink(0)
          Text(this.elapsed())
            .fontSize(WebType.text3)
            .lineHeight(WebType.text3Line)
            .fontColor($r('app.color.fg3'))
            .fontFeature('"tnum" on')
            .maxLines(1)
            .flexShrink(0)
        }
        .width('100%')
        if (this.detail().length > 0) {
          Text(this.detail())
            .fontSize(WebType.text3)
            .lineHeight(WebType.text3Line)
            .fontColor($r('app.color.fg3'))
            .maxLines(1)
            .textOverflow({ overflow: TextOverflow.Ellipsis })
            .width('100%')
        }
      }
      .alignItems(HorizontalAlign.Start)
      .layoutWeight(1)
    }
    .alignItems(VerticalAlign.Center)
    .padding({ top: Metrics.gapHalf, bottom: Metrics.gapHalf })
    .constraintSize({ minHeight: WebMetrics.chatRow })
    .width('100%')
  }
}

/**
 * SubagentGroup: the turn's top-level children in announce order, finished ones included (a workflow's rows are its
 * progress). No gap between rows: each carries its own padding
 */
@ComponentV2
export struct SubagentGroup {
  @Param nodes: SubagentSummary[] = [];
  @Param all: SubagentSummary[] = [];
  @Param now: () => number = () => Date.now();

  build() {
    Column() {
      ForEach(rootRows(this.nodes), (n: SubagentSummary) => {
        SubagentRow({ node: n, all: this.all, now: this.now })
      }, (n: SubagentSummary) => nodeSig(n))
    }
    .accessibilityText(tp('subagents.group', 'n', `${this.nodes.length}`))
    .width('100%')
  }
}

/** The path from the root to a node, through its parents */
function breadcrumb(id: string, all: SubagentSummary[]): SubagentSummary[] {
  const out: SubagentSummary[] = [];
  const seen = new Set<string>();
  let cur = all.find((n: SubagentSummary) => n.id === id);
  while (cur !== undefined && !seen.has(cur.id)) {
    seen.add(cur.id);
    out.unshift(cur);
    const parent = cur.parentId;
    cur = parent !== undefined ? all.find((n: SubagentSummary) => n.id === parent) : undefined;
  }
  return out;
}

/** One child's pending approval: who asks, then its card */
class ChildCard {
  node: SubagentSummary = { id: '', turnIndex: 0, state: 'running', announcedAt: 0 };
  block: Block = { type: 'permission' };
}

/**
 * ChildPermissions: a child's pending approval under the rows, with a provenance line naming the path to it. The card
 * answers through the root session, which routes it to the child
 */
@ComponentV2
export struct ChildPermissions {
  @Param nodes: SubagentSummary[] = [];
  @Param all: SubagentSummary[] = [];
  @Event onPermission: (blockId: string, optionId: string) => void = (blockId: string, optionId: string) => {};

  cards(): ChildCard[] {
    const out: ChildCard[] = [];
    for (const n of this.nodes) {
      for (const b of n.permissions ?? []) {
        const c = new ChildCard();
        c.node = n;
        c.block = b;
        out.push(c);
      }
    }
    return out;
  }

  path(id: string): string {
    return breadcrumb(id, this.all.length > 0 ? this.all : this.nodes).map(subagentTitle).join(' › ');
  }

  provenance(c: ChildCard): RowSpec {
    const s = new RowSpec();
    s.lead = 'none';
    s.height = WebMetrics.rowDense;
    s.label = tp('subagents.provenance', 'path', this.path(c.node.id));
    s.labelInk = $r('app.color.fg3');
    return s;
  }

  build() {
    Column({ space: Metrics.gap }) {
      ForEach(this.cards(), (c: ChildCard) => {
        Column({ space: Metrics.s1 }) {
          AcpRow({ spec: this.provenance(c) })
          PermissionCard({ block: c.block, onPick: (optionId: string) => this.onPermission(c.block.id ?? '', optionId) })
        }
        .width('100%')
      }, (c: ChildCard) => `${c.block.id ?? ''}:${(c.block.options ?? []).length}`)
    }
    .width('100%')
  }
}

/** DelegatedSubagents: the rows of one delegation point, with any pending child approval right under them */
@ComponentV2
export struct DelegatedSubagents {
  @Param ids: string[] = [];
  @Param delegations: Map<string, SubagentSummary[]> = new Map<string, SubagentSummary[]>();
  @Param all: SubagentSummary[] = [];
  @Param now: () => number = () => Date.now();
  @Event onPermission: (blockId: string, optionId: string) => void = (blockId: string, optionId: string) => {};

  nodes(): SubagentSummary[] {
    const out: SubagentSummary[] = [];
    for (const id of this.ids) {
      for (const n of this.delegations.get(id) ?? []) {
        out.push(n);
      }
    }
    return out;
  }

  build() {
    if (this.nodes().length > 0) {
      Column({ space: Metrics.gap }) {
        SubagentGroup({ nodes: this.nodes(), all: this.all, now: this.now })
        ChildPermissions({ nodes: this.nodes(), all: this.all, onPermission: this.onPermission })
      }
      .width('100%')
    }
  }
}
