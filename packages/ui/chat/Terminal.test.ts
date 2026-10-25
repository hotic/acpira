import { describe, expect, it } from 'vitest';
import { unfence } from './Terminal';

describe('unfence', () => {
  it('unwraps a body that is a single fenced block', () => {
    expect(unfence('```\nRequest was aborted.\n```')).toBe('Request was aborted.');
    expect(unfence('```text\na\nb\n```\n')).toBe('a\nb');
  });
  it('leaves prose and several blocks alone', () => {
    expect(unfence('intro\n```\nx\n```')).toBe('intro\n```\nx\n```');
    expect(unfence('```\na\n```\n\n```\nb\n```')).toBe('```\na\n```\n\n```\nb\n```');
  });
});
