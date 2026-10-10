// Model sources of the built-in agent (`acpira agent`): `<ACPIRA_HOME>/providers.json`, mirror of
// rust/crates/acpira-shared/src/providers.rs. The host writes the file and keeps each source's API key in secrets.json;
// the settings page only ever sees `hasKey`. Unknown fields survive a rewrite, so an entry sent back carries them along

export const PROVIDERS_FILE = 'providers.json';

// `openai-chat` (OpenAI Chat Completions and compatibles) | `anthropic` (Messages); anything else is listed but not used
export type ApiFormat = 'openai-chat' | 'anthropic';

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
  // The preset it was created from (`deepseek`, `ollama`, … or `custom`)
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
  // What a new source can start from (the agent's table, rust/crates/acpira-agent/src/llm/presets.rs)
  presets: Preset[];
  // The families a model can be pinned to (llm/family.rs), the generic one first
  families: string[];
}

// A known service a new source can start from; `custom` has an empty URL
export interface Preset {
  id: string;
  name: string;
  format: ApiFormat | (string & {});
  baseUrl: string;
  // Where the service hands out API keys
  keyUrl?: string;
  // Runs on this machine and takes no key (Ollama, LM Studio)
  local?: boolean;
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
  | { kind: 'delete'; id: string };

// A model entry with the file's defaults, for a hand-typed id
export function newModel(id: string): ProviderModel {
  return { id, enabled: true, input: ['text'], thinking: 'auto', efforts: [], sampling: {}, estimated: [] };
}

// The built-in agent's id in the registry (`SELF_AGENT_ID` in rust/crates/acpira-host/src/acp/agents/registry.rs)
export const BUILTIN_AGENT_ID = 'acpira';
