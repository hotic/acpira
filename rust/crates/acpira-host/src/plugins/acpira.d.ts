// Types for an Acpira plugin written in TypeScript. `acpira plugin new --lang ts` writes this file beside the code;
// the engine compiles the plugin itself (types are stripped, never checked), so the file only serves the editor.
// For React's own types run `npm i -D @types/react` in the folder; nothing else is installed.
//
// `@acpira/plugin` is what `runtime.main` imports: it default-exports `definePlugin({...})`, and every name in it is
// also declared in plugin.json (hooks, events, methods, tools, schedules, commands), the list the user confirmed.
// `@acpira/plugin-ui` is what an interface piece (`ui[].entry`) imports: it default-exports a React component, which
// runs in a sandboxed frame with React and these components provided by Acpira.

declare module '@acpira/plugin' {
  export interface PluginInfo {
    id: string;
    version: string;
    snapshot: string;
    // The frozen copy the process runs from, read-only
    dir: string;
    // A folder of its own that survives upgrades
    data: string;
  }

  export interface Storage {
    get<T = unknown>(key: string): Promise<T | null>;
    set(key: string, value: unknown): Promise<unknown>;
    delete(key: string): Promise<unknown>;
    list(prefix?: string): Promise<string[]>;
  }

  // An account `accounts.set` lists: picked in the agent's account menu like the user's own, the agent then starts with
  // `env` on top of its definition's (for Antigravity, ACPIRA_ANTIGRAVITY_GEMINI_DIR names the gemini dir holding the login).
  // `id` is the plugin's own; `badge` is the tag beside the label; `quota` the bars, `remaining` a share from 0 to 1
  export interface PluginAccount {
    id: string;
    agent: string;
    label: string;
    badge?: string;
    env?: Record<string, string>;
    quota?: { windows: { id: string; remaining: number; resetsAt?: string }[] };
  }

  // Another plugin's methods: `client.rate({ from: 'EUR' })` calls its `rate` method
  export type Client = Record<string, <T = any>(params?: unknown) => Promise<T>>;

  export interface Context {
    plugin: PluginInfo;
    engine: { version?: string; [key: string]: unknown };
    // Aborted when the process closes (or, inside a handler, when the call is cancelled)
    signal: AbortSignal;
    log(message: unknown): void;
    storage: Storage;
    session: {
      list(cwd?: string): Promise<any[]>;
      get(session: string, turns?: number): Promise<any>;
      act(message: Record<string, unknown>): Promise<any>;
      // Send a prompt into a session, as the user would
      send(sessionId: string, text: string): Promise<any>;
    };
    ui: {
      toast(text: string, level?: 'info' | 'warn' | 'error'): Promise<unknown>;
      ask(params: Record<string, unknown>): Promise<any>;
      // To this plugin's interface pieces on every page (`useMessage` there)
      post(message: unknown): Promise<unknown>;
      composer(text: string, replace?: boolean, session?: string): Promise<unknown>;
    };
    // One call through the built-in agent's models: `{ prompt, system, model, maxTokens, images: [{ mimeType, data }] }`,
    // answered `{ text, model, usage }`
    model: { generate(params: string | Record<string, unknown>): Promise<any> };
    providers: { set(providers: unknown[]): Promise<unknown> };
    // Accounts this plugin lists beside the user's own, for agents Acpira keeps accounts for; each call replaces the last list
    accounts: { set(accounts: PluginAccount[]): Promise<{ accounts: number }> };
    request<T = any>(method: string, params?: Record<string, unknown>): Promise<T>;
    plugins: {
      // A plugin of `optionalDependencies`: undefined when it is not in effect
      get<T = Client>(id: string): T | undefined;
      // One of `dependencies`: always in effect while this plugin runs
      require<T = Client>(id: string): T;
    };
  }

  export interface PromptPayload {
    origin: 'user' | 'goal';
    text: string;
    attachments: unknown[];
    first: boolean;
    command?: string;
    agent?: string;
    session?: string;
    cwd?: string;
    // Every hook's payload carries it while a plugin command has a menu: the session's picks, by command
    menus?: Record<string, MenuPicks>;
    [key: string]: unknown;
  }

  // Fields to change (`text`, `context`), `block` with a reason, `redirect` to another agent, or null for no opinion
  export interface PromptAnswer {
    text?: string;
    context?: string;
    block?: string;
    redirect?: string;
  }

  export interface AcpiraEvent {
    type: string;
    data?: any;
    [key: string]: unknown;
  }

  // A command menu's state: each radio group's pick (by group name) and each toggle (by its id)
  export type MenuPicks = Record<string, string | boolean>;

  // One of the draft's attachments, as the composer holds it: a pasted image is base64 `data`
  export type Attachment =
    | { kind: 'image'; mimeType: string; data: string; name?: string }
    | { kind: 'text'; name: string; text: string }
    | { kind: 'file'; uri: string; name: string }
    | { kind: 'selection'; uri: string; name: string; startLine: number; endLine: number; text: string }
    | { kind: 'quote'; text: string; comment?: string };

