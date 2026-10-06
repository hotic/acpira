import { Inbox, Plus } from 'lucide-react';

// Glyphs of the session list's user categories. A project is a real directory and keeps the folder glyph; a category
// is a user-made group inside one, so it gets the inbox (picked from twelve candidates in lab/session-folders, 2026-10-06)

// "New category": the inbox with a small plus at its top-right corner (lucide has no plus variant of the inbox)
export function CategoryAddIcon() {
  return (
    <span className="relative flex size-icon items-center justify-center" aria-hidden>
      <Inbox className="size-icon" strokeWidth={1.5} />
      <Plus className="absolute -top-1 -right-1 size-2.5" strokeWidth={2.5} />
    </span>
  );
}
