'use strict';
const test = require('node:test');
const assert = require('node:assert');
const s = require('../semver.js');

test('parse and format', () => {
  assert.deepStrictEqual(s.parse(' v1.2.3-beta.4+sha.5 '), { major: 1, minor: 2, patch: 3, prerelease: ['beta', 4], build: ['sha', '5'] });
  assert.strictEqual(s.parse('1.02.3'), null);
  assert.strictEqual(s.format(s.parse('1.2.3-rc.1')), '1.2.3-rc.1');
});

test('compare', () => {
  assert.strictEqual(s.compare('1.0.0-alpha', '1.0.0-alpha.1'), -1);
  assert.strictEqual(s.compare('1.0.0-rc.1', '1.0.0'), -1);
  assert.strictEqual(s.compare('1.0.0+a', '1.0.0+b'), 0);
  assert.throws(() => s.compare('x', '1.0.0'), TypeError);
});

test('ranges', () => {
  assert.ok(s.satisfies('1.9.9', '^1.2.3'));
  assert.ok(!s.satisfies('0.3.0', '^0.2.3'));
  assert.ok(s.satisfies('2.3.9', '1.2.3 - 2.3'));
  assert.ok(!s.satisfies('1.3.0-beta', '>=1.2.0'));
  assert.ok(s.satisfies('1.3.0-beta.2', '>=1.3.0-beta.1 <2'));
  assert.strictEqual(s.maxSatisfying(['1.2.3', '1.4.0', '2.0.0'], '~1.2 || ~1.4'), '1.4.0');
});

test('inc', () => {
  assert.strictEqual(s.inc('1.2.3', 'minor'), '1.3.0');
  assert.strictEqual(s.inc('1.2.3', 'prerelease'), '1.2.4-0');
  assert.strictEqual(s.inc('1.2.4-alpha.1', 'prerelease'), '1.2.4-alpha.2');
  assert.strictEqual(s.inc('2.0.0-rc.1', 'major'), '2.0.0');
});
