import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import type { ImageRef } from '../../shared/transcript';
import { AgentImage } from './AgentImage';
import { BlobUrlContext, FileStagerContext, type FileStager } from './fileLinks';

// An image kept as a host path (a session on an SSH host): what the page shows before, while and after the host reads it
const hostImage: ImageRef = { mimeType: 'image/png', uri: '/srv/app/shot.png' };

function render(image: ImageRef, stager?: FileStager) {
  return renderToStaticMarkup(createElement(BlobUrlContext.Provider, { value: blob => `/blobs/${blob}` },
    createElement(FileStagerContext.Provider, { value: stager }, createElement(AgentImage, { image, caption: false }))));
}

describe('an image kept as a path', () => {
  it('shows the copy this page already read without asking the host again', () => {
    const asked: string[] = [];
    const html = render(hostImage, { known: path => (path === hostImage.uri ? 'abc.png' : undefined), load: async path => { asked.push(path); return undefined; } });
    expect(html).toContain('src="/blobs/abc.png"');
    expect(asked).toEqual([]);
  });

  it('holds an image-sized place while the host reads it', () => {
    const html = render(hostImage, { known: () => undefined, load: () => new Promise(() => {}) });
    expect(html).toContain('role="status"');
    expect(html).toContain('h-agent-image');
    expect(html).not.toContain('<img');
  });

  it('names the file when no host can read it, and a saved copy needs none', () => {
    expect(render(hostImage)).toContain('shot.png');
    expect(render(hostImage)).not.toContain('<img');
    expect(render({ ...hostImage, blob: 'saved.png' })).toContain('src="/blobs/saved.png"');
  });
});
