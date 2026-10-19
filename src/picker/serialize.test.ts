import { describe, expect, it } from 'vitest';
import { envelopeOf, normalizePreviewUrl, pickedContext, redactAttr, redactUrl, type PickedElement } from '@shared/picker';
import { serialize, type MiniNode } from './serialize';

const text = (v: string): MiniNode => ({ nodeType: 3, nodeValue: v, childNodes: [] });
const el = (tag: string, attrs: Record<string, string> = {}, ...kids: MiniNode[]): MiniNode => ({
  nodeType: 1, nodeValue: null, tagName: tag.toUpperCase(), childNodes: kids,
  attributes: Object.entries(attrs).map(([name, value]) => ({ name, value })),
});

describe('picked element html', () => {
  it('drops secrets, handlers and code but keeps the structure', () => {
    const form = el('form', { action: '/login?next=/a&csrf_token=abc123', onsubmit: 'steal()' },
      el('input', { type: 'password', name: 'pw', value: 'hunter2' }),
      el('input', { type: 'hidden', name: 'x', value: 'opaque' }),
      el('input', { type: 'text', name: 'user', value: 'ada' }),
      el('script', {}, text('window.KEY = "sk-live"')),
      el('button', { 'data-auth-token': 'tok', class: 'primary' }, text('Sign in')));
    const html = serialize(form);
    expect(html).toBe('<form action="/login?next=/a&amp;csrf_token=[redacted]"><input type="password" name="pw" value="[redacted]">'
      + '<input type="hidden" name="x" value="[redacted]"><input type="text" name="user" value="ada"><script></script>'
      + '<button data-auth-token="[redacted]" class="primary">Sign in</button></form>');
    expect(html).not.toMatch(/hunter2|opaque|sk-live|steal|abc123/);
  });

  it('caps depth, children and length, and hides blobs', () => {
    const deep = el('div', {}, el('div', {}, el('div', {}, el('p', {}, text('too deep')))));
    expect(serialize(deep, { depth: 2, children: 12, max: 4000 })).toBe('<div><div><div>…</div></div></div>');
    const list = el('ul', {}, ...Array.from({ length: 5 }, (_, i) => el('li', {}, text(`#${i}`))));
    expect(serialize(list, { depth: 4, children: 2, max: 4000 })).toBe('<ul><li>#0</li><li>#1</li><!-- 3 more --></ul>');
    expect(serialize(list, { depth: 4, children: 12, max: 20 })).toBe('<ul><li>#0</li><li>…');
    const img = el('img', { src: 'data:image/png;base64,iVBORw0KGgo', alt: 'logo ' + 'A'.repeat(150) });
    expect(serialize(img)).toBe(`<img src="data:image/png;base64,[data]" alt="logo [base64]">`);
    expect(serialize(el('svg', { viewBox: '0 0 1 1' }, el('path', { d: 'M0 0' })))).toBe('<svg viewBox="0 0 1 1">…</svg>');
  });
});

describe('redaction', () => {
  it('keeps ordinary links whole and secret query values out', () => {
    expect(redactUrl('/a/b?page=2#top')).toBe('/a/b?page=2#top');
    expect(redactUrl('https://x.test/cb?code=1&access_token=zzz&state=s#f')).toBe('https://x.test/cb?code=1&access_token=[redacted]&state=s#f');
    expect(redactUrl('?apiKey=1&q')).toBe('?apiKey=[redacted]&q');
    expect(redactAttr('meta', undefined, 'name', 'csrf-token')).toBe('csrf-token');
    expect(redactAttr('div', undefined, 'aria-label', 'Session list')).toBe('Session list');
    expect(redactAttr('div', undefined, 'data-session-id', '42')).toBe('[redacted]');
    expect(redactAttr('a', undefined, 'onclick', 'x()')).toBeUndefined();
  });
});

describe('picker messages', () => {
  it('only takes envelopes stamped with the token', () => {
    expect(envelopeOf({ acpiraPicker: 't1', msg: { type: 'pong' } }, 't1')).toEqual({ type: 'pong' });
    expect(envelopeOf({ acpiraPicker: 't2', msg: { type: 'pong' } }, 't1')).toBeUndefined();
    expect(envelopeOf({ type: 'pong' }, 't1')).toBeUndefined();
    expect(envelopeOf(null, 't1')).toBeUndefined();
  });

  it('turns picked elements into one context item', () => {
    const one: PickedElement = { label: 'h1#title', selector: '#title', text: 'Hello', html: '<h1 id="title">Hello</h1>', rect: { x: 0, y: 0, width: 1, height: 1 } };
    expect(pickedContext([one], 'http://localhost:8765/post', 'Blog')).toEqual({
      label: 'h1#title',
      text: 'Page: Blog <http://localhost:8765/post>\n\nElement: `#title`  \nText: Hello\n\n```html\n<h1 id="title">Hello</h1>\n```',
    });
    const two = pickedContext([one, { ...one, label: 'p', selector: 'body > p', text: '' }], 'http://localhost/', '');
    expect(two.label).toBe('h1#title +1');
    expect(two.text).toContain('\n\nElement 2: `body > p`\n\n```html');
  });
});

describe('preview addresses', () => {
  it('fills in loopback http and refuses the rest', () => {
    expect(normalizePreviewUrl('5173')).toBe('http://localhost:5173/');
    expect(normalizePreviewUrl(' localhost:8765/post?id=1 ')).toBe('http://localhost:8765/post?id=1');
    expect(normalizePreviewUrl('http://0.0.0.0:3000')).toBe('http://localhost:3000/');
    expect(normalizePreviewUrl('127.0.0.1')).toBe('http://127.0.0.1/');
    expect(normalizePreviewUrl('http://[::1]:9/')).toBe('http://[::1]:9/');
    for (const bad of ['', 'example.com', 'https://localhost:5173', 'http://u:p@localhost/', 'file:///etc/hosts', 'http://192.168.1.2/']) {
      expect(normalizePreviewUrl(bad), bad).toBeUndefined();
    }
  });
});
