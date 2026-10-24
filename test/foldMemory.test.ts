import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { rememberFold, rememberedFold, resetFoldMemory } from '../packages/ui/chat/foldMemory';

// The webview's persisted state stands in for acquireVsCodeApi(); vitest runs in node, so the memoized api is a stub on globalThis
let persisted: unknown;
const api = { postMessage() {}, getState: () => persisted, setState: (s: unknown) => { persisted = s; } };
const scope = globalThis as { __acpiraApi?: unknown };

beforeEach(() => {
  persisted = undefined;
  scope.__acpiraApi = api;
  resetFoldMemory();
});
afterEach(() => {
  delete scope.__acpiraApi;
  resetFoldMemory();
});

describe('fold memory', () => {
  it('has no opinion about a fold nobody touched', () => {
    expect(rememberedFold('s1:1:100')).toBeUndefined();
  });

  it('keeps the last manual choice per turn and writes it through to the webview state', () => {
    rememberFold('s1:1:100', true);
    rememberFold('s1:3:200', false);
    expect(rememberedFold('s1:1:100')).toBe(true);
    expect(rememberedFold('s1:3:200')).toBe(false);
    expect(persisted).toEqual({ folds: { 's1:1:100': true, 's1:3:200': false } });
    rememberFold('s1:1:100', false);
    expect(rememberedFold('s1:1:100')).toBe(false);
  });

  it('reads choices back after the module state is gone, as a reloaded webview would', () => {
    rememberFold('s1:1:100', true);
    resetFoldMemory();
    expect(rememberedFold('s1:1:100')).toBe(true);
  });

  it('leaves unrelated webview state alone', () => {
    persisted = { other: 1 };
    rememberFold('s1:1:100', true);
    expect(persisted).toEqual({ other: 1, folds: { 's1:1:100': true } });
  });

  it('trims the oldest choices once the store is full', () => {
    for (let i = 0; i < 205; i++) rememberFold(`s:${i}:0`, true);
    expect(rememberedFold('s:0:0')).toBeUndefined();
    expect(rememberedFold('s:4:0')).toBeUndefined();
    expect(rememberedFold('s:5:0')).toBe(true);
    expect(rememberedFold('s:204:0')).toBe(true);
  });

  it('works without a host api and without persisted state', () => {
    delete scope.__acpiraApi;
    resetFoldMemory();
    rememberFold('s1:1:100', true);
    expect(rememberedFold('s1:1:100')).toBe(true);
  });
});
