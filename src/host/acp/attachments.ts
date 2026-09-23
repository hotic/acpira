import { createHash } from 'node:crypto';
import { readFile, stat } from 'node:fs/promises';
import { basename } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import type * as acp from '@agentclientprotocol/sdk';
import type { Attachment, Draft } from '@shared/transcript';
import { MAX_IMAGE_BYTES, MAX_TEXT_BYTES, base64Bytes, extOfMime, imageMimeOf } from '@shared/attachments';
import type { AgentDef } from './AgentRegistry';
import { msg } from '../errors';
import { t } from '../i18n';

// Blob names are content-derived so a transcript can reference a blob before its write completes (agent images are
// referenced synchronously while the store write lands async) and identical payloads share one file
export function blobName(ext: string, bytes: Uint8Array): string {
  return `${createHash('sha256').update(bytes).digest('hex').slice(0, 16)}${ext}`;
}

// Where a session parks attachment payloads (TranscriptStore implements it): the name is what the turn keeps and the webview loads via blobBase,
// the absolute path is what the agent gets told when the content is embedded. Names follow blobName()
export interface BlobStore {
  saveBlob(sessionId: string, ext: string, bytes: Uint8Array): Promise<{ name: string; path: string }>;
  readBlob(sessionId: string, name: string): Promise<Uint8Array>;
  // Absolute path of a saved blob, when the store is on this machine's disk (export links, opening in the editor)
  blobPath?(sessionId: string, name: string): string | undefined;
}

export interface PreparedPrompt {
  blocks: acp.ContentBlock[];
  attachments: Attachment[];
  // What could not be done as asked (a draft dropped for size, a blob that failed to write); the prompt itself still goes out
  problems: string[];
}

// What the agent says it can take in a prompt, plus the per-agent override for a capability that lies.
// An absent capability (or no caps at all) means permitted — the historical behaviour
export interface PromptCaps {
  embeddedContext?: boolean;
  image?: boolean;
  // The agent advertises image: false yet accepts image blocks (Grok) — declared per agent as AgentDef.prompt.imagesRegardless
  imagesRegardless?: boolean;
}

// The prompt capabilities to stage against: the advertised set, defaulting to permitted when the agent said nothing,
// overridden by what the registry knows about the agent
export function promptCapsOf(init: acp.InitializeResponse | undefined, def: AgentDef): PromptCaps {
  const p = init?.agentCapabilities?.promptCapabilities;
  return { embeddedContext: p?.embeddedContext ?? true, image: p?.image ?? true, imagesRegardless: def.prompt?.imagesRegardless };
}

