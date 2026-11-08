import { FileHit, SlashCommand } from '../../wire/Wire';
import { Cmp, Metrics, Type } from '../../design/Tokens';
import { MentionPersona, SlashCommands } from '../../logic/SlashCommands';
import { AgentMark, Lucide } from '../Glyphs';

// Completion.tsx, Slash.tsx and Mention.tsx at phone size (apps/ios Chat/Suggestions.swift): the `/` command list and
// the `@` list (personas, then files) as one card over the composer, part of the composer's own column so its rows keep
// their taps.

const IMAGE_EXTENSIONS: string[] = ['png', 'jpg', 'jpeg', 'gif', 'webp', 'heic', 'bmp', 'svg'];

/** attachments.ts imageMimeOf, by extension */
function imageFile(path: string): boolean {
  return IMAGE_EXTENSIONS.includes((path.split('.').pop() ?? '').toLowerCase());
}

/** The command list: a row per hit, scrolling past gooPanelRows */
@ComponentV2
export struct CommandList {
  @Param items: SlashCommand[] = [];
  @Event pick: (command: SlashCommand) => void = () => {};

  build() {
    Scroll() {
      Column() {
        ForEach(this.items, (c: SlashCommand) => {
          Row({ space: Metrics.s2 }) {
            Lucide({ name: 'squareSlash', tint: $r('app.color.fg3'), extent: Metrics.icon })
            Text(`/${c.name}`)
              .fontSize(Type.mono)
              .fontFamily('monospace')
              .fontColor($r('app.color.fg_strong'))
              .maxLines(1)
              .flexShrink(0)
            if ((c.input?.hint ?? '').length > 0) {
              Text(c.input?.hint ?? '')
                .fontSize(Type.mono)
                .fontFamily('monospace')
                .fontColor($r('app.color.fg3'))
                .maxLines(1)
                .flexShrink(1)
            }
            if (c.description.length > 0) {
              Text(c.description)
                .fontSize(Type.text3)
                .fontColor($r('app.color.fg3'))
                .maxLines(1)
                .textOverflow({ overflow: TextOverflow.Ellipsis })
                .layoutWeight(1)
            }
          }
          .suggestionRow()
          .onClick(() => this.pick(c))
        }, (c: SlashCommand) => c.name)
      }
      .padding({ top: Cmp.s1_5, bottom: Cmp.s1_5 })
    }
    .suggestionCard(this.items.length)
  }
}

/** The mention list: personas (their CLI's mark, name, CLI and model faint), then files (name bright, folder faint) */
@ComponentV2
export struct MentionList {
  @Param items: FileHit[] = [];
  @Param personas: MentionPersona[] = [];
  /** Stands in for no rows once the search answered */
  @Param empty: string = '';
  @Event pick: (hit: FileHit) => void = () => {};

  build() {
    Scroll() {
      Column() {
        if (this.items.length === 0) {
          Text(this.empty)
            .fontSize(Type.text3)
            .fontColor($r('app.color.fg3'))
            .padding({ left: Cmp.gooPadX, right: Cmp.gooPadX })
            .height(Cmp.gooRow)
            .width('100%')
        }
        ForEach(this.items, (hit: FileHit) => {
          Row({ space: Metrics.s2 }) {
            if (SlashCommands.personaOf(hit, this.personas) !== undefined) {
              this.persona(SlashCommands.personaOf(hit, this.personas) as MentionPersona)
            } else {
              this.file(hit)
            }
          }
          .suggestionRow()
          .onClick(() => this.pick(hit))
        }, (hit: FileHit) => hit.uri)
      }
      .padding({ top: Cmp.s1_5, bottom: Cmp.s1_5 })
    }
    .suggestionCard(Math.max(1, this.items.length))
  }

  @Builder
  persona(p: MentionPersona) {
    if (p.group) {
      Lucide({ name: 'users', tint: $r('app.color.fg2'), extent: Metrics.icon })
    } else {
      AgentMark({ agent: p.agent, name: p.cli ?? p.name, tint: $r('app.color.fg2'), extent: Metrics.icon })
    }
    Text(p.name)
      .fontSize(Type.text2)
      .fontColor($r('app.color.fg_strong'))
      .maxLines(1)
      .flexShrink(0)
    if (p.cli !== undefined) {
      Text(p.cli).fontSize(Type.text3).fontColor($r('app.color.fg2')).maxLines(1).flexShrink(1)
    }
    if (p.model !== undefined) {
      Text(p.model).fontSize(Type.text3).fontColor($r('app.color.fg3')).maxLines(1).flexShrink(1)
    }
  }

  @Builder
  file(hit: FileHit) {
    Lucide({ name: imageFile(hit.path) ? 'image' : 'fileText', tint: $r('app.color.fg3'), extent: Metrics.icon })
    Text(hit.path.includes('/') ? hit.path.slice(hit.path.lastIndexOf('/') + 1) : hit.path)
      .fontSize(Type.mono)
      .fontFamily('monospace')
      .fontColor($r('app.color.fg_strong'))
      .maxLines(1)
      .flexShrink(0)
    if (hit.path.includes('/')) {
      Text(hit.path.slice(0, hit.path.lastIndexOf('/')))
        .fontSize(Type.text3)
        .fontColor($r('app.color.fg3'))
        .maxLines(1)
        .textOverflow({ overflow: TextOverflow.Ellipsis })
        .ellipsisMode(EllipsisMode.START)
        .layoutWeight(1)
    }
  }
}

@Extend(Row)
function suggestionRow() {
  .width('100%')
  .height(Cmp.gooRow)
  .padding({ left: Cmp.gooPadX, right: Cmp.gooPadX })
  .stateStyles({
    pressed: {
      .backgroundColor($r('app.color.pop_hover'))
    },
    normal: {
      .backgroundColor(Color.Transparent)
    }
  })
}

@Extend(Scroll)
function suggestionCard(rows: number) {
  .scrollBar(BarState.Off)
  .edgeEffect(EdgeEffect.None)
  .align(Alignment.Top)
  .width('100%')
  .height(Math.min(rows, Cmp.gooPanelRows) * Cmp.gooRow + Cmp.s1_5 * 2)
  .backgroundColor($r('app.color.menu_bg'))
  .border({ width: Metrics.hairline, color: $r('app.color.menu_line') })
  .borderRadius(Cmp.rGooPanel)
  .transition(TransitionEffect.OPACITY.combine(TransitionEffect.scale({ x: Cmp.gooSeed, y: Cmp.gooSeed, centerX: 0, centerY: '100%' })))
}
