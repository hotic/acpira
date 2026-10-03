// Attachment helpers shared by host and webview: which files count as images, and the size ceilings both sides enforce
import type { Attachment, Draft } from './transcript';

// Inline image payloads above this are refused (the webview refuses the paste, the host falls back to a resource_link for dropped files)
export const MAX_IMAGE_BYTES = 10 * 1024 * 1024;
// Text dropped from outside the workspace is embedded into the prompt, so it stays small
export const MAX_TEXT_BYTES = 1024 * 1024;

const IMAGE_MIME: Record<string, string> = {
  '.png': 'image/png',
  '.jpg': 'image/jpeg',
  '.jpeg': 'image/jpeg',
  '.gif': 'image/gif',
  '.webp': 'image/webp',
};

// MIME type for an image file name, undefined for anything that isn't an image the models accept
export function imageMimeOf(name: string): string | undefined {
  const m = /\.[^./\\]+$/.exec(name);
  return m ? IMAGE_MIME[m[0].toLowerCase()] : undefined;
}

// File extension to persist a blob of the given MIME type under
export function extOfMime(mimeType: string): string {
  return Object.entries(IMAGE_MIME).find(([, m]) => m === mimeType)?.[0] ?? '.bin';
}

// Base64 payload size in bytes (without decoding)
export function base64Bytes(data: string): number {
  const pad = data.endsWith('==') ? 2 : data.endsWith('=') ? 1 : 0;
  return Math.floor((data.length * 3) / 4) - pad;
}

// Line-range suffix of a selection chip: `(12-19)`, or `(12)` for one line
export function lineRangeLabel(startLine: number, endLine: number): string {
  return startLine === endLine ? `(${startLine})` : `(${startLine}-${endLine})`;
}

// What a chip / export line calls an attachment. Quotes have no name of their own (they are grouped under one annotations chip)
export function attachmentLabel(a: Attachment | Draft): string | undefined {
  switch (a.kind) {
    case 'selection': return `${a.name} ${lineRangeLabel(a.startLine, a.endLine)}`;
    case 'quote': return undefined;
    default: return a.name;
  }
}

// The persisted shape of a draft before the host has staged it (no blob yet): what an optimistic transcript shows
export function attachmentOfDraft(d: Draft): Attachment {
  switch (d.kind) {
    case 'image': return { kind: 'image', mimeType: d.mimeType, name: d.name };
    case 'text': return { kind: 'text', name: d.name };
    case 'file': return { kind: 'file', uri: d.uri, name: d.name };
    case 'selection': return { kind: 'selection', uri: d.uri, name: d.name, startLine: d.startLine, endLine: d.endLine };
    case 'quote': return d.comment ? { kind: 'quote', text: d.text, comment: d.comment } : { kind: 'quote', text: d.text };
  }
}