  // What a `message.actions` button's method may answer to fill the popover its click opened: this Markdown, `title`
  // dim beside the button's title. Any other answer closes the popover; each opening calls the method again
  export interface NoteAnswer {
    note: string | { markdown: string; title?: string };
  }

  type Maybe<T> = T | null | undefined | void | Promise<T | null | undefined | void>;
  type Handler<P = any, R = unknown> = (payload: P, ctx: Context) => Maybe<R>;

  export interface PluginDefinition {
    // Runs before the plugin is ready; what it returns runs when the process closes
    activate?(ctx: Context): Maybe<() => unknown>;
    // The version stored last differs from this one: move the data along (the first run has nothing to migrate)
    migrate?(from: string, to: string, ctx: Context): Maybe<unknown>;
    // `prompt`, `permission`, `retry`, `title`, `tool.before`, `tool.after`, ...
    hooks?: { prompt?: Handler<PromptPayload, PromptAnswer> } & Record<string, Handler>;
    // An exact name (`turn.ended`), a family (`tool.*`) or `*`
    events?: Record<string, (event: AcpiraEvent, ctx: Context) => unknown>;
    // Called by interface pieces (`rpc.name()`) and by plugins that depend on this one
    methods?: Record<string, Handler>;
    // Tools of plugin.json without a `command`: the model's arguments in, the text it reads out
    tools?: Record<string, Handler<any, string | object>>;
    // Schedules of plugin.json: `due` is the occurrence in milliseconds (a missed run catches up once)
    schedules?: Record<string, (ctx: Context & { due: number }) => unknown>;
    // Commands of plugin.json without a `prompt`: the prompt to send in their place, or nothing
    // `menus`: every command button menu's picks in the session, by command; `picks` in an answer moves this command's.
    // `attachments`: what the draft holds when its button runs it (`draft`), images as `{ kind: 'image', mimeType, data }`.
    // `session` here is the session id, so it hides the session API: `ctx.request('session/get', { session })` reads it.
    // A method reads a command menu's picks from the checked rows of that view's `commands`
    commands?: Record<string, (args: string, ctx: Context & { draft: boolean; attachments: Attachment[]; session?: string; menus: Record<string, MenuPicks> }) => Maybe<string | { text?: string; picks?: MenuPicks }>>;
    // Command buttons with a `menu`: a row was picked (`item`, its id), `picks` already holds the result; answer
    // `{ picks }` to change them (an `action` row keeps nothing otherwise)
    menus?: Record<string, (item: string, ctx: Context & { session: string; picks: MenuPicks }) => Maybe<{ picks?: MenuPicks }>>;
  }

  export function definePlugin<T extends PluginDefinition>(definition: T): T;
}

declare module '@acpira/plugin-ui' {
  import type { ButtonHTMLAttributes, ComponentType, HTMLAttributes, InputHTMLAttributes, JSX } from 'react';

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

  // A method of this plugin's process (`methods` in plugin.json)
  export function call<T = unknown>(method: string, params?: unknown): Promise<T>;
  // `rpc.ping(params)` is `call('ping', params)`
  export const rpc: Record<string, <T = unknown>(params?: unknown) => Promise<T>>;
  // What the process posts (`ctx.ui.post`); returns the unsubscribe
  export function onMessage<T = unknown>(fn: (message: T) => void): () => void;
  export function useMessage<T = unknown>(): T | undefined;
  export function useSessionState(): SessionState;
  // Counts per exchange, never text; null without a session on screen
  export function useSession(): SessionOutline | null;
  export function useOptions<T extends Record<string, unknown> = Record<string, unknown>>(): T;
  export function scrollTo(turn: number): void;

  export type ButtonProps = ButtonHTMLAttributes<HTMLButtonElement> & { variant?: 'default' | 'primary' | 'icon' };
  export function Button(props: ButtonProps): JSX.Element;
  export type ChipProps = ButtonHTMLAttributes<HTMLButtonElement> & { on?: boolean };
  export function Chip(props: ChipProps): JSX.Element;
  export type RowProps = HTMLAttributes<HTMLDivElement> & { onClick?: () => void };
  export function Row(props: RowProps): JSX.Element;
  export function Card(props: HTMLAttributes<HTMLDivElement>): JSX.Element;
  export function Input(props: InputHTMLAttributes<HTMLInputElement>): JSX.Element;
  export function Stack(props: HTMLAttributes<HTMLDivElement> & { row?: boolean }): JSX.Element;
  export function Title(props: HTMLAttributes<HTMLHeadingElement>): JSX.Element;
  export function Text(props: HTMLAttributes<HTMLSpanElement> & { muted?: boolean; mono?: boolean; truncate?: boolean }): JSX.Element;
  export function Rule(): JSX.Element;
  export function Dot(props: { tone?: 'ok' | 'warn' | 'danger' | 'brand' }): JSX.Element;
  // The loader calls this with the entry's default export; a piece never needs it
  export function mount(Piece: ComponentType): void;
}
