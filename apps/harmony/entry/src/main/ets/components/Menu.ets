import { ListMetrics, MenuMetrics, Metrics, Type } from '../design/Tokens';
import { Lucide } from './Glyphs';

// A menu as the iOS app's system menus read (UIMenu, apps/android ui/Menu.kt): one large-radius card a step lighter
// than the page, rows a menu row tall, the current choice ticked on the leading side, glyphs on the trailing side. A
// submenu replaces the level it came from, with its head as the way back. The card sits in a bindMenu whose own chrome
// is cleared (menuOptions), so only this look shows.

/** bindMenu's options for a card drawn here: no system background, blur or arrow */
export function menuOptions(onDisappear: () => void, placement?: Placement): MenuOptions {
  const options: MenuOptions = {
    placement: placement ?? Placement.BottomRight,
    enableArrow: false,
    backgroundColor: Color.Transparent,
    backgroundBlurStyle: BlurStyle.NONE,
    onDisappear: onDisappear
  };
  return options;
}

/** One row. `checked` undefined: an action (no tick column); else whether this choice is the current one */
@ComponentV2
export struct MenuRow {
  @Param text: string = '';
  @Param subtitle: string | undefined = undefined;
  @Param checked: boolean | undefined = undefined;
  @Param icon: string | undefined = undefined;
  @Param danger: boolean = false;
  @Param tag: string = '';
  @Event onTap: () => void = () => {};

  build() {
    Row({ space: Metrics.s2 }) {
      if (this.checked !== undefined) {
        Stack() {
          if (this.checked) {
            Lucide({ name: 'check', tint: this.danger ? $r('app.color.danger') : $r('app.color.fg_strong'), extent: Metrics.icon })
          }
        }
        .width(Metrics.icon)
        .height(Metrics.icon)
      }
      Column() {
        Text(this.text)
          .fontSize(Type.text1)
          .fontColor(this.danger ? $r('app.color.danger') : $r('app.color.fg_strong'))
          .maxLines(1)
          .textOverflow({ overflow: TextOverflow.Ellipsis })
        if (this.subtitle !== undefined) {
          Text(this.subtitle)
            .fontSize(Type.text3)
            .fontColor($r('app.color.fg3'))
            .maxLines(1)
            .textOverflow({ overflow: TextOverflow.Ellipsis })
        }
      }
      .alignItems(HorizontalAlign.Start)
      .layoutWeight(1)
      if (this.icon !== undefined) {
        Stack() {
          Lucide({ name: this.icon, tint: this.danger ? $r('app.color.danger') : $r('app.color.fg_strong'), extent: Metrics.icon })
        }
        .width(Metrics.iconCtl)
        .height(Metrics.iconCtl)
      }
    }
    .id(this.tag)
    .constraintSize({ minHeight: MenuMetrics.row })
    .padding({ left: Metrics.pad, right: Metrics.pad, top: ListMetrics.s1_5, bottom: ListMetrics.s1_5 })
    .width('100%')
    .stateStyles({
      pressed: { .backgroundColor($r('app.color.active')) },
      normal: { .backgroundColor(Color.Transparent) }
    })
    .onClick(() => this.onTap())
  }
}

/** A submenu's head: its title, and the way back to the level above */
@ComponentV2
export struct MenuBack {
  @Param title: string = '';
  @Event onTap: () => void = () => {};

  build() {
    Row({ space: Metrics.s2 }) {
      Lucide({ name: 'chevronLeft', tint: $r('app.color.fg_strong'), extent: Metrics.icon })
      Text(this.title)
        .fontSize(Type.text1)
        .fontWeight(FontWeight.Medium)
        .fontColor($r('app.color.fg_strong'))
        .maxLines(1)
        .textOverflow({ overflow: TextOverflow.Ellipsis })
        .layoutWeight(1)
    }
    .constraintSize({ minHeight: MenuMetrics.row })
    .padding({ left: Metrics.pad, right: Metrics.pad })
    .width('100%')
    .onClick(() => this.onTap())
  }
}

/** A titled section's caption (UIMenu's section title), over the rows it names */
@ComponentV2
export struct MenuCaption {
  @Param title: string = '';

  build() {
    Text(this.title)
      .fontSize(Type.text4)
      .fontColor($r('app.color.fg3'))
      .maxLines(1)
      .padding({ left: Metrics.pad, right: Metrics.pad, top: Metrics.s2, bottom: Metrics.s1 })
      .width('100%')
  }
}

/** The band between a menu's sections */
@ComponentV2
export struct MenuGap {
  build() {
    Column()
      .height(MenuMetrics.gap)
      .width('100%')
      .backgroundColor($r('app.color.menu_gap'))
  }
}

/** The card's look: wrap a menu's rows in it */
@Extend(Column)
function menuCardLook() {
  .backgroundColor($r('app.color.menu_bg'))
  .borderRadius(MenuMetrics.radius)
  .border({ width: Metrics.hairline, color: $r('app.color.menu_line') })
  .shadow({ radius: MenuMetrics.shadow, color: $r('app.color.scrim'), offsetY: Metrics.s1 })
  .clip(true)
  .padding({ top: Metrics.s1, bottom: Metrics.s1 })
  .width(MenuMetrics.w)
  .constraintSize({ maxHeight: MenuMetrics.row * MenuMetrics.maxRows })
}

/** The card a menu's rows sit on */
@ComponentV2
export struct MenuCard {
  @BuilderParam content: () => void;

  build() {
    Column() {
      Scroll() {
        Column() {
          this.content()
        }
        .width('100%')
      }
      .width('100%')
      .scrollBar(BarState.Off)
      .edgeEffect(EdgeEffect.None)
    }
    .menuCardLook()
  }
}
