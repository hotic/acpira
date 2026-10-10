'use strict';
// semver-lite: parse, compare, range matching and increments for semantic versions (a subset of npm's semver)

const NUM = '0|[1-9]\\d*';
const ID = `(?:${NUM}|\\d*[a-zA-Z-][a-zA-Z0-9-]*)`;
const FULL = new RegExp(`^v?(${NUM})\\.(${NUM})\\.(${NUM})(?:-(${ID}(?:\\.${ID})*))?(?:\\+([0-9A-Za-z-]+(?:\\.[0-9A-Za-z-]+)*))?$`);
// A partial version in a range: 1, 1.2, 1.2.3, with x / X / * wildcards
const PARTIAL = new RegExp(`^v?(${NUM}|[xX*])(?:\\.(${NUM}|[xX*])(?:\\.(${NUM}|[xX*])(?:-(${ID}(?:\\.${ID})*))?)?)?$`);

function parse(text) {
  if (typeof text !== 'string') return null;
  const m = FULL.exec(text.trim());
  if (!m) return null;
  return {
    major: Number(m[1]),
    minor: Number(m[2]),
    patch: Number(m[3]),
    prerelease: m[4] ? m[4].split('.').map(id => (/^\d+$/.test(id) ? Number(id) : id)) : [],
    build: m[5] ? m[5].split('.') : [],
  };
}

function format(v) {
  let s = `${v.major}.${v.minor}.${v.patch}`;
  if (v.prerelease.length) s += '-' + v.prerelease.join('.');
  if (v.build.length) s += '+' + v.build.join('.');
  return s;
}

function must(v) {
  const p = typeof v === 'string' ? parse(v) : v;
  if (!p) throw new TypeError(`invalid version: ${v}`);
  return p;
}

function cmpNum(a, b) {
  return a < b ? -1 : a > b ? 1 : 0;
}

function comparePre(a, b) {
  // A version without prerelease ranks above one with it
  if (!a.length && !b.length) return 0;
  if (!a.length) return 1;
  if (!b.length) return -1;
  for (let i = 0; ; i++) {
    if (i >= a.length && i >= b.length) return 0;
    if (i >= a.length) return -1;
    if (i >= b.length) return 1;
    const x = a[i];
    const y = b[i];
    if (x === y) continue;
    const xn = typeof x === 'number';
    const yn = typeof y === 'number';
    if (xn && yn) return cmpNum(x, y);
    if (xn) return -1;
    if (yn) return 1;
    return x < y ? -1 : 1;
  }
}

// -1, 0 or 1 by precedence; build metadata is ignored
function compare(a, b) {
  const x = must(a);
  const y = must(b);
  return cmpNum(x.major, y.major) || cmpNum(x.minor, y.minor) || cmpNum(x.patch, y.patch) || comparePre(x.prerelease, y.prerelease);
}

const isX = p => p === undefined || p === 'x' || p === 'X' || p === '*';

function partial(text) {
  const m = PARTIAL.exec(text);
  if (!m) throw new TypeError(`invalid range: ${text}`);
  const [, a, b, c, pre] = m;
  const xs = [a, b, c].map(isX);
  // Once a part is a wildcard every later part is too (1.x.3 is 1.x)
  const n = xs.indexOf(true) === -1 ? 3 : xs.indexOf(true);
  return {
    parts: n,
    major: n > 0 ? Number(a) : 0,
    minor: n > 1 ? Number(b) : 0,
    patch: n > 2 ? Number(c) : 0,
    prerelease: n > 2 && pre ? parse(`0.0.0-${pre}`).prerelease : [],
    build: [],
  };
}

const ver = (major, minor, patch, prerelease = []) => ({ major, minor, patch, prerelease, build: [] });
const cmp = (op, v) => ({ op, v });

// The upper bound right above a partial version: 1 -> 2.0.0, 1.2 -> 1.3.0
function above(p) {
  return p.parts === 1 ? ver(p.major + 1, 0, 0) : ver(p.major, p.minor + 1, 0);
}

