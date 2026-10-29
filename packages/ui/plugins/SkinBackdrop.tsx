import { cn } from '../ui/cn';
import { SandboxFrame } from './SandboxFrame';
import { useReducedMotion, useWindowFocused } from './motion';
import { backdropDrawn, backdropsShown, skinKey, useSkin } from './skin';

// The skin's backdrop, a fixed layer behind the whole page (styles/skin.css), mounted by App beside whichever shell is up
// so a visit to the settings page neither reloads it nor restarts its clock; the settings page is opaque over it and it
// holds still meanwhile. It holds: the shown backdrop; the incoming one, laid out unseen until its first frame; and,
// during a reveal without View Transitions, the leaving one, which the circle uncovers or covers. Each keeps its frame as
// it changes role, so the scene the reveal uncovers is the one that was drawn
export function SkinBackdrop({ paused, motion }: { paused: boolean; motion: 'on' | 'none' }) {
  const { shown, incoming, leaving } = useSkin();
  const focused = useWindowFocused();
  const reduced = useReducedMotion();
  if (!backdropsShown) return null;
  const list = [leaving?.skin, shown, incoming].filter((s, i, all) => s?.backdrop && all.findIndex(o => o && skinKey(o) === skinKey(s)) === i);
  if (!list.length) return null;
  const still = motion === 'none' || reduced;
  return (
    <div data-skin-backdrop aria-hidden="true">
      {list.map(s => {
        const key = skinKey(s!);
        const visible = (shown !== undefined && skinKey(shown) === key) || (leaving !== undefined && skinKey(leaving.skin) === key);
        // The leaving scene lies over the new one when the circle closes on it (undo), under it when the circle opens
        const over = leaving !== undefined && skinKey(leaving.skin) === key && leaving.over;
        return (
          <SandboxFrame key={key} plugin={s!.plugin} piece={`backdrop:${s!.snapshot}`} html={s!.backdrop} title={s!.name}
            options={{ run: !still }} run={!still && (visible ? focused && !paused : true)} onDrawn={() => backdropDrawn(key)}
            data-skin-frame={key}
            // Unseen, not hidden: a frame out of the layout or invisible may not get animation frames
            className={cn('absolute inset-0 size-full', !visible && 'opacity-0', over && 'z-1')} />
        );
      })}
    </div>
  );
}
