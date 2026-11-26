import { useId, type ComponentType } from 'react';

export interface ModeGlyphProps {
  className?: string;
  strokeWidth?: number;
}

// Any icon rendered on a mode chip, pill, dropdown row, or slash list row (solid mode glyphs and Lucide fallbacks alike).
export type ModeIcon = ComponentType<ModeGlyphProps>;

// Exact Lucide shield silhouette (x=4..20, y=2..22), filled solid so permission marks match the optical weight of solid vendor marks.
const SHIELD_D =
  'M20 13c0 5-3.5 7.5-7.66 8.95a1 1 0 0 1-.67-.01C7.5 20.5 4 18 4 13V6a1 1 0 0 1 1-1c2 0 4.5-1.2 6.24-2.72a1.17 1.17 0 0 1 1.52 0C14.51 3.81 17 5 19 5a1 1 0 0 1 1 1z';
const BUBBLE_D = 'M7.9 20A9 9 0 1 0 4 16.1L2 22Z';
const EYE_D =
  'M2.062 12.348a1 1 0 0 1 0-.696 10.75 10.75 0 0 1 19.876 0 1 1 0 0 1 0 .696 10.75 10.75 0 0 1-19.876 0';
const FOLDER_D =
  'M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z';

// Approval level: ask before every action (solid shield with negative ellipsis cutout).
export function AskShieldGlyph({ className }: ModeGlyphProps) {
  const id = useId();
  return (
    <svg aria-hidden="true" viewBox="0 0 24 24" width="24" height="24" fill="none" className={className}>
      <defs>
        <mask id={id}>
          <rect width="24" height="24" fill="white" />
          <circle cx="8.3" cy="12" r="1.45" fill="black" />
          <circle cx="12" cy="12" r="1.45" fill="black" />
          <circle cx="15.7" cy="12" r="1.45" fill="black" />
        </mask>
      </defs>
      <path d={SHIELD_D} fill="currentColor" mask={`url(#${id})`} />
    </svg>
  );
}

// Approval level: auto-edit (solid shield with negative checkmark cutout).
export function AutoEditShieldGlyph({ className }: ModeGlyphProps) {
  const id = useId();
  return (
    <svg aria-hidden="true" viewBox="0 0 24 24" width="24" height="24" fill="none" className={className}>
      <defs>
        <mask id={id}>
          <rect width="24" height="24" fill="white" />
          <path d="m8.8 12.2 2.2 2.2 4.4-4.4" stroke="black" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round" />
        </mask>
      </defs>
      <path d={SHIELD_D} fill="currentColor" mask={`url(#${id})`} />
    </svg>
  );
}

// Approval level: auto-approve / review (half-solid shield).
export function AutoShieldGlyph({ className }: ModeGlyphProps) {
  const id = useId();
  return (
    <svg aria-hidden="true" viewBox="0 0 24 24" width="24" height="24" fill="none" className={className}>
      <defs>
        <clipPath id={id}>
          <rect x="0" y="0" width="12" height="24" />
        </clipPath>
      </defs>
      <path d={SHIELD_D} fill="currentColor" opacity="0.32" />
      <path d={SHIELD_D} fill="currentColor" clipPath={`url(#${id})`} />
    </svg>
  );
}

// Approval level: full access / bypass / yolo (solid shield sliced by diagonal slash).
export function FullAccessShieldGlyph({ className }: ModeGlyphProps) {
  const id = useId();
  return (
    <svg aria-hidden="true" viewBox="0 0 24 24" width="24" height="24" fill="none" className={className}>
      <defs>
        <mask id={id}>
          <rect width="24" height="24" fill="white" />
          <path d="M2 2 L22 22" stroke="black" strokeWidth="5" strokeLinecap="round" />
        </mask>
      </defs>
      <path d={SHIELD_D} fill="currentColor" mask={`url(#${id})`} />
      <path d="M2.5 2.5 L21.5 21.5" stroke="currentColor" strokeWidth="2.1" strokeLinecap="round" />
    </svg>
  );
}

// Built-in Raw mode: tools unplugged, direct to model.
export function RawGlyph({ className }: ModeGlyphProps) {
  return (
    <svg aria-hidden="true" viewBox="0 0 24 24" width="24" height="24" fill="none" className={className}>
      <path d="m19 5 3-3M2 22l3-3M7.5 13.5 10 11M10.5 16.5 13 14" stroke="currentColor" strokeWidth="2.1" strokeLinecap="round" />
      <path d="M6.3 20.3a2.4 2.4 0 0 0 3.4 0L12 18l-6-6-2.3 2.3a2.4 2.4 0 0 0 0 3.4Z" fill="currentColor" stroke="currentColor" strokeWidth="1.2" strokeLinejoin="round" />
      <path d="m12 6 6 6 2.3-2.3a2.4 2.4 0 0 0 0-3.4l-2.6-2.6a2.4 2.4 0 0 0-3.4 0Z" fill="currentColor" stroke="currentColor" strokeWidth="1.2" strokeLinejoin="round" />
    </svg>
  );
}

