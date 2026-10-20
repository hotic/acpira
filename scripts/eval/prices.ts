import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { MODELS } from './harnesses';

// Catalogue prices for the eval reports (USD per million tokens): a MODELS entry's own price, else the built-in model
// catalogue's entry for its catalogue id (or for the name itself, for a model outside MODELS)

export interface Price { input: number; output: number; cacheRead?: number; cacheWrite?: number }

const catalog = JSON.parse(readFileSync(resolve(import.meta.dirname, '../../rust/crates/acpira-shared/assets/model-catalog.json'), 'utf8')) as { models: { id: string; cost?: Price }[] };

export function price(model: string): Price {
  const id = MODELS[model]?.catalog ?? model;
  const p = MODELS[model]?.price ?? catalog.models.find(m => m.id === id)?.cost;
  if (!p) throw new Error(`no catalogue price for ${id}`);
  return p;
}

export interface Tokens { model: string; input: number; cacheRead: number; cacheWrite: number; output: number }

// USD: uncached input, cache reads and writes, output (reasoning is part of output on every wire read here)
export function cost(r: Tokens): number {
  const p = price(r.model);
  const uncached = Math.max(0, r.input - r.cacheRead - r.cacheWrite);
  return (uncached * p.input + r.cacheRead * (p.cacheRead ?? p.input) + r.cacheWrite * (p.cacheWrite ?? p.input) + r.output * p.output) / 1e6;
}
