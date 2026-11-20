// Registered CSS custom properties, registered once through JS instead of CSS `@property` rules.
// Chromium < 138.0.7204.49 (CVE-2025-6555 / Issue 407328533) frees `@property`-declared PropertyRegistration objects
// during StyleEngine::ClearPropertyRules() (stylesheet additions, media query changes) without invalidating active
// TransitionInterpolation objects, causing a renderer STATUS_ACCESS_VIOLATION (0xC0000005) on the next style recalc.
// `CSS.registerProperty()` lives in Blink's separate registered map, is never cleared by ClearPropertyRules(), and
// takes precedence over any `@property` rule of the same name (including Tailwind v4's `--tw-gradient-*` stops, which
// appear in Tailwind's `transition` / `transition-colors` property lists).

export interface CssPropertyDef {
  name: string;
  syntax: string;
  inherits: boolean;
  initialValue: string;
}

export const CSS_PROPERTIES: readonly CssPropertyDef[] = [
  // Overlay scrollbars (styles/base.css): transitioned on scrollers, inherited by ::-webkit-scrollbar-thumb
  { name: '--scrollbar-thumb', syntax: '<color>', inherits: false, initialValue: 'transparent' },
  { name: '--scrollbar-inset', syntax: '<length>', inherits: false, initialValue: '3.5px' },

  // File tree row hover / selection tint (styles/panel.css): transitioned as a colour
  { name: '--tint', syntax: '<color>', inherits: false, initialValue: 'transparent' },

  // Transcript bottom-follow pad and edge fades (styles/chat.css): non-inherited so changes restyle one element
  { name: '--follow-lead', syntax: '<length>', inherits: false, initialValue: '0px' },
  { name: '--fade-top', syntax: '<length>', inherits: false, initialValue: '0px' },
  { name: '--fade-bottom', syntax: '<length>', inherits: false, initialValue: '0px' },
  { name: '--thread-fade-clear', syntax: '<length>', inherits: false, initialValue: '0px' },
  { name: '--thread-fade-span', syntax: '<length>', inherits: false, initialValue: '0px' },
  { name: '--thread-tail-room', syntax: '<length>', inherits: false, initialValue: '0px' },

  // Composer focus beam (styles/beam.css): keyframe-animated custom properties registered once
  { name: '--bb-fade', syntax: '<number>', inherits: true, initialValue: '0' },
  { name: '--bb-angle', syntax: '<angle>', inherits: true, initialValue: '0deg' },
  { name: '--bb-x', syntax: '<number>', inherits: true, initialValue: '0' },
  { name: '--bb-w', syntax: '<number>', inherits: true, initialValue: '1' },
  { name: '--bb-h', syntax: '<number>', inherits: true, initialValue: '1' },
  { name: '--bb-spike', syntax: '<number>', inherits: true, initialValue: '1' },
  { name: '--bb-spike2', syntax: '<number>', inherits: true, initialValue: '1' },
  { name: '--bb-edge', syntax: '<number>', inherits: true, initialValue: '1' },
  { name: '--bb-swing', syntax: '<number>', inherits: false, initialValue: '0' },
  { name: '--bb-hue', syntax: '<angle>', inherits: true, initialValue: '0deg' },
  ...Array.from({ length: 17 }, (_, i): CssPropertyDef => ({
    name: `--bb-o${i}`,
    syntax: '<number>',
    inherits: true,
    initialValue: '0',
  })),

  // Shadow Tailwind v4's @property gradient stops included in `transition` / `transition-colors`
  { name: '--tw-gradient-from', syntax: '<color>', inherits: false, initialValue: '#0000' },
  { name: '--tw-gradient-via', syntax: '<color>', inherits: false, initialValue: '#0000' },
  { name: '--tw-gradient-to', syntax: '<color>', inherits: false, initialValue: '#0000' },
];

type RegisterFn = (def: CssPropertyDef) => void;
let globalRegistered = false;

export function registerCssProperties(register?: RegisterFn): void {
  if (!register) {
    if (globalRegistered) return;
    const css = globalThis.CSS as { registerProperty?: RegisterFn } | undefined;
    if (typeof css?.registerProperty !== 'function') return;
    globalRegistered = true;
    register = def => css.registerProperty!(def);
  }
  for (const def of CSS_PROPERTIES) {
    try {
      register(def);
    } catch {
      // Ignore InvalidModificationError (already registered, e.g. HMR) and unsupported syntax errors
    }
  }
}
