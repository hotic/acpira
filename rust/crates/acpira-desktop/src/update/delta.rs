//! Bundles as file lists, and the two update packages built from them. The release workflow's pack tool and the app
//! share this code, so what one writes the other reads.
//!
//! - Full package: the bundle as a tar, zstd-compressed, paths relative to the bundle root.
//! - Delta: an uncompressed tar whose first entry is `index.json` (the `from` version's whole file list and, for every
//!   file of the new bundle, how to make it), then one `data/<n>` entry per file that needs bytes. Ops: `keep` copies an
//!   installed file with the same digest (a clone on APFS), `patch` is a zstd frame compressed with the installed file
//!   as reference prefix (zstd's `--patch-from`, long-distance matching on), `add` a plain zstd frame, `link` a symlink.
//!   A file missing from the new list is gone.
//!
//! What either package unpacks to must match the manifest's file list byte for byte (`verify_tree`): the bundle was
//! signed and notarized before it was packed, so the client verifies the signature and never signs anything itself

use std::collections::{HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::manifest::{FileEntry, hex};

pub const DELTA_SCHEMA: u32 = 1;
const INDEX: &str = "index.json";
const MAX_INDEX: u64 = 16 << 20;
/// zstd's largest window on 64-bit targets: a patch's reference prefix plus its output must fit in it
const WINDOW_LOG_MAX: u32 = 31;
/// The full package's window: repetitions across the whole bundle (the shell and the engine share code)
const FULL_WINDOW_LOG: u32 = 27;
/// clonefile(2)'s CLONE_NOFOLLOW (libc has no constant for it)
#[cfg(target_os = "macos")]
const CLONE_NOFOLLOW: u32 = 0x0001;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Index {
  pub schema: u32,
  pub from: String,
  pub to: String,
  /// The whole `from` bundle: an installed copy that differs where an op reads it falls back to the full package
  pub from_files: Vec<FileEntry>,
  pub files: Vec<DeltaFile>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeltaFile {
  #[serde(flatten)]
  pub entry: FileEntry,
  pub op: Op,
  /// The installed file a `keep` copies or a `patch` starts from
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub from: Option<String>,
  /// The tar entry holding a `patch` or `add`'s bytes
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub data: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Op {
  Keep,
  Patch,
  Add,
  Link,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct DeltaStats {
  pub keep: usize,
  pub patch: usize,
  pub add: usize,
  pub link: usize,
}

fn io_err(what: impl std::fmt::Display, e: impl std::fmt::Display) -> String {
  format!("{what}: {e}")
}

/// Every file and symlink under `root`, sorted by path; directories are implied by the paths
pub fn scan(root: &Path) -> Result<Vec<FileEntry>, String> {
  let mut out = vec![];
  walk(root, root, &mut out)?;
  out.sort_by(|a, b| a.path.cmp(&b.path));
  Ok(out)
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<FileEntry>) -> Result<(), String> {
  for e in fs::read_dir(dir).map_err(|e| io_err(dir.display(), e))? {
    let e = e.map_err(|e| io_err(dir.display(), e))?;
    let p = e.path();
    let meta = fs::symlink_metadata(&p).map_err(|e| io_err(p.display(), e))?;
    let rel = rel_path(root, &p)?;
    let ft = meta.file_type();
    if ft.is_symlink() {
      let target = fs::read_link(&p).map_err(|e| io_err(p.display(), e))?;
      let target = target.to_str().ok_or_else(|| format!("{rel}: link target is not UTF-8"))?.to_owned();
      out.push(FileEntry { path: rel, size: 0, sha256: String::new(), mode: 0, link: Some(target) });
    } else if ft.is_dir() {
      walk(root, &p, out)?;
    } else if ft.is_file() {
      let sha256 = sha256_file(&p).map_err(|e| io_err(p.display(), e))?;
      out.push(FileEntry { path: rel, size: meta.len(), sha256, mode: mode_of(&meta), link: None });
    } else {
      return Err(format!("{rel}: neither a file, a folder nor a symlink"));
    }
  }
  Ok(())
}

fn rel_path(root: &Path, p: &Path) -> Result<String, String> {
  let rel = p.strip_prefix(root).map_err(|_| format!("{} is outside {}", p.display(), root.display()))?;
  let parts: Option<Vec<&str>> = rel.components().map(|c| c.as_os_str().to_str()).collect();
  parts.map(|p| p.join("/")).ok_or_else(|| format!("{}: path is not UTF-8", p.display()))
}

#[cfg(unix)]
fn mode_of(meta: &fs::Metadata) -> u32 {
  use std::os::unix::fs::PermissionsExt;
  meta.permissions().mode() & 0o777
}

#[cfg(not(unix))]
fn mode_of(_meta: &fs::Metadata) -> u32 {
  0o644
}

pub fn sha256_file(p: &Path) -> io::Result<String> {
  let mut f = File::open(p)?;
  let mut h = Sha256::new();
  io::copy(&mut f, &mut h)?;
  Ok(hex(&h.finalize()))
}

/// `root` holds exactly `expected`: the same paths, sizes, digests, modes and link targets, nothing more
pub fn verify_tree(root: &Path, expected: &[FileEntry]) -> Result<(), String> {
  let got = scan(root)?;
  let want: HashMap<&str, &FileEntry> = expected.iter().map(|e| (e.path.as_str(), e)).collect();
  let have: HashMap<&str, &FileEntry> = got.iter().map(|e| (e.path.as_str(), e)).collect();
  for e in expected {
    match have.get(e.path.as_str()) {
      None => return Err(format!("{} is missing", e.path)),
      Some(g) if *g != e => return Err(format!("{} differs from the release", e.path)),
      _ => {}
    }
  }
  if let Some(extra) = got.iter().find(|g| !want.contains_key(g.path.as_str())) {
    return Err(format!("{} is not part of the release", extra.path));
  }
  Ok(())
}

/// A package path as a relative path: no absolute paths, no `.` or `..`, no backslashes, not empty
fn safe_rel(path: &str) -> Result<PathBuf, String> {
  let p = Path::new(path);
  if path.is_empty() || path.contains('\\') || !p.components().all(|c| matches!(c, Component::Normal(_))) {
    return Err(format!("unsafe path in update package: {path:?}"));
  }
  Ok(p.to_path_buf())
}

/// No folder on the way to `rel` under `root` is a symlink, so a write cannot leave `root` through one the package made
fn no_link_on_the_way(root: &Path, rel: &Path) -> Result<(), String> {
  let mut at = root.to_path_buf();
  let mut parts = rel.components().peekable();
  while let Some(c) = parts.next() {
    if parts.peek().is_none() {
      break;
    }
    at.push(c);
    if fs::symlink_metadata(&at).is_ok_and(|m| m.file_type().is_symlink()) {
      return Err(format!("update package writes through a symlink: {}", rel.display()));
    }
  }
  Ok(())
}

/// The output path of `rel` under `root`, its folders created
fn out_path(root: &Path, rel: &str) -> Result<PathBuf, String> {
  let rel = safe_rel(rel)?;
  no_link_on_the_way(root, &rel)?;
  let out = root.join(&rel);
  if let Some(parent) = out.parent() {
    fs::create_dir_all(parent).map_err(|e| io_err(parent.display(), e))?;
  }
  Ok(out)
}

fn create_new(p: &Path) -> Result<File, String> {
  OpenOptions::new().write(true).create_new(true).open(p).map_err(|e| io_err(p.display(), e))
}

#[cfg(unix)]
fn set_mode(p: &Path, mode: u32) -> Result<(), String> {
  use std::os::unix::fs::PermissionsExt;
  fs::set_permissions(p, fs::Permissions::from_mode(mode & 0o777)).map_err(|e| io_err(p.display(), e))
}

#[cfg(not(unix))]
fn set_mode(_p: &Path, _mode: u32) -> Result<(), String> {
  Ok(())
}

#[cfg(unix)]
fn make_link(target: &str, at: &Path) -> Result<(), String> {
  std::os::unix::fs::symlink(target, at).map_err(|e| io_err(at.display(), e))
}

#[cfg(not(unix))]
fn make_link(_target: &str, at: &Path) -> Result<(), String> {
  Err(format!("{}: symlinks are not supported here", at.display()))
}

/// A copy that shares the blocks on APFS (clonefile), a plain copy elsewhere or across volumes
fn clone_or_copy(src: &Path, dst: &Path) -> Result<(), String> {
  #[cfg(target_os = "macos")]
  {
    use std::os::unix::ffi::OsStrExt;
    if let (Ok(s), Ok(d)) = (std::ffi::CString::new(src.as_os_str().as_bytes()), std::ffi::CString::new(dst.as_os_str().as_bytes())) {
      // SAFETY: both are valid NUL-terminated paths for the duration of the call
      if unsafe { libc::clonefile(s.as_ptr(), d.as_ptr(), CLONE_NOFOLLOW) } == 0 {
        return Ok(());
      }
    }
  }
  fs::copy(src, dst).map(|_| ()).map_err(|e| io_err(dst.display(), e))
}

fn tar_header(kind: tar::EntryType, size: u64, mode: u32) -> tar::Header {
  let mut h = tar::Header::new_gnu();
  h.set_entry_type(kind);
  h.set_size(size);
  h.set_mode(mode);
  h.set_mtime(0);
  h
}

fn append_bytes<W: Write>(tar: &mut tar::Builder<W>, name: &str, bytes: &[u8]) -> Result<(), String> {
  let mut h = tar_header(tar::EntryType::Regular, bytes.len() as u64, 0o644);
  tar.append_data(&mut h, name, bytes).map_err(|e| io_err(name, e))
}

/// Writes `root` as a full package to `out` and returns its file list (the manifest's `files`)
pub fn pack_full(root: &Path, out: &Path, level: i32) -> Result<Vec<FileEntry>, String> {
  let files = scan(root)?;
  let f = File::create(out).map_err(|e| io_err(out.display(), e))?;
  let wrap = |e: io::Error| io_err(out.display(), e);
  let mut enc = zstd::stream::write::Encoder::new(BufWriter::new(f), level).map_err(wrap)?;
  enc.long_distance_matching(true).map_err(wrap)?;
  enc.window_log(FULL_WINDOW_LOG).map_err(wrap)?;
  let mut tar = tar::Builder::new(enc);
  for e in &files {
    match &e.link {
      Some(target) => {
        let mut h = tar_header(tar::EntryType::Symlink, 0, 0o755);
        tar.append_link(&mut h, &e.path, target).map_err(|x| io_err(&e.path, x))?;
      }
      None => {
        let mut h = tar_header(tar::EntryType::Regular, e.size, e.mode);
        let src = File::open(root.join(&e.path)).map_err(|x| io_err(&e.path, x))?;
        tar.append_data(&mut h, &e.path, src).map_err(|x| io_err(&e.path, x))?;
      }
    }
  }
  let enc = tar.into_inner().map_err(wrap)?;
  let w = enc.finish().map_err(wrap)?;
  w.into_inner().map_err(|e| io_err(out.display(), e.into_error()))?.sync_all().map_err(wrap)?;
  Ok(files)
}

/// Unpacks a full package into `dest`, which must not exist yet (its parent must). Regular files, folders and symlinks
/// only; the caller verifies the result against the manifest
pub fn unpack_full(archive: &Path, dest: &Path) -> Result<(), String> {
  let f = File::open(archive).map_err(|e| io_err(archive.display(), e))?;
  let dec = zstd::stream::read::Decoder::new(f).map_err(|e| io_err(archive.display(), e))?;
  fs::create_dir(dest).map_err(|e| io_err(dest.display(), e))?;
  let mut ar = tar::Archive::new(dec);
  for entry in ar.entries().map_err(|e| io_err(archive.display(), e))? {
    let mut entry = entry.map_err(|e| io_err(archive.display(), e))?;
    let path = entry.path().map_err(|e| io_err(archive.display(), e))?.to_str().map(str::to_owned);
    let path = path.ok_or("update package path is not UTF-8")?;
    let path = path.trim_end_matches('/').to_owned();
    let kind = entry.header().entry_type();
    if kind.is_dir() {
      let rel = safe_rel(&path)?;
      no_link_on_the_way(dest, &rel)?;
      fs::create_dir_all(dest.join(&rel)).map_err(|e| io_err(&path, e))?;
      continue;
    }
    let out = out_path(dest, &path)?;
    if kind.is_symlink() {
      let target = entry.link_name().map_err(|e| io_err(&path, e))?.ok_or_else(|| format!("{path}: link without a target"))?;
      make_link(target.to_str().ok_or_else(|| format!("{path}: link target is not UTF-8"))?, &out)?;
    } else if kind.is_file() {
      let mode = entry.header().mode().map_err(|e| io_err(&path, e))?;
      let mut file = create_new(&out)?;
      io::copy(&mut entry, &mut file).map_err(|e| io_err(&path, e))?;
      set_mode(&out, mode)?;
    } else {
      return Err(format!("{path}: unsupported entry in update package"));
    }
  }
  Ok(())
}

/// `dir/name-<8 char hash>.ext` as `dir/name.ext`: Vite's chunk names, so a changed chunk is patched from its
/// predecessor; None for a name without such a hash
fn hashed_stem(path: &str) -> Option<String> {
  let (dir, file) = path.rsplit_once('/').map_or(("", path), |(d, f)| (d, f));
  let (base, ext) = file.split_once('.')?;
  let (name, hash) = base.rsplit_once('-')?;
  let hashy = hash.len() == 8 && hash.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
  (hashy && !name.is_empty()).then(|| format!("{dir}/{name}.{ext}"))
}

/// The smallest window that holds `n` bytes (a patch's reference and output together)
fn window_log_for(n: usize) -> u32 {
  let mut log = 17;
  while log < WINDOW_LOG_MAX && (1usize << log) < n {
    log += 1;
  }
  log
}

fn patch_bytes(reference: &[u8], new: &[u8], level: i32) -> io::Result<Vec<u8>> {
  let mut enc = zstd::stream::write::Encoder::with_ref_prefix(Vec::new(), level, reference)?;
  enc.long_distance_matching(true)?;
  enc.window_log(window_log_for(reference.len() + new.len()))?;
  enc.include_checksum(true)?;
  enc.write_all(new)?;
  enc.finish()
}

fn add_bytes(new: &[u8], level: i32) -> io::Result<Vec<u8>> {
  let mut enc = zstd::stream::write::Encoder::new(Vec::new(), level)?;
  enc.long_distance_matching(true)?;
  enc.window_log(window_log_for(new.len()))?;
  enc.include_checksum(true)?;
  enc.write_all(new)?;
  enc.finish()
}

/// Writes the delta from the bundle at `old_root` (version `from`) to the one at `new_root` (version `to`)
pub fn make_delta(old_root: &Path, new_root: &Path, from: &str, to: &str, out: &Path, level: i32) -> Result<DeltaStats, String> {
  let old = scan(old_root)?;
  let new = scan(new_root)?;
  let by_path: HashMap<&str, &FileEntry> = old.iter().filter(|e| e.link.is_none()).map(|e| (e.path.as_str(), e)).collect();
  let mut by_hash: HashMap<&str, &str> = HashMap::new();
  let mut by_stem: HashMap<String, &str> = HashMap::new();
  for e in old.iter().filter(|e| e.link.is_none()) {
    by_hash.entry(e.sha256.as_str()).or_insert(e.path.as_str());
    if let Some(s) = hashed_stem(&e.path) {
      by_stem.insert(s, e.path.as_str());
    }
  }
  let mut stats = DeltaStats::default();
  let mut files = vec![];
  let mut data: Vec<(String, Vec<u8>)> = vec![];
  for e in &new {
    let mut f = DeltaFile { entry: e.clone(), op: Op::Link, from: None, data: None };
    if e.link.is_some() {
      stats.link += 1;
      files.push(f);
      continue;
    }
    // The same bytes anywhere in the old bundle; the same path first
    let same = by_path
      .get(e.path.as_str())
      .filter(|o| o.sha256 == e.sha256)
      .map(|o| o.path.as_str())
      .or_else(|| by_hash.get(e.sha256.as_str()).copied());
    if let Some(src) = same {
      f.op = Op::Keep;
      f.from = Some(src.to_owned());
      stats.keep += 1;
      files.push(f);
      continue;
    }
    let bytes = fs::read(new_root.join(&e.path)).map_err(|x| io_err(&e.path, x))?;
    let reference =
      by_path.get(e.path.as_str()).map(|o| o.path.as_str()).or_else(|| hashed_stem(&e.path).and_then(|s| by_stem.get(&s).copied()));
    let mut chosen: Option<(Op, Vec<u8>, Option<String>)> = None;
    if let Some(r) = reference {
      let old_bytes = fs::read(old_root.join(r)).map_err(|x| io_err(r, x))?;
      let p = patch_bytes(&old_bytes, &bytes, level).map_err(|x| io_err(&e.path, x))?;
      chosen = Some((Op::Patch, p, Some(r.to_owned())));
    }
    // A patch under half the file is kept without trying a plain copy; otherwise the smaller of the two
    let (op, bytes, src) = match chosen {
      Some(patch) if (patch.1.len() as u64) < e.size / 2 => patch,
      patch => {
        let a = add_bytes(&bytes, level).map_err(|x| io_err(&e.path, x))?;
        match patch {
          Some(patch) if patch.1.len() < a.len() => patch,
          _ => (Op::Add, a, None),
        }
      }
    };
    match op {
      Op::Patch => stats.patch += 1,
      _ => stats.add += 1,
    }
    let name = format!("data/{}", data.len());
    f.op = op;
    f.from = src;
    f.data = Some(name.clone());
    data.push((name, bytes));
    files.push(f);
  }
  let index = Index { schema: DELTA_SCHEMA, from: from.to_owned(), to: to.to_owned(), from_files: old, files };
  let json = serde_json::to_vec(&index).map_err(|e| io_err("index", e))?;
  let file = File::create(out).map_err(|e| io_err(out.display(), e))?;
  let mut tar = tar::Builder::new(BufWriter::new(file));
  append_bytes(&mut tar, INDEX, &json)?;
  for (name, bytes) in &data {
    append_bytes(&mut tar, name, bytes)?;
  }
  let w = tar.into_inner().map_err(|e| io_err(out.display(), e))?;
  w.into_inner().map_err(|e| io_err(out.display(), e.into_error()))?.sync_all().map_err(|e| io_err(out.display(), e))?;
  Ok(stats)
}

/// Reads a delta's index without applying it
pub fn read_index(delta: &Path) -> Result<Index, String> {
  let f = File::open(delta).map_err(|e| io_err(delta.display(), e))?;
  let mut ar = tar::Archive::new(BufReader::new(f));
  let mut entries = ar.entries().map_err(|e| io_err(delta.display(), e))?;
  let first = entries.next().ok_or("the delta is empty")?.map_err(|e| io_err(delta.display(), e))?;
  index_of(first)
}

fn index_of<R: Read>(first: tar::Entry<'_, R>) -> Result<Index, String> {
  let name = first.path().map_err(|e| io_err("delta", e))?.to_string_lossy().into_owned();
  if name != INDEX || first.size() > MAX_INDEX {
    return Err("the delta does not start with its index".into());
  }
  let index: Index = serde_json::from_reader(first.take(MAX_INDEX)).map_err(|e| io_err("delta index", e))?;
  if index.schema != DELTA_SCHEMA {
    return Err(format!("delta schema {} (this build reads {DELTA_SCHEMA})", index.schema));
  }
  Ok(index)
}

/// The installed file a delta op reads: `rel` under the installed bundle, or, when the install is one file (Windows'
/// Acpira.exe, under whatever name it was saved), that file itself for its tree's only, top-level entry
fn installed_file(installed: &Path, rel: &str) -> Result<PathBuf, String> {
  let rel_path = safe_rel(rel)?;
  if !fs::metadata(installed).is_ok_and(|m| m.is_file()) {
    return Ok(installed.join(rel_path));
  }
  if rel_path.components().count() == 1 { Ok(installed.to_path_buf()) } else { Err(format!("the delta reads {rel}, which a one-file install lacks")) }
}

/// Builds the new bundle in `dest` (which must not exist; its parent must) from the installed bundle (or exe) and the delta.
/// Fails, leaving `dest` for the caller to remove, when an installed file an op reads differs from the delta's `from`
/// list; the caller then takes the full package. The result still has to pass `verify_tree`
pub fn apply_delta(delta: &Path, installed: &Path, dest: &Path) -> Result<Index, String> {
  let f = File::open(delta).map_err(|e| io_err(delta.display(), e))?;
  let mut ar = tar::Archive::new(BufReader::new(f));
  let mut entries = ar.entries().map_err(|e| io_err(delta.display(), e))?;
  let first = entries.next().ok_or("the delta is empty")?.map_err(|e| io_err(delta.display(), e))?;
  let index = index_of(first)?;
  // Every installed file an op reads must be the one the delta was made from
  let from: HashMap<&str, &FileEntry> = index.from_files.iter().map(|e| (e.path.as_str(), e)).collect();
  let mut checked: HashSet<&str> = HashSet::new();
  for f in &index.files {
    let Some(src) = f.from.as_deref() else { continue };
    if !checked.insert(src) {
      continue;
    }
    let want = from.get(src).filter(|e| e.link.is_none()).ok_or_else(|| format!("the delta reads {src}, which its from list lacks"))?;
    let p = installed_file(installed, src)?;
    let is_file = fs::symlink_metadata(&p).is_ok_and(|m| m.file_type().is_file());
    let sha = if is_file { sha256_file(&p).ok() } else { None };
    if sha.as_deref() != Some(want.sha256.as_str()) {
      return Err(format!("the installed {src} is not the one {} shipped", index.from));
    }
  }
  fs::create_dir(dest).map_err(|e| io_err(dest.display(), e))?;
  let mut by_data: HashMap<String, &DeltaFile> = HashMap::new();
  for f in &index.files {
    match f.op {
      Op::Keep => {
        let src = f.from.as_deref().ok_or_else(|| format!("{}: keep without a source", f.entry.path))?;
        let out = out_path(dest, &f.entry.path)?;
        clone_or_copy(&installed_file(installed, src)?, &out)?;
        set_mode(&out, f.entry.mode)?;
      }
      Op::Link => {
        let target = f.entry.link.as_deref().ok_or_else(|| format!("{}: link without a target", f.entry.path))?;
        make_link(target, &out_path(dest, &f.entry.path)?)?;
      }
      Op::Patch | Op::Add => {
        let name = f.data.clone().ok_or_else(|| format!("{}: no data entry", f.entry.path))?;
        if by_data.insert(name, f).is_some() {
          return Err(format!("{}: data entry used twice", f.entry.path));
        }
      }
    }
  }
  for entry in entries {
    let entry = entry.map_err(|e| io_err(delta.display(), e))?;
    let name = entry.path().map_err(|e| io_err(delta.display(), e))?.to_string_lossy().into_owned();
    let f = by_data.remove(&name).ok_or_else(|| format!("unexpected delta entry {name}"))?;
    let out = out_path(dest, &f.entry.path)?;
    let mut file = create_new(&out)?;
    let reader = BufReader::new(entry);
    let copied = if f.op == Op::Patch {
      let src = f.from.as_deref().ok_or_else(|| format!("{}: patch without a source", f.entry.path))?;
      let reference = fs::read(installed_file(installed, src)?).map_err(|e| io_err(src, e))?;
      let mut dec = zstd::stream::read::Decoder::with_ref_prefix(reader, &reference).map_err(|e| io_err(&f.entry.path, e))?;
      dec.window_log_max(WINDOW_LOG_MAX).map_err(|e| io_err(&f.entry.path, e))?;
      io::copy(&mut dec, &mut file)
    } else {
      let mut dec = zstd::stream::read::Decoder::with_buffer(reader).map_err(|e| io_err(&f.entry.path, e))?;
      dec.window_log_max(WINDOW_LOG_MAX).map_err(|e| io_err(&f.entry.path, e))?;
      io::copy(&mut dec, &mut file)
    };
    copied.map_err(|e| io_err(&f.entry.path, e))?;
    set_mode(&out, f.entry.mode)?;
  }
  if let Some(name) = by_data.keys().next() {
    return Err(format!("the delta lacks {name}"));
  }
  Ok(index)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn put(root: &Path, rel: &str, bytes: &[u8], mode: u32) {
    let p = root.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(&p, bytes).unwrap();
    set_mode(&p, mode).unwrap();
  }

  /// A pseudo-random "binary" whose later versions differ in a few places, as two builds of the engine do
  fn binary(seed: u64, len: usize, edits: &[(usize, &[u8])]) -> Vec<u8> {
    let mut x = seed;
    let mut v: Vec<u8> = (0..len)
      .map(|_| {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        (x >> 24) as u8
      })
      .collect();
    for (at, bytes) in edits {
      v[*at..*at + bytes.len()].copy_from_slice(bytes);
    }
    v
  }

  /// The old and new bundle of one release step: an unchanged file, a patched binary, a renamed Vite chunk, a removed
  /// file, a new file, an executable bit that changed, and a symlink
  fn bundles(dir: &Path) -> (PathBuf, PathBuf) {
    let (old, new) = (dir.join("old.app"), dir.join("new.app"));
    let engine = binary(1, 300_000, &[]);
    put(&old, "Contents/Info.plist", b"<plist>1.0</plist>", 0o644);
    put(&old, "Contents/MacOS/acpira-engine", &engine, 0o755);
    put(&old, "Contents/Resources/webview/assets/main-AbCd1234.js", &binary(2, 40_000, &[]), 0o644);
    put(&old, "Contents/Resources/webview/assets/font.woff2", b"font", 0o644);
    put(&old, "Contents/Resources/gone.txt", b"bye", 0o644);
    put(&old, "Contents/Resources/script.sh", b"#!/bin/sh\n", 0o644);
    put(&new, "Contents/Info.plist", b"<plist>1.1</plist>", 0o644);
    put(&new, "Contents/MacOS/acpira-engine", &binary(1, 300_000, &[(1000, b"new code"), (200_000, b"more")]), 0o755);
    put(&new, "Contents/Resources/webview/assets/main-ZyXw9876.js", &binary(2, 40_000, &[(10, b"edit")]), 0o644);
    put(&new, "Contents/Resources/webview/assets/font.woff2", b"font", 0o644);
    put(&new, "Contents/Resources/webview/assets/new-chunk.js", b"console.log('new')", 0o644);
    put(&new, "Contents/Resources/script.sh", b"#!/bin/sh\n", 0o755);
    #[cfg(unix)]
    {
      fs::create_dir_all(new.join("Contents/Frameworks")).unwrap();
      std::os::unix::fs::symlink("../MacOS/acpira-engine", new.join("Contents/Frameworks/engine")).unwrap();
    }
    (old, new)
  }

  #[test]
  fn a_delta_rebuilds_the_new_bundle_exactly_from_the_installed_one() {
    let d = tempfile::tempdir().unwrap();
    let (old, new) = bundles(d.path());
    let delta = d.path().join("step.delta");
    let stats = make_delta(&old, &new, "1.0.0", "1.1.0", &delta, 3).unwrap();
    // Unchanged font kept, script kept (same bytes, new mode); the plist goes whichever way is smaller
    assert_eq!((stats.keep, stats.patch + stats.add, stats.link), (2, 4, if cfg!(unix) { 1 } else { 0 }), "{stats:?}");
    let index = read_index(&delta).unwrap();
    let op_of = |suffix: &str| index.files.iter().find(|f| f.entry.path.ends_with(suffix)).unwrap().op;
    assert_eq!((op_of("acpira-engine"), op_of("new-chunk.js"), op_of("script.sh")), (Op::Patch, Op::Add, Op::Keep));
    let chunk = index.files.iter().find(|f| f.entry.path.ends_with("main-ZyXw9876.js")).unwrap();
    assert_eq!((chunk.op, chunk.from.as_deref()), (Op::Patch, Some("Contents/Resources/webview/assets/main-AbCd1234.js")));
    // The binary edit costs far less than the binary
    assert!(fs::metadata(&delta).unwrap().len() < 60_000, "{}", fs::metadata(&delta).unwrap().len());
    let out = d.path().join("staged.app");
    apply_delta(&delta, &old, &out).unwrap();
    verify_tree(&out, &scan(&new).unwrap()).unwrap();
    assert!(!out.join("Contents/Resources/gone.txt").exists());
    #[cfg(unix)]
    {
      use std::os::unix::fs::PermissionsExt;
      assert_eq!(fs::metadata(out.join("Contents/Resources/script.sh")).unwrap().permissions().mode() & 0o777, 0o755);
      assert_eq!(fs::read_link(out.join("Contents/Frameworks/engine")).unwrap(), Path::new("../MacOS/acpira-engine"));
    }
  }

  #[test]
  fn an_installed_bundle_that_differs_where_the_delta_reads_it_is_refused() {
    let d = tempfile::tempdir().unwrap();
    let (old, new) = bundles(d.path());
    let delta = d.path().join("step.delta");
    make_delta(&old, &new, "1.0.0", "1.1.0", &delta, 3).unwrap();
    // A modified engine (another version, a local edit) cannot be the reference of its patch
    put(&old, "Contents/MacOS/acpira-engine", &binary(1, 300_000, &[(5, b"tampered")]), 0o755);
    let e = apply_delta(&delta, &old, &d.path().join("a.app")).unwrap_err();
    assert!(e.contains("acpira-engine") && e.contains("1.0.0"), "{e}");
    // A missing file too
    fs::remove_file(old.join("Contents/Resources/webview/assets/font.woff2")).unwrap();
    assert!(apply_delta(&delta, &old, &d.path().join("b.app")).is_err());
    // A file the delta never reads may differ: the result is verified against the release anyway
    let (old2, _) = bundles(&d.path().join("again"));
    put(&old2, "Contents/Resources/gone.txt", b"changed but unused", 0o644);
    let out = d.path().join("c.app");
    apply_delta(&delta, &old2, &out).unwrap();
    verify_tree(&out, &scan(&new).unwrap()).unwrap();
  }

  #[test]
  fn a_one_file_install_is_patched_from_the_exe_under_any_name() {
    let d = tempfile::tempdir().unwrap();
    let (old, new) = (d.path().join("old"), d.path().join("new"));
    put(&old, "Acpira.exe", &binary(5, 200_000, &[]), 0o644);
    put(&new, "Acpira.exe", &binary(5, 200_000, &[(7000, b"1.1.0")]), 0o644);
    let delta = d.path().join("exe.delta");
    let stats = make_delta(&old, &new, "1.0.0", "1.1.0", &delta, 3).unwrap();
    assert_eq!(stats.patch, 1, "{stats:?}");
    // The browser saved it under another name; the tree still holds Acpira.exe
    let installed = d.path().join("Downloads/Acpira (1).exe");
    fs::create_dir_all(installed.parent().unwrap()).unwrap();
    fs::copy(old.join("Acpira.exe"), &installed).unwrap();
    let out = d.path().join("staged");
    apply_delta(&delta, &installed, &out).unwrap();
    verify_tree(&out, &scan(&new).unwrap()).unwrap();
    // Another build than the one 1.0.0 shipped is refused (the full package is taken)
    fs::write(&installed, binary(6, 200_000, &[])).unwrap();
    assert!(apply_delta(&delta, &installed, &d.path().join("again")).unwrap_err().contains("is not the one 1.0.0 shipped"));
  }

  // Checks permission bits, which Windows does not keep
  #[cfg(unix)]
  #[test]
  fn the_full_package_round_trips_and_verify_tree_names_any_difference() {
    let d = tempfile::tempdir().unwrap();
    let (_, new) = bundles(d.path());
    let pkg = d.path().join("full.tar.zst");
    let files = pack_full(&new, &pkg, 3).unwrap();
    assert_eq!(files, scan(&new).unwrap());
    let out = d.path().join("unpacked.app");
    unpack_full(&pkg, &out).unwrap();
    verify_tree(&out, &files).unwrap();
    // A changed byte, a changed mode, an extra file, a missing file
    put(&out, "Contents/Info.plist", b"<plist>1.X</plist>", 0o644);
    assert!(verify_tree(&out, &files).unwrap_err().contains("Info.plist"));
    put(&out, "Contents/Info.plist", b"<plist>1.1</plist>", 0o600);
    assert!(verify_tree(&out, &files).unwrap_err().contains("Info.plist"));
    put(&out, "Contents/Info.plist", b"<plist>1.1</plist>", 0o644);
    put(&out, "Contents/extra", b"x", 0o644);
    assert!(verify_tree(&out, &files).unwrap_err().contains("not part of the release"));
    fs::remove_file(out.join("Contents/extra")).unwrap();
    fs::remove_file(out.join("Contents/Info.plist")).unwrap();
    assert!(verify_tree(&out, &files).unwrap_err().contains("missing"));
  }

  #[test]
  fn packages_cannot_write_outside_their_folder() {
    #[cfg(unix)]
    let d = tempfile::tempdir().unwrap();
    for bad in ["../evil", "/etc/evil", "a/../../evil", "a\\b", "", "./a"] {
      assert!(safe_rel(bad).is_err(), "{bad:?}");
    }
    // A symlink the package made, then a file written through it
    #[cfg(unix)]
    {
      let outside = d.path().join("outside");
      fs::create_dir_all(&outside).unwrap();
      let mut raw = vec![];
      {
        let enc = zstd::stream::write::Encoder::new(&mut raw, 3).unwrap();
        let mut tar = tar::Builder::new(enc);
        let mut h = tar_header(tar::EntryType::Symlink, 0, 0o755);
        tar.append_link(&mut h, "Contents", outside.to_str().unwrap()).unwrap();
        append_bytes(&mut tar, "Contents/evil", b"x").unwrap();
        tar.into_inner().unwrap().finish().unwrap();
      }
      let pkg = d.path().join("evil.tar.zst");
      fs::write(&pkg, raw).unwrap();
      let e = unpack_full(&pkg, &d.path().join("out.app")).unwrap_err();
      assert!(e.contains("through a symlink"), "{e}");
      assert!(!outside.join("evil").exists());
    }
  }

  #[test]
  fn vite_chunk_names_lose_their_hash() {
    assert_eq!(hashed_stem("w/assets/main-AbCd1234.js").as_deref(), Some("w/assets/main.js"));
    assert_eq!(hashed_stem("w/assets/chunk-IMKFNOWR-Cyx7hdER.js").as_deref(), Some("w/assets/chunk-IMKFNOWR.js"));
    assert_eq!(hashed_stem("w/assets/main.js"), None);
    assert_eq!(hashed_stem("w/assets/new-chunk.js"), None);
    assert_eq!(hashed_stem("noext-AbCd1234"), None);
  }
}