function expand(op, text) {
  const p = partial(text);
  const lo = ver(p.major, p.minor, p.patch, p.prerelease);
  if (op === '^') {
    if (p.parts === 0) return [cmp('>=', ver(0, 0, 0))];
    let hi;
    if (p.major > 0 || p.parts === 1) hi = ver(p.major + 1, 0, 0);
    else if (p.minor > 0 || p.parts === 2) hi = ver(0, p.minor + 1, 0);
    else hi = ver(0, 0, p.patch + 1);
    return [cmp('>=', lo), cmp('<', hi)];
  }
  if (op === '~') {
    if (p.parts === 0) return [cmp('>=', ver(0, 0, 0))];
    return [cmp('>=', lo), cmp('<', p.parts === 1 ? ver(p.major + 1, 0, 0) : ver(p.major, p.minor + 1, 0))];
  }
  if (p.parts === 3) return [cmp(op || '=', lo)];
  if (p.parts === 0) return op === '<' || op === '>' ? [cmp('<', ver(0, 0, 0))] : [cmp('>=', ver(0, 0, 0))];
  // A partial version with an operator
  switch (op) {
    case '>': return [cmp('>=', above(p))];
    case '>=': return [cmp('>=', lo)];
    case '<': return [cmp('<', lo)];
    case '<=': return [cmp('<', above(p))];
    default: return [cmp('>=', lo), cmp('<', above(p))];
  }
}

function parseSet(text) {
  const t = text.trim().replace(/(<=|>=|<|>|=|\^|~)\s+/g, '$1');
  const hy = /^(\S+)\s+-\s+(\S+)$/.exec(t);
  if (hy) {
    const lo = partial(hy[1]);
    const hi = partial(hy[2]);
    const out = [cmp('>=', ver(lo.major, lo.minor, lo.patch, lo.prerelease))];
    if (hi.parts === 3) out.push(cmp('<=', ver(hi.major, hi.minor, hi.patch, hi.prerelease)));
    else if (hi.parts > 0) out.push(cmp('<', above(hi)));
    return out;
  }
  if (t === '') return [cmp('>=', ver(0, 0, 0))];
  const out = [];
  for (const part of t.split(/\s+/)) {
    const m = /^(<=|>=|<|>|=|\^|~)?(.*)$/.exec(part);
    out.push(...expand(m[1] || '', m[2]));
  }
  return out;
}

function parseRange(range) {
  if (typeof range !== 'string') throw new TypeError('invalid range');
  return range.split('||').map(parseSet);
}

function test(c, v) {
  const r = compare(v, c.v);
  switch (c.op) {
    case '=': return r === 0;
    case '<': return r < 0;
    case '<=': return r <= 0;
    case '>': return r > 0;
    case '>=': return r >= 0;
  }
  return false;
}

function setAllows(set, v) {
  if (!set.every(c => test(c, v))) return false;
  if (!v.prerelease.length) return true;
  // A prerelease only matches when the set names a prerelease of the same major.minor.patch
  return set.some(c => c.v.prerelease.length && c.v.major === v.major && c.v.minor === v.minor && c.v.patch === v.patch);
}

function satisfies(version, range) {
  const sets = parseRange(range);
  const v = parse(version);
  if (!v) return false;
  return sets.some(s => setAllows(s, v));
}

function maxSatisfying(versions, range) {
  const sets = parseRange(range);
  let best = null;
  for (const text of versions) {
    const v = parse(text);
    if (!v || !sets.some(s => setAllows(s, v))) continue;
    if (best === null || compare(v, best.v) > 0) best = { text, v };
  }
  return best && best.text;
}

function inc(version, release) {
  const v = must(version);
  const pre = v.prerelease;
  switch (release) {
    case 'major':
      // 2.0.0-rc.1 -> 2.0.0: a prerelease of a major release is finished by it
      return format(v.minor === 0 && v.patch === 0 && pre.length ? ver(v.major, 0, 0) : ver(v.major + 1, 0, 0));
    case 'minor':
      return format(v.patch === 0 && pre.length ? ver(v.major, v.minor, 0) : ver(v.major, v.minor + 1, 0));
    case 'patch':
      return format(pre.length ? ver(v.major, v.minor, v.patch) : ver(v.major, v.minor, v.patch + 1));
    case 'prerelease': {
      if (!pre.length) return format(ver(v.major, v.minor, v.patch + 1, [0]));
      const next = pre.slice();
      let i = next.length - 1;
      while (i >= 0 && typeof next[i] !== 'number') i--;
      if (i < 0) next.push(0);
      else next[i] += 1;
      return format(ver(v.major, v.minor, v.patch, next));
    }
    default:
      throw new TypeError(`invalid release type: ${release}`);
  }
}

module.exports = { parse, format, compare, satisfies, maxSatisfying, inc };
