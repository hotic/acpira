import { ListMetrics, Metrics } from '../design/Tokens';
import { Lucide } from './Glyphs';

// The desktop list's state marks (packages/ui chat/SessionList.tsx StateMark, motion in styles/motion.css; apps/ios
// SessionListScreen StateMark), one slot each so they share a centre line. Working: the spinning ring, which keeps
// turning with motion off. Waiting: an amber dot beating like a heart. Error: a red dot that double-flashes every few
// seconds. Unread: a dot with a halo rippling out of it. Background: the ring with a dot orbiting it slowly, lit with a
// dot inside once a reply came in meanwhile. Motion off stops all but the ring on their resting frame.

/** Lucide's circle: r 10 in a 24 unit box, where the orbiting dot rides */
const ORBIT_R: number = 10;
const FULL_TURN: number = 360;
/** acp-mark-flash's dim, the halo's growth and starting alpha */
const FLASH_LOW: number = 0.2;
const HALO_GROWTH: number = 2.6;
const HALO_ALPHA: number = 0.6;

@ComponentV2
export struct StateMark {
  @Param state: string = '';
  @Param unread: boolean = false;
  @Param motion: boolean = true;
  @Local turn: number = 0;
  @Local beat: number = 1;
  @Local flash: number = 1;
  @Local halo: number = 0;

  aboutToAppear(): void {
    // The next frame, so each loop starts from its resting value
    setTimeout(() => this.play(), 0);
  }

  play(): void {
    const ui = this.getUIContext();
    if (this.state === 'working' || (this.motion && this.state === 'background')) {
      const period = this.state === 'working' ? ListMetrics.spinMs : ListMetrics.orbitMs;
      ui.animateTo({ duration: period, curve: Curve.Linear, iterations: -1 }, () => {
        this.turn = FULL_TURN;
      });
    }
    if (!this.motion) {
      return;
    }
    if (this.state === 'waiting') {
      // acp-mark-beat: 0 / 100% 1, 20% 1.5, 40% 1.1, 55% 1.35, 75% 1
      const d = ListMetrics.beatMs;
      ui.keyframeAnimateTo({ iterations: -1 }, [
        { duration: d * 0.2, curve: Curve.EaseInOut, event: () => { this.beat = 1.5; } },
        { duration: d * 0.2, curve: Curve.EaseInOut, event: () => { this.beat = 1.1; } },
        { duration: d * 0.15, curve: Curve.EaseInOut, event: () => { this.beat = 1.35; } },
        { duration: d * 0.2, curve: Curve.EaseInOut, event: () => { this.beat = 1; } },
        { duration: d * 0.25, curve: Curve.Linear, event: () => { this.beat = 1; } }
      ]);
    } else if (this.state === 'error') {
      // acp-mark-flash: lit, then two dips to 0.2 at 70% and 82%
      const d = ListMetrics.flashMs;
      ui.keyframeAnimateTo({ iterations: -1 }, [
        { duration: d * 0.64, curve: Curve.Linear, event: () => { this.flash = 1; } },
        { duration: d * 0.06, curve: Curve.Linear, event: () => { this.flash = FLASH_LOW; } },
        { duration: d * 0.06, curve: Curve.Linear, event: () => { this.flash = 1; } },
        { duration: d * 0.06, curve: Curve.Linear, event: () => { this.flash = FLASH_LOW; } },
        { duration: d * 0.06, curve: Curve.Linear, event: () => { this.flash = 1; } },
        { duration: d * 0.12, curve: Curve.Linear, event: () => { this.flash = 1; } }
      ]);
    } else if (this.state === 'unread') {
      ui.animateTo({ duration: ListMetrics.haloMs, curve: Curve.EaseOut, iterations: -1 }, () => {
        this.halo = 1;
      });
    }
  }

  build() {
    Stack() {
      if (this.state === 'working') {
        Lucide({ name: 'loaderCircle', tint: $r('app.color.fg2'), extent: ListMetrics.stateRing })
          .rotate({ angle: this.turn })
      } else if (this.state === 'waiting') {
        Circle()
          .width(ListMetrics.stateDot)
          .height(ListMetrics.stateDot)
          .fill($r('app.color.warn'))
          .scale({ x: this.beat, y: this.beat })
      } else if (this.state === 'error') {
        Circle()
          .width(ListMetrics.stateDot)
          .height(ListMetrics.stateDot)
          .fill($r('app.color.danger'))
          .opacity(this.flash)
      } else if (this.state === 'unread') {
        if (this.motion) {
          Circle()
            .width(ListMetrics.stateUnread)
            .height(ListMetrics.stateUnread)
            .fill($r('app.color.fg_strong'))
            .scale({ x: 1 + (HALO_GROWTH - 1) * this.halo, y: 1 + (HALO_GROWTH - 1) * this.halo })
            .opacity(HALO_ALPHA * (1 - this.halo))
        }
        Circle()
          .width(ListMetrics.stateUnread)
          .height(ListMetrics.stateUnread)
          .fill($r('app.color.fg_strong'))
      } else if (this.state === 'background') {
        Lucide({ name: 'circle', tint: this.unread ? $r('app.color.fg_strong') : $r('app.color.fg3'), extent: ListMetrics.stateRing })
        // The orbit rides the ring's own line
        Stack({ alignContent: Alignment.Top }) {
          Circle()
            .width(ListMetrics.stateOrbit)
            .height(ListMetrics.stateOrbit)
            .fill(this.unread ? $r('app.color.fg_strong') : $r('app.color.fg2'))
            .margin({ top: ListMetrics.stateRing / 2 - ListMetrics.stateRing * ORBIT_R / Metrics.iconBox - ListMetrics.stateOrbit / 2 })
        }
        .width(ListMetrics.stateRing)
        .height(ListMetrics.stateRing)
        .rotate({ angle: this.turn })
        if (this.unread) {
          Circle()
            .width(ListMetrics.stateInner)
            .height(ListMetrics.stateInner)
            .fill($r('app.color.fg_strong'))
        }
      }
    }
    .width(ListMetrics.stateSlot)
    .height(ListMetrics.stateSlot)
  }
}
