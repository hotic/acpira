// `@acpira/plugin-ui`: what a plugin's interface piece imports inside its sandboxed frame (docs/dev/plugins.md,
// Interface). The page sends this module with React into the frame (`lib.ts`), so a piece never bundles either. The
// components draw with the frame kit's classes (packages/ui/plugins/kit.ts), so a piece reads as part of Acpira and
// follows the theme; the hooks wrap the frame's `acpira` bridge (its call, posts, session state and outline).
// Exported names are a public contract: the scaffold's type declarations list them (rust/.../plugins/sdk.rs), and a
// test keeps the two equal.

import { Component, createElement, useEffect, useState, type ButtonHTMLAttributes, type ComponentType, type HTMLAttributes, type InputHTMLAttributes, type ReactNode } from 'react';
import { createRoot } from 'react-dom/client';

// The frame's bridge (rust/crates/acpira-desktop/src/page.rs, PLUGIN_FRAME_HTML)
interface Bridge {
  call(method: string, params?: unknown): Promise<unknown>;
  on(fn: (message: unknown) => void): () => void;
  onState(fn: (state: SessionState) => void): () => void;
  onSession(fn: (session: SessionOutline | null) => void): () => void;
  scrollTo(turn: number): void;
  options: Record<string, unknown>;
  state?: SessionState;
  session?: SessionOutline | null;
}

export interface SessionState {
  working: boolean;
  waiting: boolean;
}

export interface ExchangeOutline {
  turn: number;
  edits: { files: number; add: number; del: number };
  runs: number;
  reads: number;
  tools: number;
  failed: number;
}

export interface SessionOutline {
  session: string;
  running: boolean;
  exchanges: ExchangeOutline[];
  seen: [number, number];
}

const bridge = (): Bridge | undefined => (globalThis as { acpira?: Bridge }).acpira;

/** Call a method of this plugin's process (`methods` in plugin.json) */
export function call<T = unknown>(method: string, params?: unknown): Promise<T> {
  const b = bridge();
  if (!b) return Promise.reject(new Error('not inside an Acpira plugin frame'));
  return b.call(method, params) as Promise<T>;
}

/** `rpc.ping(params)` is `call('ping', params)` */
export const rpc: Record<string, <T = unknown>(params?: unknown) => Promise<T>> = new Proxy({}, {
  get: (_, method) => (typeof method === 'string' ? (params?: unknown) => call(method, params) : undefined),
});

/** Hear what the process posts to its frames (`ctx.ui.post`); returns the unsubscribe */
export function onMessage<T = unknown>(fn: (message: T) => void): () => void {
  return bridge()?.on(fn as (m: unknown) => void) ?? (() => {});
}

/** The last message the process posted, re-rendering on each */
export function useMessage<T = unknown>(): T | undefined {
  const [last, setLast] = useState<T>();
  useEffect(() => onMessage<T>(m => setLast(() => m)), []);
  return last;
}

/** Whether the session on screen is working, or waiting for the user (nothing of its content) */
export function useSessionState(): SessionState {
  const [state, setState] = useState<SessionState>(() => bridge()?.state ?? { working: false, waiting: false });
  useEffect(() => bridge()?.onState(setState), []);
  return state;
}

/** The conversation's shape per exchange (counts, never text, paths or commands); null without a session on screen */
export function useSession(): SessionOutline | null {
  const [session, setSession] = useState<SessionOutline | null>(() => bridge()?.session ?? null);
  useEffect(() => bridge()?.onSession(setSession), []);
  return session;
}

/** What the page set for this frame */
export function useOptions<T extends Record<string, unknown> = Record<string, unknown>>(): T {
  return (bridge()?.options ?? {}) as T;
}

/** Bring an exchange of the conversation into view */
export function scrollTo(turn: number): void {
  bridge()?.scrollTo(turn);
}

const cls = (...names: (string | false | undefined)[]) => names.filter(Boolean).join(' ');

export type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & { variant?: 'default' | 'primary' | 'icon' };

export function Button({ variant = 'default', className, type = 'button', ...rest }: ButtonProps) {
  return <button type={type} className={cls('a-btn', variant !== 'default' && variant, className)} {...rest} />;
}

export type ChipProps = ButtonHTMLAttributes<HTMLButtonElement> & { on?: boolean };

export function Chip({ on, className, type = 'button', ...rest }: ChipProps) {
  return <button type={type} className={cls('a-chip', on && 'on', className)} aria-pressed={on} {...rest} />;
}

export type RowProps = HTMLAttributes<HTMLDivElement> & { onClick?: () => void };

/** A row like Acpira's lists; clickable when it has `onClick` */
export function Row({ className, onClick, ...rest }: RowProps) {
  if (onClick) return <button type="button" className={cls('a-row', className)} onClick={onClick} {...(rest as ButtonHTMLAttributes<HTMLButtonElement>)} />;
  return <div className={cls('a-row', className)} {...rest} />;
}

export function Card({ className, ...rest }: HTMLAttributes<HTMLDivElement>) {
  return <div className={cls('a-card', className)} {...rest} />;
}

export function Input({ className, ...rest }: InputHTMLAttributes<HTMLInputElement>) {
  return <input className={cls('a-input', className)} {...rest} />;
}

/** Children in a column, or a line with `row` */
export function Stack({ row, className, ...rest }: HTMLAttributes<HTMLDivElement> & { row?: boolean }) {
  return <div className={cls(row ? 'a-line' : 'a-stack', className)} {...rest} />;
}

export function Title({ className, ...rest }: HTMLAttributes<HTMLHeadingElement>) {
  return <h3 className={cls('a-title', className)} {...rest} />;
}

/** Secondary text: `mono` for code, `truncate` for one line with an ellipsis */
export function Text({ muted, mono, truncate, className, ...rest }: HTMLAttributes<HTMLSpanElement> & { muted?: boolean; mono?: boolean; truncate?: boolean }) {
  return <span className={cls(muted && 'a-muted', mono && 'a-mono', truncate && 'a-truncate', className)} {...rest} />;
}

export function Rule() {
  return <hr className="a-rule" />;
}

export function Dot({ tone }: { tone?: 'ok' | 'warn' | 'danger' | 'brand' }) {
  return <span className={cls('a-dot', tone)} aria-hidden />;
}

// A piece that throws shows its message in place instead of a blank frame
class Boundary extends Component<{ children?: ReactNode }, { error?: Error }> {
  override state: { error?: Error } = {};

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  override render() {
    if (this.state.error) return <div className="a-muted" data-error="">{this.state.error.message}</div>;
    return this.props.children;
  }
}

/** Render a piece's component into the frame (the engine's loader calls this with the entry's default export) */
export function mount(Piece: ComponentType) {
  const host = document.getElementById('root') ?? document.body.appendChild(document.createElement('div'));
  createRoot(host).render(<Boundary>{createElement(Piece)}</Boundary>);
}
