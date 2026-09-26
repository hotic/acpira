import { afterEach, describe, expect, it } from 'vitest';
import type { ToolCallBlock } from '../src/shared/transcript';
import { isImageGenBlock, isImageGenTool, toolImages, toolTexts } from '../src/shared/imageTools';
import { setLocale } from '../src/webview/i18n';
import { foldActivity, toolVerb } from '../src/webview/chat/folding';
import { agentTurn } from './fixtures/engine';
import { composite, imageGenColors } from '../src/webview/effects/imageGenPalette';

afterEach(() => setLocale('en'));

describe('image generation tools', () => {
  it('keeps the engine-assigned identity from the first packet through completion', () => {
    const running = agentTurn('image-gen-codex', 0);
    const call = running.blocks[0] as ToolCallBlock;
    expect(isImageGenTool(call)).toBe(true);
    expect(foldActivity(running)).toMatchObject({ label: 'Generate image…', active: true });
    setLocale('zh-CN');
    expect(toolVerb(call)).toBe('正在生成图片');
    const done = agentTurn('image-gen-codex').blocks[0] as ToolCallBlock;
    expect(isImageGenTool(done)).toBe(true);
    expect(toolVerb(done)).toBe('已生成图片');
    // Without a blob saver (the golden engine has none) the image degrades to text and no card is due
    expect(toolImages(done)).toEqual([]);
    expect(toolTexts(done)).toContain('Revised prompt: a red dot on white');
  });

  it('collects saved images in wire order and skips unsaved ones', () => {
    const base: ToolCallBlock = { type: 'tool_call', id: 'g', kind: 'other', verb: 'Generate image', verbKey: 'verb.imagegen', status: 'completed' };
    const a = { type: 'image' as const, blob: 'a.png', mimeType: 'image/png' };
    const b = { type: 'image' as const, blob: 'b.png', mimeType: 'image/png', uri: '/tmp/b.png' };
    const lost = { type: 'image' as const, mimeType: 'image/png', uri: '/tmp/gone.png' };
    const text = { type: 'text' as const, text: 'Revised prompt: two dots' };
    expect(toolImages({ ...base, content: a })).toEqual([a]);
    expect(toolImages({ ...base, content: text, contents: [text, a, lost, b] })).toEqual([a, b]);
    expect(toolTexts({ ...base, content: text, contents: [text, a] })).toBe('Revised prompt: two dots');
    expect(isImageGenBlock({ type: 'text', markdown: 'x' })).toBe(false);
    expect(isImageGenBlock({ ...base, verbKey: 'verb.todo' })).toBe(false);
    expect(isImageGenBlock(base)).toBe(true);
  });
});

describe('image generation colours', () => {
  it('composites translucent surfaces down to the opaque colour behind the card', () => {
    // --chip (white at 7%) over the dark sidebar base #181818
    expect(composite([[24, 24, 24, 1], [255, 255, 255, 0.07]]).map(Math.round)).toEqual([40, 40, 40]);
  });

  it('builds the mosaic palette on the real surface, with the peak in the highlight slot', () => {
    const dark = imageGenColors([24, 24, 24], [237, 237, 237], [125, 196, 255]);
    expect(dark.cardBg).toBe('#181818');
    expect(dark.colors).toHaveLength(7);
    expect(dark.colors[3]).toBe('#181818');
    expect(dark.colors[4]).toBe('#7dc4ff');
    // Light paper takes lighter ink steps: the mid slot stays much closer to the paper than on dark
    const light = imageGenColors([248, 248, 248], [13, 13, 13], [36, 110, 200]);
    expect(light.cardBg).toBe('#f8f8f8');
    const gap = (hex: string, paper: number) => Math.abs(parseInt(hex.slice(1, 3), 16) - paper);
    expect(gap(light.colors[2]!, 248)).toBeLessThan(gap(dark.colors[2]!, 24));
  });
});
