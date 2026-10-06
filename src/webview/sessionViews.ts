import { applySession } from '@shared/reuse';
import { applySessionPatch, type SessionPatch } from '@shared/sessionPatch';
import type { SessionView } from '@shared/transcript';

// Sessions whose last view the page keeps after switching away. At least the host's SENT_VIEWS_MAX
// (rust/crates/acpira-shared/src/session_patch.rs): the host only patches against views it sent within its own bound, so
// every view it may patch against is still here. Fewer would cost a resync round trip and a whole view
export const VIEW_CACHE_ENTRIES = 8;

// What the page shows plus the last view of each recent session. Switching back to a kept session arrives as a patch
// against the kept view (only what changed since), not as the whole transcript again: over Remote-SSH a long session's
// view took seconds to cross, and every switch paid it
export class SessionViews {
  // Least recently shown first; the view on screen is in here too
  private views = new Map<string, SessionView>();
  current: SessionView | undefined;

  // `init`: the host starts its record of what the page holds over, so the page does too
  init(active: SessionView | undefined): SessionView | undefined {
    const cur = this.current;
    this.views.clear();
    this.current = undefined;
    return active ? this.show(cur?.id === active.id ? applySession(cur, active) : active) : undefined;
  }

  // A whole view. Another session's view is taken as it is: the host sends one whole only when the page holds nothing it
  // can patch (or a reopened instance whose revs count from 0 again), so the kept copy must not win on rev
  whole(next: SessionView): SessionView {
    const cur = this.current;
    return this.show(cur?.id === next.id ? applySession(cur, next) : next);
  }

  // The view to show, 'stale' for a patch older than the view on screen, or undefined when the page does not hold the view
  // the patch was computed against (the caller asks for the whole view; the kept copy is dropped as unknown)
  patch(p: SessionPatch): SessionView | 'stale' | undefined {
    const cur = this.current;
    const onScreen = cur?.id === p.id;
    const next = applySessionPatch(onScreen ? cur : this.views.get(p.id), p);
    if (next) return this.show(onScreen ? applySession(cur, next) : next);
    if (onScreen && cur.rev != null && cur.rev >= (p.view.rev ?? 0)) return 'stale';
    this.views.delete(p.id);
    return undefined;
  }

  // Ids kept, least recently shown first (tests)
  kept(): string[] {
    return [...this.views.keys()];
  }

  private show(view: SessionView): SessionView {
    this.views.delete(view.id);
    this.views.set(view.id, view);
    for (const id of this.views.keys()) {
      if (this.views.size <= VIEW_CACHE_ENTRIES) break;
      this.views.delete(id);
    }
    this.current = view;
    return view;
  }
}
