// The usage page's mark in the desktop column foot (chat/DesktopFrame.tsx AppMenu): three columns, low, tallest and
// middling, joined in one rounded outline with the seams drawn in, drawn as the foot's other glyphs are (the gear, the
// devices row's phone in remote/DevicesCard.tsx): a 24-unit box, a 1.5 stroke, round caps and joins. Column tops nearly
// round like the page's own bars, inner corners eased. It stands as tall as the phone (2.5 to 21.5, so the two share a
// top and a bottom) and a unit left of centre: the taller right column made it read as leaning on the phone after it
export function UsageGlyph({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 24 24" className={className} fill="none" stroke="currentColor" strokeWidth={1.5} strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <path d="M1.5 19v-5.5a3 3 0 0 1 3-3h2.25a1 1 0 0 0 1-1v-4a3 3 0 0 1 3-3h.5a3 3 0 0 1 3 3v.5a1 1 0 0 0 1 1h2.25a3 3 0 0 1 3 3v9a2.5 2.5 0 0 1-2.5 2.5h-14A2.5 2.5 0 0 1 1.5 19Z" />
      <path d="M7.75 9.5v12" />
      <path d="M14.25 6v15.5" />
    </svg>
  );
}
