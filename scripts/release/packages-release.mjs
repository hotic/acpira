// The `packages` Release: one prerelease holding every version's files nobody downloads by hand (the desktop app's update
// packages and deltas, and the VSIXes, which install from the marketplaces and which an engine fetches for an SSH host of
// another platform, rust/crates/acpira-host/src/remote/fetch.rs), so each version's own Release lists only downloads.
// A prerelease never becomes the latest Release, and release.yml skips prereleases when this one is created.
//
//   node scripts/release/packages-release.mjs upload <file>...   create the Release when missing, upload files it lacks
//   node scripts/release/packages-release.mjs prune <count>      drop files of versions outside the newest <count> stable Releases
//
// Needs gh with GH_TOKEN and GH_REPO. Every name carries its version (a delta's is the one it leads to), so files of all
// versions share the one Release; GitHub caps a Release at 1000 files, which pruning keeps it well under.
import { execFileSync } from 'node:child_process';
import { basename } from 'node:path';

const TAG = 'packages';
const NOTES =
  'Files the apps and the release pipeline fetch: desktop update packages and deltas, and the VS Code extension packages. ' +
  'Downloads for people are on each version’s Release, whose SHA256SUMS lists these files too.';

const gh = (...args) => execFileSync('gh', args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'] });

/** The version a file belongs to: the target of a delta (`Acpira-1.0.0-to-1.1.0-…`), else the first x.y.z in its name */
function versionOf(name) {
  return (name.match(/-to-(\d+\.\d+\.\d+)-/) ?? name.match(/(\d+\.\d+\.\d+)/))?.[1];
}

function assets() {
  return gh('release', 'view', TAG, '--json', 'assets', '--jq', '.assets[].name').split('\n').filter(Boolean);
}

function ensure() {
  try {
    gh('release', 'view', TAG, '--json', 'tagName');
    return;
  } catch {}
  try {
    gh('release', 'create', TAG, '--prerelease', '--title', 'Packages', '--notes', NOTES);
  } catch (e) {
    // Another job of the same run may have created it first
    gh('release', 'view', TAG, '--json', 'tagName');
  }
}

function upload(files) {
  ensure();
  const have = new Set(assets());
  // Keep files already there on retries instead of replacing a published package
  for (const f of files.filter(f => !have.has(basename(f)))) gh('release', 'upload', TAG, f);
}

function prune(count) {
  const tags = gh('release', 'list', '--exclude-drafts', '--exclude-pre-releases', '--limit', String(count), '--json', 'tagName', '--jq', '.[].tagName');
  const keep = new Set(tags.split('\n').filter(Boolean).map(t => t.replace(/^v/, '')));
  if (keep.size === 0) throw new Error('no stable Release found; refusing to prune');
  for (const name of assets()) {
    const v = versionOf(name);
    if (v && !keep.has(v)) {
      console.log(`drop ${name}`);
      gh('release', 'delete-asset', TAG, name, '--yes');
    }
  }
}

const [cmd, ...rest] = process.argv.slice(2);
if (cmd === 'upload' && rest.length) upload(rest);
else if (cmd === 'prune' && Number(rest[0]) > 0) prune(Number(rest[0]));
else {
  console.error('usage: packages-release.mjs upload <file>... | prune <count>');
  process.exit(2);
}
