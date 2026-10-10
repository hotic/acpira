'use strict';
// Reads [[fn, args], ...] as JSON on stdin and prints [{ok: result} | {error: true}, ...]
const s = require(process.argv[2]);
const calls = JSON.parse(require('node:fs').readFileSync(0, 'utf8'));
const out = calls.map(([fn, args]) => {
  try {
    const r = s[fn](...args);
    return { ok: r === undefined ? null : r };
  } catch (e) {
    if (!(e instanceof TypeError)) throw e;
    return { error: true };
  }
});
process.stdout.write(JSON.stringify(out));
