import type { ComponentProps } from 'react';
import { Switch as Base } from '@base-ui/react/switch';
import { cn, cnState } from './cn';

// The menu row and the settings toggle share behavior, with distinct geometry.
// The menu skin is a bare row; SwitchRow draws its own track. The settings skin paints
// the track here and the knob on ::after, monochrome like the send button's lit state.
const skins = {
  menu: 'flex min-h-row w-full items-center gap-2 rounded-md px-2 text-left text-2 text-fg-1 outline-none transition-colors hover:bg-hover focus-visible:bg-hover disabled:text-fg-3 disabled:hover:bg-transparent',
  settings: cn(
    'relative inline-flex h-switch-lg-h w-switch-lg-w shrink-0 rounded-full bg-hover shadow-[inset_0_0_0_1px_var(--line-strong)] transition-[background,box-shadow,filter]',
    'after:absolute after:top-switch-lg-pad after:left-switch-lg-pad after:size-switch-lg-knob after:rounded-full after:bg-fg-3 after:transition-[translate,background]',
    'aria-checked:bg-btn-1 aria-checked:shadow-none aria-checked:after:translate-x-switch-lg-on aria-checked:after:bg-btn-1-fg',
    'enabled:hover:shadow-[inset_0_0_0_1px_var(--fg-3)] aria-checked:enabled:hover:shadow-none aria-checked:enabled:hover:brightness-[.92]',
    // Outer focus ring replaces any hover ring; the important flag mirrors the hover/checked precedence.
    'focus-visible:shadow-[0_0_0_2px_var(--bg-0),0_0_0_3px_var(--fg-3)]!',
    'disabled:cursor-not-allowed disabled:opacity-40',
  ),
  // Row-sized toggle beside a list entry (the settings rail): the menu track geometry, painted like the settings skin
  compact: cn(
    'relative inline-flex h-switch-track-h w-switch-track-w shrink-0 rounded-full bg-active transition-[background,filter]',
    'after:absolute after:top-switch-off after:left-0 after:size-switch-thumb after:translate-x-switch-off after:rounded-full after:bg-fg-3 after:transition-[translate,background]',
    'aria-checked:bg-btn-1 aria-checked:after:translate-x-switch-on aria-checked:after:bg-btn-1-fg aria-checked:enabled:hover:brightness-[.92]',
    'outline-none focus-visible:shadow-[0_0_0_2px_var(--bg-0),0_0_0_3px_var(--fg-3)]',
    'disabled:cursor-not-allowed disabled:opacity-40',
  ),
};

export function Switch({ skin = 'settings', className, ...props }: ComponentProps<typeof Base.Root> & { skin?: keyof typeof skins }) {
  return <Base.Root render={<button type="button" />} nativeButton {...props} className={cnState(skins[skin], className)} />;
}
