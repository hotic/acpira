// Model sources of the built-in agent (`acpira agent`): `<ACPIRA_HOME>/providers.json`, mirror of
// rust/crates/acpira-shared/src/providers.rs. The host writes the file and keeps each source's API key in secrets.json;
// the settings page only ever sees `hasKey`. Unknown fields survive a rewrite, so an entry sent back carries them along

export const PROVIDERS_FILE = 'providers.json';

// `openai-chat` (OpenAI Chat Completions and compatibles) | `anthropic` (Messages); anything else is listed but not used
export type ApiFormat = 'openai-chat' | 'openai-responses' | 'anthropic';

// Whether a model reasons: auto leaves the provider's default, on / off send the family's switch
export type Thinking = 'auto' | 'on' | 'off';

export interface Sampling {
  temperature?: number;
  topP?: number;
  topK?: number;
}

export interface ProviderModel {
  // The id the API takes
  id: string;
  name?: string;
  enabled: boolean;
  // Context window / output limit in tokens
  context?: number;
  output?: number;
  // `text` always, `image` when the model takes images
  input: string[];
  thinking: Thinking;
  // Reasoning levels offered as the effort select (empty = no select)
  efforts: string[];
  // The prompt / request family pinned for this model; null matches automatically
  family?: string | null;
  // Whether sessions offer the Fast switch (the provider's premium speed tier); absent decides from the API format and
  // the model id (llm/fast.rs), true / false override that
  fast?: boolean | null;
  sampling: Sampling;
  // Tool-call rounds per turn before the agent pauses; null = no cap
  maxSteps?: number | null;
  // Field names whose values were guessed rather than reported by the endpoint
  estimated: string[];
  [extra: string]: unknown;
}

export interface Provider {
  // Unique, no `/` (model picks are `<provider id>/<model id>`); empty on a new entry, the host makes one from the name
  id: string;
  name: string;
  // `custom`, or `ollama` / `lmstudio` for a local server found on this machine (takes no key); older files may name a
  // vendor (`deepseek`, …), still read as a family hint
  preset: string;
  format: ApiFormat | (string & {});
  // The API root, or the full endpoint when fullUrl is on
  baseUrl: string;
  fullUrl: boolean;
  enabled: boolean;
  headers?: Record<string, string>;
  models: ProviderModel[];
  [extra: string]: unknown;
}

export interface ProviderView extends Provider {
  hasKey: boolean;
}

// `error` when providers.json exists but cannot be read (the host then leaves it alone)
export interface ProvidersView {
  providers: ProviderView[];
  error?: string;
  // The families a model can be pinned to (llm/family.rs), the generic one first
  families: string[];
  // The agent's approval level as the file holds it; absent reads as 'ask'
  approval?: string;
  // The model for quick side jobs such as naming a session (`<provider id>/<model id>`); absent turns them off
  fastModel?: string;
  // The websearch backend as the file holds it; absent reads as 'default'
  webSearch?: string;
  // The model source DeepSeek-shaped search goes through (a gateway serving /v1/tools/search); absent is DeepSeek's API
  webSearchVia?: string;
  // The search backends with a stored key (never the key itself)
  webKeys?: string[];
  // The compaction strategy as the file holds it; absent reads as 'summary'
  compaction?: string;
}

// How the built-in agent compacts its history (`COMPACTION_STRATEGIES` in acpira-shared/src/providers.rs), the default
// first: a model-written summary, or chunks of the finished turns (no model call)
export const COMPACTION_STRATEGIES = ['summary', 'chunk'] as const;
export type CompactionStrategy = typeof COMPACTION_STRATEGIES[number];

// The view's strategy, unknown or absent values reading as the default
export function compactionOf(view?: ProvidersView): CompactionStrategy {
  return COMPACTION_STRATEGIES.find(c => c === view?.compaction) ?? COMPACTION_STRATEGIES[0];
}