// Turns the composer's text + drafts into the wire prompt and the transcript attachments. Text goes first, then one block per draft:
// images inline as base64 (unless the agent says it takes none — the Grok exception is declared per agent, AgentDef.prompt.imagesRegardless),
// dropped text as an embedded resource whose uri is the blob written to disk (plain marked-up text when embeddedContext is unsupported),
// files as resource_link so the agent reads them itself — except image files, which are read here and sent as pixels.
// Never throws for a single draft: a payload the disk refuses still goes over the wire (the preview is lost), an oversized image is dropped with a note
export async function preparePrompt(sessionId: string, text: string, drafts: Draft[], blobs: BlobStore, caps?: PromptCaps): Promise<PreparedPrompt> {
  const out: PreparedPrompt = { blocks: text ? [{ type: 'text', text }] : [], attachments: [], problems: [] };
  const noImages = caps?.image === false && !caps.imagesRegardless;
  const stage = async (ext: string, bytes: Uint8Array, label: string) => {
    try { return await blobs.saveBlob(sessionId, ext, bytes); }
    catch (e) { out.problems.push(t('host.attachStageFailed', { label, error: msg(e) })); return undefined; }
  };
  for (const d of drafts) {
    if (d.kind === 'image') {
      if (noImages) { out.problems.push(t('host.imageUnsupported', { name: d.name ?? t('common.image') })); continue; }
      if (base64Bytes(d.data) > MAX_IMAGE_BYTES) { out.problems.push(t('host.imageTooBig', { name: d.name ?? t('common.image'), mb: MAX_IMAGE_BYTES >> 20 })); continue; }
      const saved = await stage(extOfMime(d.mimeType), Buffer.from(d.data, 'base64'), d.name ?? t('common.image'));
      out.blocks.push({ type: 'image', mimeType: d.mimeType, data: d.data });
      out.attachments.push({ kind: 'image', blob: saved?.name, mimeType: d.mimeType, name: d.name });
    } else if (d.kind === 'text') {
      // The webview refuses oversized drops too, but the ceiling is enforced here as well: the text is embedded into the prompt verbatim
      const bytes = Buffer.from(d.text, 'utf8');
      if (bytes.byteLength > MAX_TEXT_BYTES) { out.problems.push(t('attach.tooBigText', { name: d.name, kb: MAX_TEXT_BYTES >> 10 })); continue; }
      // The embedded resource is addressed by the blob on disk, so an agent that insists on reading a real file finds one
      const saved = await stage('.txt', bytes, d.name);
      if (caps?.embeddedContext === false) {
        out.blocks.push({ type: 'text', text: `[Attachment: ${d.name}]\n${d.text}\n[End of attachment: ${d.name}]` });
      } else {
        const uri = saved ? pathToFileURL(saved.path).href : `attachment:///${encodeURIComponent(d.name)}`;
        out.blocks.push({ type: 'resource', resource: { uri, mimeType: 'text/plain', text: d.text } });
      }
      out.attachments.push({ kind: 'text', blob: saved?.name, name: d.name });
    } else {
      const image = await readImageFile(d.uri);
      if (image) {
        // The agent said it takes no images — a resource_link is not a fallback, it reads the file as pixels anyway
        if (noImages) { out.problems.push(t('host.imageUnsupported', { name: d.name })); continue; }
        const saved = await stage(extOfMime(image.mimeType), image.bytes, d.name);
        out.blocks.push({ type: 'image', mimeType: image.mimeType, data: image.bytes.toString('base64') });
        out.attachments.push({ kind: 'image', blob: saved?.name, mimeType: image.mimeType, name: d.name });
      } else {
        out.blocks.push({ type: 'resource_link', uri: d.uri, name: d.name || basename(d.uri) });
        out.attachments.push({ kind: 'file', uri: d.uri, name: d.name });
      }
    }
  }
  return out;
}

// The inverse of preparePrompt, for sending a persisted user turn again: blobs are read back into image / text drafts, files stay links.
// An image that came from a file on disk was persisted as an image blob, so it goes out as pixels again rather than being re-read from the original path.
// Attachments whose blob never made it to disk cannot come back and are left out
export async function restoreDrafts(sessionId: string, attachments: Attachment[], blobs: BlobStore): Promise<Draft[]> {
  const drafts = await Promise.all(attachments.map(async (a): Promise<Draft | undefined> => {
    if (a.kind === 'file') return { kind: 'file', uri: a.uri, name: a.name };
    if (!a.blob) return undefined;
    const bytes = Buffer.from(await blobs.readBlob(sessionId, a.blob));
    return a.kind === 'image'
      ? { kind: 'image', mimeType: a.mimeType, data: bytes.toString('base64'), name: a.name }
      : { kind: 'text', name: a.name, text: bytes.toString('utf8') };
  }));
  return drafts.filter((d): d is Draft => d !== undefined);
}

// A file:// URI that names an image the models accept, small enough to inline; anything else (non-image, unreadable, oversized, remote) yields undefined.
// Size is checked before reading so an oversized file never lands in memory
async function readImageFile(uri: string): Promise<{ mimeType: string; bytes: Buffer } | undefined> {
  const mimeType = imageMimeOf(uri);
  if (!mimeType || !uri.startsWith('file:')) return undefined;
  try {
    const path = fileURLToPath(uri);
    const { size } = await stat(path);
    return size <= MAX_IMAGE_BYTES ? { mimeType, bytes: await readFile(path) } : undefined;
  } catch {
    return undefined;
  }
}

// One-line description of what a prompt carried (drafts before staging or attachments after), for the title of a session opened with attachments only
export function describeDrafts(drafts: (Draft | Attachment)[]): string {
  const images = drafts.filter(d => d.kind === 'image').length;
  const files = drafts.filter(d => d.kind !== 'image').map(d => d.name);
  return [images ? t('host.images', { n: images }) : '', ...files].filter(Boolean).join(t('common.listSep'));
}