// Plan / spec mode: checklist with solid checked badge and subtle unchecked box.
export function PlanGlyph({ className }: ModeGlyphProps) {
  const id = useId();
  return (
    <svg aria-hidden="true" viewBox="0 0 24 24" width="24" height="24" fill="none" className={className}>
      <defs>
        <mask id={id}>
          <rect width="24" height="24" fill="white" />
          <path d="m4.4 16.7 1.4 1.4 2.7-2.9" stroke="black" strokeWidth="1.9" strokeLinecap="round" strokeLinejoin="round" />
        </mask>
      </defs>
      <rect x="3" y="4" width="6.5" height="6.5" rx="1.6" fill="currentColor" opacity="0.38" />
      <rect x="3" y="13.5" width="6.5" height="6.5" rx="1.6" fill="currentColor" mask={`url(#${id})`} />
      <path d="M13 6.5h8M13 12h8M13 17.5h8" stroke="currentColor" strokeWidth="2.2" strokeLinecap="round" />
    </svg>
  );
}

// Team / multi-agent orchestration mode: layered solid figures.
export function TeamGlyph({ className }: ModeGlyphProps) {
  return (
    <svg aria-hidden="true" viewBox="0 0 24 24" width="24" height="24" fill="currentColor" className={className}>
      <circle cx="9" cy="7.5" r="3.8" />
      <path d="M2 19.5c0-3.3 3.1-5.5 7-5.5s7 2.2 7 5.5a1 1 0 0 1-1 1H3a1 1 0 0 1-1-1z" />
      <circle cx="16.8" cy="8.2" r="2.8" opacity="0.58" />
      <path d="M16.5 13.8c2.9.2 5.5 2 5.5 5.2a1 1 0 0 1-1 1h-3.6a7.2 7.2 0 0 0-1.9-5.3c.3-.3.6-.6 1-.9z" opacity="0.58" />
    </svg>
  );
}

// Code / build / edit mode: solid rounded badge with negative code chevrons.
export function CodeGlyph({ className }: ModeGlyphProps) {
  const id = useId();
  return (
    <svg aria-hidden="true" viewBox="0 0 24 24" width="24" height="24" fill="none" className={className}>
      <defs>
        <mask id={id}>
          <rect width="24" height="24" fill="white" />
          <path d="m9.6 9-3.1 3 3.1 3M14.4 9l3.1 3-3.1 3" stroke="black" strokeWidth="2.3" strokeLinecap="round" strokeLinejoin="round" />
        </mask>
      </defs>
      <rect x="2.5" y="4" width="19" height="16" rx="3.5" fill="currentColor" mask={`url(#${id})`} />
    </svg>
  );
}

// Smart / auto-review mode: solid four-point sparkle pair.
export function SmartGlyph({ className }: ModeGlyphProps) {
  return (
    <svg aria-hidden="true" viewBox="0 0 24 24" width="24" height="24" fill="currentColor" className={className}>
      <path d="M10.2 3.2c.3-1 1.7-1 2 0l1.4 4.8a2 2 0 0 0 1.4 1.4l4.8 1.4c1 .3 1 1.7 0 2l-4.8 1.4a2 2 0 0 0-1.4 1.4l-1.4 4.8c-.3 1-1.7 1-2 0l-1.4-4.8a2 2 0 0 0-1.4-1.4L2.6 12.8c-1-.3-1-1.7 0-2l4.8-1.4a2 2 0 0 0 1.4-1.4z" />
      <path d="M18.6 2.6c.15-.45.75-.45.9 0l.45 1.45c.1.3.3.5.6.6L22 5.1c.45.15.45.75 0 .9l-1.45.45a.9.9 0 0 0-.6.6L19.5 8.5c-.15.45-.75.45-.9 0l-.45-1.45a.9.9 0 0 0-.6-.6L16.1 6c-.45-.15-.45-.75 0-.9l1.45-.45a.9.9 0 0 0 .6-.6z" opacity="0.72" />
    </svg>
  );
}

// Workspace-only edits (Codex workspace-write, Hermes accept_edits): solid folder with carved pen badge.
export function WorkspaceGlyph({ className }: ModeGlyphProps) {
  const id = useId();
  return (
    <svg aria-hidden="true" viewBox="0 0 24 24" width="24" height="24" fill="none" className={className}>
      <defs>
        <mask id={id}>
          <rect width="24" height="24" fill="white" />
          <path d="M21.8 11.4a2.2 2.2 0 0 0-3.1 0l-6.4 6.4-1.1 4 4-1.1 6.4-6.4a2.2 2.2 0 0 0 .2-2.9Z" fill="black" stroke="black" strokeWidth="2.8" strokeLinejoin="round" />
        </mask>
      </defs>
      <path d={FOLDER_D} fill="currentColor" mask={`url(#${id})`} />
      <path d="M21.5 11.7a1.8 1.8 0 0 0-2.55 0l-5.85 5.85-.85 3.15 3.15-.85 5.85-5.85a1.8 1.8 0 0 0 .25-2.3Z" fill="currentColor" />
    </svg>
  );
}