// The built-in agent's search backends (`WEB_SEARCH_BACKENDS` in acpira-shared/src/providers.rs), the default first:
// the free hosted endpoints, Jina Search with the user's key, or DeepSeek's Search API (its own or through a model
// source); a keyed pick has the other keyed backend and the free ones behind it
export const WEB_SEARCH_BACKENDS = ['default', 'jina', 'deepseek'] as const;
export type WebSearchBackend = typeof WEB_SEARCH_BACKENDS[number];

// The view's backend, unknown or absent values reading as the default
export function webSearchOf(view?: ProvidersView): WebSearchBackend {
  return WEB_SEARCH_BACKENDS.find(b => b === view?.webSearch) ?? WEB_SEARCH_BACKENDS[0];
}

// How much the built-in agent runs without a permission card (`APPROVAL_LEVELS` in acpira-shared/src/providers.rs),
// the default first. A setting of the agent, not a session control
export const APPROVAL_LEVELS = ['ask', 'auto-edit', 'full'] as const;
export type ApprovalLevel = typeof APPROVAL_LEVELS[number];

// The view's level, unknown or absent values reading as the default
export function approvalOf(view?: ProvidersView): ApprovalLevel {
  return APPROVAL_LEVELS.find(l => l === view?.approval) ?? APPROVAL_LEVELS[0];
}

// The formats the agent speaks (`ApiFormat::parse`; an empty format reads as openai-chat)
const KNOWN_FORMATS = new Set(['', 'openai-chat', 'openai-responses', 'anthropic']);

// Every model the agent can call, in file order, as `ProvidersFile::usable` picks them: enabled source with a known
// format, enabled model
export function usableModels(view?: ProvidersView): { pick: string; provider: ProviderView; model: ProviderModel }[] {
  return (view?.providers ?? [])
    .filter(p => p.enabled && KNOWN_FORMATS.has(p.format))
    .flatMap(p => p.models.filter(m => m.enabled).map(m => ({ pick: `${p.id}/${m.id}`, provider: p, model: m })));
}

// A network question from the settings page; none changes providers.json. key: typed into the form and not saved yet;
// absent uses the stored key of the source with this id
export type ProviderProbe =
  | { kind: 'models'; provider: Provider; key?: string }
  | { kind: 'check'; provider: Provider; key?: string }
  // One tiny real call: spends a few tokens
  | { kind: 'test'; provider: Provider; model: ProviderModel; key?: string }
  | { kind: 'local' };

export type ProbeOutcome =
  | { kind: 'models'; models: ProviderModel[] }
  | { kind: 'check'; count: number }
  | { kind: 'test'; ms: number; text: string }
  | { kind: 'local'; servers: LocalSource[] }
  | { kind: 'failed'; error: string };

// A model server running on this machine, ready to save as a source
export interface LocalSource {
  preset: string;
  name: string;
  baseUrl: string;
  models: ProviderModel[];
}

// key: absent keeps the stored key, empty removes it
export type ProviderAction =
  | { kind: 'save'; provider: Provider; key?: string }
  | { kind: 'delete'; id: string }
  | { kind: 'approval'; level: ApprovalLevel }
  // pick: a usable `<provider id>/<model id>`; absent turns the fast model off
  | { kind: 'fastModel'; pick?: string }
  // key: that backend's; absent keeps the stored key, empty removes it. via: the model source DeepSeek-shaped search goes
  // through; absent keeps it, empty goes back to DeepSeek's own API
  | { kind: 'webSearch'; backend: WebSearchBackend; key?: string; via?: string }
  | { kind: 'compaction'; strategy: CompactionStrategy };

// A model entry with the file's defaults, for a hand-typed id
export function newModel(id: string): ProviderModel {
  return { id, enabled: true, input: ['text'], thinking: 'auto', efforts: [], sampling: {}, estimated: [] };
}

// The built-in agent's id in the registry (`SELF_AGENT_ID` in rust/crates/acpira-host/src/acp/agents/registry.rs)
export const BUILTIN_AGENT_ID = 'acpira';
