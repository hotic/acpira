import { expect, test } from 'vitest';
import { firstPlainLine } from '../subagents';
test('firstPlainLine drops inline marks from a preview line', () => {
  expect(firstPlainLine('结论：**建议暂缓合入**。正常的旧版 DPA')).toBe('结论：建议暂缓合入。正常的旧版 DPA');
  expect(firstPlainLine('\n\n## Summary\nmore')).toBe('Summary');
  expect(firstPlainLine('- [x] done `cargo test` and *ok* see [docs](http://x)')).toBe('done cargo test and ok see docs');
  expect(firstPlainLine('```rust\nfn main')).toBe('fn main');
  expect(firstPlainLine('a_b_c snake_case 2*3*4')).toBe('a_b_c snake_case 2*3*4');
  expect(firstPlainLine('~~old~~ new __bold__')).toBe('old new bold');
  expect(firstPlainLine('---\n\n')).toBeUndefined();
});