// Read-only mode: solid eye silhouette with negative iris ring and solid pupil.
export function ReadOnlyGlyph({ className }: ModeGlyphProps) {
  const id = useId();
  return (
    <svg aria-hidden="true" viewBox="0 0 24 24" width="24" height="24" fill="none" className={className}>
      <defs>
        <mask id={id}>
          <rect width="24" height="24" fill="white" />
          <circle cx="12" cy="12" r="4.3" fill="black" />
          <circle cx="12" cy="12" r="2.2" fill="white" />
        </mask>
      </defs>
      <path d={EYE_D} fill="currentColor" mask={`url(#${id})`} />
    </svg>
  );
}

// Chat / ask / question mode: solid speech bubble with negative question mark.
export function ChatGlyph({ className }: ModeGlyphProps) {
  const id = useId();
  return (
    <svg aria-hidden="true" viewBox="0 0 24 24" width="24" height="24" fill="none" className={className}>
      <defs>
        <mask id={id}>
          <rect width="24" height="24" fill="white" />
          <path d="M9.3 9.2a2.8 2.8 0 0 1 5.4.9c0 1.8-2.7 2.5-2.7 3.7" stroke="black" strokeWidth="2.3" strokeLinecap="round" />
          <circle cx="12" cy="17.1" r="1.35" fill="black" />
        </mask>
      </defs>
      <path d={BUBBLE_D} fill="currentColor" mask={`url(#${id})`} />
    </svg>
  );
}

// Debug mode: solid beetle body with negative wing seam.
export function DebugGlyph({ className }: ModeGlyphProps) {
  const id = useId();
  return (
    <svg aria-hidden="true" viewBox="0 0 24 24" width="24" height="24" fill="none" className={className}>
      <defs>
        <mask id={id}>
          <rect width="24" height="24" fill="white" />
          <path d="M12 11v9" stroke="black" strokeWidth="1.9" strokeLinecap="round" />
        </mask>
      </defs>
      <path d="m8 2 1.9 1.9M16 2l-1.9 1.9M6 13H2M22 13h-4M6 17l-3 2M18 17l3 2M6.5 9 3.5 7.5M17.5 9l3-1.5" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
      <path d="M9 7.1V6a3 3 0 0 1 6 0v1.1" fill="currentColor" />
      <rect x="6.5" y="7" width="11" height="13.5" rx="5.5" fill="currentColor" mask={`url(#${id})`} />
    </svg>
  );
}

// Default / agent / normal mode: weighted infinity loop with soft inner tint.
export function AgentGlyph({ className }: ModeGlyphProps) {
  const id = useId();
  return (
    <svg aria-hidden="true" viewBox="0 0 24 24" width="24" height="24" fill="none" className={className}>
      <defs>
        <mask id={id}>
          <rect width="24" height="24" fill="white" />
          <path d="M9.8 14.2 14.2 9.8" stroke="black" strokeWidth="4.2" />
        </mask>
      </defs>
      <path d="M12 12c-2-2.67-4-4-6-4a4 4 0 1 0 0 8c2 0 4-1.33 6-4Zm0 0c2 2.67 4 4 6 4a4 4 0 0 0 0-8c-2 0-4 1.33-6 4Z"
        stroke="currentColor" strokeWidth="2.65" strokeLinecap="round" strokeLinejoin="round" />
      <path d="M12 12c-2-2.67-4-4-6-4a4 4 0 1 0 0 8c2 0 4-1.33 6-4Z" fill="currentColor" opacity="0.24" mask={`url(#${id})`} />
      <path d="M12 12c2 2.67 4 4 6 4a4 4 0 0 0 0-8c-2 0-4 1.33-6 4Z" fill="currentColor" opacity="0.24" />
    </svg>
  );
}

// Fallback glyph for custom / unrecognized ACP agent modes.
export function CustomModeGlyph({ className }: ModeGlyphProps) {
  return (
    <svg aria-hidden="true" viewBox="0 0 24 24" width="24" height="24" fill="none" className={className}>
      <circle cx="12" cy="12" r="7.5" fill="currentColor" opacity="0.24" />
      <circle cx="12" cy="12" r="9.2" stroke="currentColor" strokeWidth="2.3" strokeDasharray="4.2 3.2" strokeLinecap="round" />
    </svg>
  );
}
