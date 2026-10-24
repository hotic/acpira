//! `read`: a text file with line numbers, a window at a time. The header always states the total line count, so the
//! model knows whether it saw everything. `paths` adds more files, or more windows, to the same call: a model that
//! makes one call per reply (GPT-6.1 on every harness, 2026-10) otherwise spends a request per file. A path may carry a
//! line range the way grep prints a hit (`src/a.rs:120-180`, or `src/a.rs:120` from that line on), and the windows of
//! one call share the output budget evenly, so a large first file no longer crowds out the rest (before 2026-10-18 the
//! first file took what it needed and the others what was left: 17 of 63 multi-file reads in the local logs ran out).
//! A PNG, JPEG, GIF or WebP file is read as the picture itself (2026-10-24, as Claude Code's and OpenCode's read do):
//! it goes on the tool card, and to the model when the model takes images; before, the model was told only that the
//! file was binary and had no way at all to look at a local image. A PDF is read as its text, page by page
//! (`pdf.rs`): for a PDF, offset and limit (and a range in the path, or `pages`) count pages

use std::path::{Path, PathBuf};

use base64::Engine as _;
use serde_json::{Value, json};

use super::{Action, Ctx, Output, num_arg, resolve, str_arg};
use crate::budget::MAX_BYTES;
use crate::llm::ToolSpec;

pub const DEFAULT_LIMIT: usize = 2000;
/// Windows one call may read
pub const MAX_PATHS: usize = 12;
/// A longer line is cut, with a marker
const MAX_LINE: usize = 2000;
/// Encoded image bytes one call may send: Anthropic refuses a request over 32 MB, and the rest of the history needs room
const MAX_CALL_IMAGE_BYTES: usize = 16 * 1024 * 1024;
/// A picture wider or taller than this is not sent: Anthropic refuses one over 8000 pixels on a side, and a refused
/// image would stay in the history and fail every later request of the session
const MAX_IMAGE_SIDE: u32 = 8000;

/// One window of a file
#[derive(Debug, Clone, PartialEq)]
pub struct Slice {
  pub path: PathBuf,
  /// First line, 1-based
  pub offset: usize,
  pub limit: usize,
}

pub fn spec() -> ToolSpec {
  ToolSpec {
    name: super::READ.into(),
    description: "Read a text file. Returns numbered lines (up to 2000 per call) and the file's total line count; use offset / limit for \
                  the next window. Relative paths resolve against the session folder. To look at several files or places, list them in \
                  paths, each whole or with a line range as grep prints it (src/a.rs:120-180, or src/a.rs:120 from that line on): one \
                  call reads them all and shares its output budget evenly between them. A PNG, JPEG, GIF or WebP file comes back as \
                  the image itself, with its size in pixels, so you can look at screenshots, photos and video frames. A PDF comes back \
                  as its text, page by page: for a PDF, offset / limit and a range in the path count pages (or pass pages, e.g. 3-5)."
      .into(),
    parameters: json!({
      "type": "object",
      "properties": {
        "path": { "type": "string", "description": "File path, absolute or relative to the session folder" },
        "offset": { "type": "integer", "description": "First line to read, 1-based (default 1)" },
        "limit": { "type": "integer", "description": "How many lines to read (default 2000)" },
        "pages": { "type": "string", "description": "For a PDF in path: the pages to read, e.g. 3-5, 3 or 10- (default: from the first page as far as the output allows)" },
        "paths": { "type": "array", "items": { "type": "string" }, "description": "More files to read in the same call: path, path:start-end or path:start" },
      },
      "required": ["path"],
    }),
  }
}

/// The first line of a range and its last, None for the rest of the file
type Range = (usize, Option<usize>);

/// A path as the model gave it, with the line range it carries: `file:N-M` is lines N to M, `file:N` from line N on
/// (also `file:N:`, copied from a grep hit). A file whose name really ends that way is taken as named
fn split_range(raw: &str, cwd: &Path) -> Result<(PathBuf, Option<Range>), String> {
  let whole = resolve(raw, cwd);
  let Some((file, range)) = raw.strip_suffix(':').unwrap_or(raw).rsplit_once(':').filter(|_| !whole.exists()) else { return Ok((whole, None)) };
  let (start, end) = match range.split_once('-') {
    Some((a, b)) => (a, Some(b)),
    None => (range, None),
  };
  let Ok(start) = start.trim().parse::<usize>() else { return Ok((whole, None)) };
  let end = match end.map(|e| e.trim().parse::<usize>()) {
    None => None,
    Some(Ok(e)) => Some(e),
    Some(Err(_)) => return Ok((whole, None)),
  };
  let start = start.max(1);
  if end.is_some_and(|e| e < start) {
    return Err(format!("{raw}: the line range ends before it starts"));
  }
  Ok((resolve(file, cwd), Some((start, end))))
}

/// A path argument as a window: `offset` / `limit` when the call gives either, else the range the path carries, else
/// the whole file. The bool says whether the window came from the path
fn window(raw: &str, cwd: &Path, offset: Option<usize>, limit: Option<usize>) -> Result<(Slice, bool), String> {
  let (path, range) = split_range(raw, cwd)?;
  if offset.is_some() || limit.is_some() {
    return Ok((Slice { path, offset: offset.unwrap_or(1).max(1), limit: limit.unwrap_or(DEFAULT_LIMIT) }, false));
  }
  Ok(match range {
    Some((start, end)) => (Slice { path, offset: start, limit: end.map_or(DEFAULT_LIMIT, |e| e + 1 - start) }, true),
    None => (Slice { path, offset: 1, limit: DEFAULT_LIMIT }, false),
  })
}

/// The `paths` strings; a lone string, or an array sent as a JSON string, is taken too
fn more_paths(args: &Value) -> Vec<String> {
  let parsed;
  let list = match args.get("paths") {
    Some(Value::String(s)) => match serde_json::from_str::<Value>(s) {
      Ok(v @ Value::Array(_)) => {
        parsed = v;
        &parsed
      }
      _ => return vec![s.clone()].into_iter().filter(|p| !p.trim().is_empty()).collect(),
    },
    Some(v) => v,
    None => return vec![],
  };
  list.as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_owned).collect()
}

/// `images`: the turn's model takes images, so an image file's pixels go to it
pub fn prepare(args: &Value, cwd: &Path, images: bool) -> Result<Action, String> {
  let first = str_arg(args, "path").or_else(|_| str_arg(args, "file_path"))?;
  let mut offset = num_arg(args, "offset").map(|o| (o as usize).max(1));
  let mut limit = num_arg(args, "limit").filter(|l| *l > 0).map(|l| l as usize);
  // A PDF's pages, as Claude Code's read takes them: the window of the first file, unless offset / limit set one
  if let Some(pages) = args.get("pages").and_then(Value::as_str).filter(|_| offset.is_none() && limit.is_none()) {
    let (start, end) = page_range(pages)?;
    offset = Some(start);
    limit = end.map(|e| e + 1 - start);
  }
  let (head, head_ranged) = window(first, cwd, offset, limit)?;
  let bare = !head_ranged && offset.is_none() && limit.is_none();
  let mut files = vec![head];
  for (i, raw) in more_paths(args).iter().enumerate() {
    let (s, ranged) = window(raw, cwd, None, None).map_err(|e| format!("paths[{i}]: {e}"))?;
    // Models repeat the first path in the list; a range of the same file is another window
    if (!ranged && s.path == files[0].path) || files.contains(&s) {
      continue;
    }
    files.push(s);
  }
  // A bare `path` beside ranges of the same file only fills the required field: Gemini 4 Argon sends the file there
  // and the lines it wants in `paths` (2026-10-23, 29 of 43 reads in one session), and reading the file's start as
  // well spent most of each call's budget on lines nobody asked for
  if bare && files[1..].iter().any(|s| s.path == files[0].path) {
    files.remove(0);
  }
  if files.len() > MAX_PATHS {
    return Err(format!("One read takes at most {MAX_PATHS} files; split the rest into another call"));
  }
  Ok(Action::Read { files, images })
}

/// `pages` as the first page and the last one (None: to the end): `3-5`, `3` (that page alone) or `10-`
fn page_range(raw: &str) -> Result<Range, String> {
  let bad = || format!("pages \"{raw}\" is not a page range such as 3-5, 3 or 10-");
  let num = |s: &str| s.trim().parse::<usize>().ok().filter(|n| *n > 0);
  let (start, end) = match raw.split_once('-') {
    Some((a, b)) if b.trim().is_empty() => (num(a).ok_or_else(bad)?, None),
    Some((a, b)) => (num(a).ok_or_else(bad)?, Some(num(b).ok_or_else(bad)?)),
    None => {
      let n = num(raw).ok_or_else(bad)?;
      (n, Some(n))
    }
  };
  if end.is_some_and(|e| e < start) {
    return Err(bad());
  }
  Ok((start, end))
}

/// The files a read call's arguments name, ranges left out (compaction's file restore and the chunk index)
pub fn targets(args: &Value, cwd: &Path) -> Vec<PathBuf> {
  let first = args.get("path").or_else(|| args.get("file_path")).and_then(Value::as_str);
  first.map(str::to_owned).into_iter().chain(more_paths(args)).filter_map(|raw| split_range(&raw, cwd).ok().map(|(p, _)| p)).collect()
}

/// A file's window as numbered rows, before any byte budget; Err is what the model gets instead (an error, or the
/// note that the file is empty)
struct View {
  path: PathBuf,
  offset: usize,
  total: usize,
  rows: Vec<String>,
  /// A PDF's pages: rows and the counts are pages, not lines
  pdf: bool,
}

impl View {
  fn size(&self) -> usize {
    self.rows.iter().map(String::len).sum()
  }

  /// The header and the rows that fit in `max_bytes`, at least one
  fn render(&self, max_bytes: usize) -> String {
    let mut body = String::new();
    let mut last = self.offset - 1;
    for row in &self.rows {
      if body.len() + row.len() > max_bytes && last >= self.offset {
        break;
      }
      body.push_str(row);
      last += 1;
    }
    let (path, total, offset) = (self.path.display(), self.total, self.offset);
    let (kind, unit) = if self.pdf { ("PDF, ", "pages") } else { ("", "lines") };
    let header = match (offset, last == total) {
      (1, true) => format!("{path} ({kind}{total} {unit})\n"),
      (_, true) => format!("{path} ({kind}{unit} {offset}-{last} of {total})\n"),
      _ => format!("{path} ({kind}{unit} {offset}-{last} of {total}; continue with offset {})\n", last + 1),
    };
    header + &body
  }
}

/// The lines of a window, or what the call says in their place: an error, a note, or an image file's picture (boxed:
/// an Output is large next to a View). `left` is what the call may still send of encoded images
fn view(s: &Slice, images: bool, left: &mut usize) -> Result<View, Box<Output>> {
  let path = &s.path;
  let meta = match std::fs::metadata(path) {
    Ok(m) => m,
    Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(Box::new(Output::error(format!("File not found: {}", path.display())))),
    Err(e) => return Err(Box::new(Output::error(format!("Cannot read {}: {e}", path.display())))),
  };
  if meta.is_dir() {
    return Err(Box::new(Output::error(format!("{} is a directory; list it with the list tool instead", path.display()))));
  }
  let bytes = match std::fs::read(path) {
    Ok(b) => b,
    Err(e) => return Err(Box::new(Output::error(format!("Cannot read {}: {e}", path.display())))),
  };
  // Known by its first bytes, so a picture saved under another extension is still one
  if let Some(mime) = image_mime(&bytes) {
    return Err(Box::new(picture(path, &bytes, mime, images, left)));
  }
  // Before the binary test: a PDF need not hold a NUL byte early on
  if super::pdf::is_pdf(&bytes) {
    return pdf_view(path, &bytes, s);
  }
  if bytes.iter().take(8192).any(|b| *b == 0) {
    // A picture in a format the providers do not take: say how to get one they do
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    if ext.as_deref().is_some_and(|e| ["avif", "bmp", "heic", "heif", "ico", "tif", "tiff"].contains(&e)) {
      return Err(Box::new(Output::error(format!(
        "{} is an image in a format models do not take; convert it to PNG or JPEG with a command (sips, ImageMagick or ffmpeg) and read that",
        path.display()
      ))));
    }
    return Err(Box::new(Output::error(format!("{} is a binary file ({} bytes); it cannot be read as text", path.display(), bytes.len()))));
  }
  let text = String::from_utf8_lossy(&bytes);
  let total = text.lines().count();
  if total == 0 {
    return Err(Box::new(Output { model: format!("{} is empty.", path.display()), is_error: false, content: vec![], raw_output: None }));
  }
  if s.offset > total {
    return Err(Box::new(Output::error(format!("offset {} is past the end: {} has {total} lines", s.offset, path.display()))));
  }
  let rows = text
    .lines()
    .enumerate()
    .skip(s.offset - 1)
    .take(s.limit)
    .map(|(i, line)| {
      let line = if line.len() > MAX_LINE { format!("{}… [line cut at {MAX_LINE} bytes]", crate::budget::cut(line, MAX_LINE)) } else { line.to_owned() };
      format!("{:>6}\t{line}\n", i + 1)
    })
    .collect();
  Ok(View { path: path.clone(), offset: s.offset, total, rows, pdf: false })
}

/// A PDF's window of pages as a view; a window without any text says so, with how to look at the pages instead
fn pdf_view(path: &Path, bytes: &[u8], s: &Slice) -> Result<View, Box<Output>> {
  let pages = super::pdf::pages(bytes, s.offset, s.limit, MAX_BYTES).map_err(|e| Box::new(Output::error(format!("Cannot read {} as a PDF: {e}", path.display()))))?;
  if pages.total == 0 {
    return Err(Box::new(Output { model: format!("{} is a PDF without pages.", path.display()), is_error: false, content: vec![], raw_output: None }));
  }
  if pages.textless {
    let last = s.offset + pages.rows.len() - 1;
    let which = if s.offset == 1 && last == pages.total { "it has no text".to_owned() } else { format!("pages {}-{last} have no text", s.offset) };
    let model = format!("{} (PDF, {} pages): {which} (scanned, or fonts without an encoding). To look at it, {}.", path.display(), pages.total, super::pdf::RENDER_HINT);
    return Err(Box::new(Output { model, is_error: false, content: vec![], raw_output: None }));
  }
  Ok(View { path: path.to_owned(), offset: s.offset, total: pages.total, rows: pages.rows, pdf: true })
}

/// The image formats every provider takes, known by their signatures
fn image_mime(b: &[u8]) -> Option<&'static str> {
  if b.starts_with(b"\x89PNG\r\n\x1a\n") {
    Some("image/png")
  } else if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
    Some("image/jpeg")
  } else if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") {
    Some("image/gif")
  } else if b.len() >= 12 && b.starts_with(b"RIFF") && &b[8..12] == b"WEBP" {
    Some("image/webp")
  } else {
    None
  }
}

/// Width and height in pixels, from the file's header; None when the header is not where the format puts it
fn dimensions(b: &[u8], mime: &str) -> Option<(u32, u32)> {
  let be16 = |i: usize| Some(u32::from(u16::from_be_bytes(b.get(i..i + 2)?.try_into().ok()?)));
  let le16 = |i: usize| Some(u32::from(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?)));
  let le24 = |i: usize| b.get(i..i + 3).map(|s| u32::from(s[0]) | u32::from(s[1]) << 8 | u32::from(s[2]) << 16);
  match mime {
    "image/png" => Some((u32::from_be_bytes(b.get(16..20)?.try_into().ok()?), u32::from_be_bytes(b.get(20..24)?.try_into().ok()?))),
    "image/gif" => Some((le16(6)?, le16(8)?)),
    "image/webp" => match b.get(12..16)? {
      // Lossy: the frame header after the 3-byte tag and the start code
      b"VP8 " => Some((le16(26)? & 0x3FFF, le16(28)? & 0x3FFF)),
      // Lossless: 14 bits each of width - 1 and height - 1 after the signature byte
      b"VP8L" => {
        let v = u32::from_le_bytes(b.get(21..25)?.try_into().ok()?);
        Some(((v & 0x3FFF) + 1, ((v >> 14) & 0x3FFF) + 1))
      }
      // Extended: the canvas, 24 bits each of width - 1 and height - 1
      b"VP8X" => Some((le24(24)? + 1, le24(27)? + 1)),
      _ => None,
    },
    "image/jpeg" => {
      // Walk the segments to the frame header (SOFn), which holds the height, then the width
      let mut i = 2;
      loop {
        if *b.get(i)? != 0xFF {
          return None;
        }
        let mut j = i + 1;
        while *b.get(j)? == 0xFF {
          j += 1;
        }
        let marker = b[j];
        if (0xD0..=0xD9).contains(&marker) || marker == 0x01 {
          i = j + 1;
          continue;
        }
        // C4, C8 and CC share the range but are not frame headers
        if (0xC0..=0xCF).contains(&marker) && ![0xC4, 0xC8, 0xCC].contains(&marker) {
          return Some((be16(j + 6)?, be16(j + 4)?));
        }
        i = j + 1 + be16(j + 1)? as usize;
      }
    }
    _ => None,
  }
}

/// An image file as the call's result. It goes on the card when it is small enough to carry, and to the model as well
/// when the model takes images and the picture is within what the providers accept; the model's text says which,
/// and why not, so it never takes a missing picture for an empty one
fn picture(path: &Path, bytes: &[u8], mime: &str, images: bool, left: &mut usize) -> Output {
  let size = dimensions(bytes, mime);
  let pixels = size.map_or_else(String::new, |(w, h)| format!(", {w}x{h} pixels"));
  let about = format!("{}: {} image{pixels}, {} KB", path.display(), mime.trim_start_matches("image/").to_uppercase(), bytes.len().div_ceil(1024));
  let data = base64::engine::general_purpose::STANDARD.encode(bytes);
  let encoded = data.len();
  let fits = encoded <= super::mcp::MAX_IMAGE_BYTES;
  let smaller = "make a smaller copy with a command (sips, ImageMagick or ffmpeg) and read that";
  let withheld = if !images {
    Some("this model does not accept images".to_owned())
  } else if !fits {
    Some(format!("it is over {} MB encoded; {smaller}", super::mcp::MAX_IMAGE_BYTES >> 20))
  } else if size.is_some_and(|(w, h)| w.max(h) > MAX_IMAGE_SIDE) {
    Some(format!("it is over {MAX_IMAGE_SIDE} pixels on a side; {smaller}"))
  } else if encoded > *left {
    Some("this call already holds as many images as one request takes; read it in another call".to_owned())
  } else {
    None
  };
  let mut content = vec![];
  if fits {
    content.push(json!({ "type": "content", "content": { "type": "image", "data": data, "mimeType": mime } }));
  }
  let model = match withheld {
    None => {
      *left -= encoded;
      format!("{about}. The image is attached to this result.")
    }
    Some(why) => {
      // On the card only (`turn::images_of` leaves a marked image out of the model's result)
      if let Some(c) = content.last_mut() {
        c["_meta"] = json!({ "acpira/notForModel": true });
      }
      format!("{about}. It is not shown to you: {why}.")
    }
  };
  Output { model, is_error: false, content, raw_output: None }
}

/// The windows within one output budget, split evenly: a window under its share is read whole and leaves the rest to
/// the others. A file that cannot be read is reported in its place; only a single file that fails makes the call an
/// error. Image files come back as pictures (`picture`); `images` says whether the model takes them
pub fn run(files: &[Slice], images: bool, _ctx: &Ctx) -> Output {
  let mut left = MAX_CALL_IMAGE_BYTES;
  let mut views: Vec<Result<View, Box<Output>>> = files.iter().map(|s| view(s, images, &mut left)).collect();
  if views.len() == 1 {
    return match views.remove(0) {
      Ok(v) => Output { model: v.render(MAX_BYTES), is_error: false, content: vec![], raw_output: None },
      Err(out) => *out,
    };
  }
  let sizes: Vec<usize> = views.iter().map(|v| v.as_ref().map_or_else(|o| o.model.len(), View::size)).collect();
  let parts: Vec<String> =
    views.iter().zip(shares(&sizes, MAX_BYTES)).map(|(v, share)| v.as_ref().map_or_else(|o| o.model.clone(), |v| v.render(share))).collect();
  // The pictures among the files go on the card in order; text windows stay the model's alone, as before
  let content = views
    .iter()
    .filter_map(|v| v.as_ref().err())
    .flat_map(|o| o.content.iter().filter(|c| c.pointer("/content/type").and_then(Value::as_str) == Some("image")).cloned())
    .collect();
  Output { model: parts.join("\n\n"), is_error: false, content, raw_output: None }
}

/// `budget` split over requests of these sizes: from the smallest up, each gets what it asks while that is under an even
/// split of what is left, and the rest share the remainder
fn shares(sizes: &[usize], budget: usize) -> Vec<usize> {
  let mut order: Vec<usize> = (0..sizes.len()).collect();
  order.sort_by_key(|&i| sizes[i]);
  let mut out = vec![0; sizes.len()];
  let mut left = budget;
  for (k, &i) in order.iter().enumerate() {
    out[i] = sizes[i].min(left / (sizes.len() - k));
    left -= out[i];
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  fn ctx(dir: &Path) -> Ctx {
    Ctx { cwd: dir.to_owned(), outputs: dir.join("outputs"), call_id: "c".into(), progress: std::sync::Arc::new(|_| {}), jobs: Default::default(), search: Default::default(), mcp: Default::default(), history: Default::default() }
  }

  #[test]
  fn windows_and_totals() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("a.txt");
    std::fs::write(&f, (1..=10).map(|n| format!("l{n}\n")).collect::<String>()).unwrap();
    let read = |offset, limit| run(&[Slice { path: f.clone(), offset, limit }], false, &ctx(dir.path()));
    let all = read(1, 2000);
    assert!(all.model.contains("(10 lines)") && all.model.contains("    10\tl10"));
    let part = read(3, 2);
    assert!(part.model.contains("(lines 3-4 of 10; continue with offset 5)"), "{}", part.model);
    assert!(read(9, 5).model.contains("(lines 9-10 of 10)\n"), "a window that reaches the end offers no next one");
    assert!(read(11, 5).is_error);
    std::fs::write(&f, [0u8, 1, 2]).unwrap();
    assert!(read(1, 10).model.contains("binary"));
  }

  fn slices(args: Value, cwd: &Path) -> Vec<Slice> {
    let Ok(Action::Read { files, .. }) = prepare(&args, cwd, false) else { panic!("{args}") };
    files
  }

  #[test]
  fn several_files_in_one_call() {
    let dir = tempfile::tempdir().unwrap();
    let (a, b) = (dir.path().join("a.txt"), dir.path().join("b.txt"));
    std::fs::write(&a, "a1\na2\n").unwrap();
    std::fs::write(&b, "b1\n").unwrap();
    let files = slices(json!({ "path": "a.txt", "paths": ["a.txt", "b.txt", "gone.txt"] }), dir.path());
    let paths: Vec<&Path> = files.iter().map(|s| s.path.as_path()).collect();
    assert_eq!(paths, [a.as_path(), b.as_path(), dir.path().join("gone.txt").as_path()], "the first path is not read twice");
    let out = run(&files, false, &ctx(dir.path()));
    assert!(!out.is_error, "one missing file does not fail the others");
    assert!(out.model.contains("     2\ta2") && out.model.contains("     1\tb1") && out.model.contains("File not found"), "{}", out.model);
    let many: Vec<String> = (0..MAX_PATHS).map(|n| format!("f{n}")).collect();
    assert!(prepare(&json!({ "path": "a.txt", "paths": many }), dir.path(), false).is_err());
    // A lone string, and an array sent as a JSON string
    assert_eq!(slices(json!({ "path": "a.txt", "paths": "b.txt" }), dir.path()).len(), 2);
    assert_eq!(slices(json!({ "path": "a.txt", "paths": "[\"b.txt\"]" }), dir.path()).len(), 2);
  }

  #[test]
  fn paths_take_line_ranges_as_grep_prints_them() {
    let dir = tempfile::tempdir().unwrap();
    let f = dir.path().join("a.rs");
    std::fs::write(&f, (1..=300).map(|n| format!("line {n}\n")).collect::<String>()).unwrap();
    assert_eq!(slices(json!({ "path": "a.rs:7:" }), dir.path()), [Slice { path: f.clone(), offset: 7, limit: DEFAULT_LIMIT }]);
    let files = slices(json!({ "path": "a.rs:120-122", "paths": ["a.rs:200", "a.rs:120-122", "a.rs"] }), dir.path());
    assert_eq!(
      files,
      [Slice { path: f.clone(), offset: 120, limit: 3 }, Slice { path: f.clone(), offset: 200, limit: DEFAULT_LIMIT }],
      "a repeated window and a bare repeat of the first file are dropped"
    );
    let out = run(&files, false, &ctx(dir.path()));
    assert!(out.model.contains("(lines 120-122 of 300; continue with offset 123)\n   120\tline 120\n   121\tline 121\n   122\tline 122\n"), "{}", out.model);
    assert!(out.model.contains("(lines 200-300 of 300)\n   200\tline 200"), "{}", out.model);
    // The bare file in `path` with its ranges in `paths` reads only the ranges; another file named bare stays whole
    let files = slices(json!({ "path": "a.rs", "paths": ["a.rs:120-122", "b.rs"] }), dir.path());
    assert_eq!(files, [Slice { path: f.clone(), offset: 120, limit: 3 }, Slice { path: dir.path().join("b.rs"), offset: 1, limit: DEFAULT_LIMIT }]);
    assert_eq!(slices(json!({ "path": "a.rs", "offset": 1, "paths": ["a.rs:120-122"] }), dir.path()).len(), 2, "an explicit offset is a window");
    // offset / limit given with the call win over the range in the path
    assert_eq!(slices(json!({ "path": "a.rs:120-122", "offset": 5, "limit": 2 }), dir.path()), [Slice { path: f.clone(), offset: 5, limit: 2 }]);
    // A file really named that way is read as named; a backwards range is refused
    std::fs::write(dir.path().join("odd:7"), "x\n").unwrap();
    assert_eq!(slices(json!({ "path": "odd:7" }), dir.path())[0].path, dir.path().join("odd:7"));
    assert!(prepare(&json!({ "path": "a.rs", "paths": ["a.rs:20-10"] }), dir.path(), false).unwrap_err().contains("paths[0]: a.rs:20-10"));
    assert_eq!(targets(&json!({ "path": "a.rs:1-2", "paths": ["b.rs:9", "c.rs"] }), dir.path()), [f, dir.path().join("b.rs"), dir.path().join("c.rs")]);
  }

  #[test]
  fn the_budget_is_split_evenly_between_the_files() {
    let dir = tempfile::tempdir().unwrap();
    let (big, other, small) = (dir.path().join("big.txt"), dir.path().join("other.txt"), dir.path().join("small.txt"));
    // Each about 60 KB, more than the whole budget
    std::fs::write(&big, format!("{}\n", "x".repeat(1999)).repeat(30)).unwrap();
    std::fs::write(&other, format!("{}\n", "y".repeat(1999)).repeat(30)).unwrap();
    std::fs::write(&small, "tiny\n").unwrap();
    let files = slices(json!({ "path": "big.txt", "paths": ["other.txt", "small.txt"] }), dir.path());
    let out = run(&files, false, &ctx(dir.path()));
    assert!(out.model.contains("     1\ttiny"), "the small file is read whole");
    let rows = |c: &str| out.model.lines().filter(|l| l.contains(&format!("\t{c}{c}{c}"))).count();
    assert_eq!((rows("x"), rows("y")), (12, 12), "the two large files get the same share");
    assert!(out.model.len() < MAX_BYTES + 1024, "{}", out.model.len());
    assert_eq!(shares(&[100, 100_000, 100_000], 51_200), [100, 25_550, 25_550]);
    assert_eq!(shares(&[10, 20], 100), [10, 20]);
  }

  #[test]
  fn a_pdf_is_read_as_its_pages() {
    let dir = tempfile::tempdir().unwrap();
    let doc = dir.path().join("spec.pdf");
    std::fs::write(&doc, super::super::pdf::tests::pdf(&[Some("Intro page"), Some("Second page"), Some("Third page")])).unwrap();
    let out = run(&slices(json!({ "path": "spec.pdf" }), dir.path()), true, &ctx(dir.path()));
    assert!(!out.is_error && out.model.contains("spec.pdf (PDF, 3 pages)\n--- page 1 ---\nIntro page"), "{}", out.model);
    assert!(out.content.is_empty(), "text goes to the model only, as for any read");
    // A range in the path, offset / limit and `pages` all count pages, and a window short of the end says where to go on
    let out = run(&slices(json!({ "path": "spec.pdf:2-2" }), dir.path()), true, &ctx(dir.path()));
    assert!(out.model.contains("(PDF, pages 2-2 of 3; continue with offset 3)") && out.model.contains("Second page") && !out.model.contains("Third"), "{}", out.model);
    assert_eq!(slices(json!({ "path": "spec.pdf", "pages": "2-3" }), dir.path()), [Slice { path: doc.clone(), offset: 2, limit: 2 }]);
    assert_eq!(slices(json!({ "path": "spec.pdf", "pages": "3" }), dir.path()), [Slice { path: doc.clone(), offset: 3, limit: 1 }]);
    assert_eq!(slices(json!({ "path": "spec.pdf", "pages": "2-" }), dir.path()), [Slice { path: doc.clone(), offset: 2, limit: DEFAULT_LIMIT }]);
    assert!(prepare(&json!({ "path": "spec.pdf", "pages": "5-2" }), dir.path(), true).is_err());
    assert!(prepare(&json!({ "path": "spec.pdf", "pages": "first" }), dir.path(), true).unwrap_err().contains("not a page range"));
    let out = run(&slices(json!({ "path": "spec.pdf", "pages": "9" }), dir.path()), true, &ctx(dir.path()));
    assert!(out.is_error && out.model.contains("past the end"), "{}", out.model);

    // A scan has no text: the model hears how to look at its pages instead
    let scan = dir.path().join("scan.pdf");
    std::fs::write(&scan, super::super::pdf::tests::pdf(&[None, None])).unwrap();
    let out = run(&slices(json!({ "path": "scan.pdf" }), dir.path()), true, &ctx(dir.path()));
    assert!(!out.is_error && out.model.contains("(PDF, 2 pages): it has no text") && out.model.contains("pdftoppm"), "{}", out.model);
    std::fs::write(&scan, b"%PDF-1.7 broken").unwrap();
    assert!(run(&slices(json!({ "path": "scan.pdf" }), dir.path()), true, &ctx(dir.path())).model.contains("Cannot read"));
  }

  /// A PNG header of this size (the IHDR chunk is all `dimensions` reads), padded so it holds NUL bytes like a real one
  fn png(w: u32, h: u32) -> Vec<u8> {
    let mut b = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    b.extend(w.to_be_bytes());
    b.extend(h.to_be_bytes());
    b.extend([8, 6, 0, 0, 0]);
    b
  }

  #[test]
  fn image_headers_give_their_size() {
    assert_eq!(image_mime(&png(640, 480)), Some("image/png"));
    assert_eq!(dimensions(&png(640, 480), "image/png"), Some((640, 480)));
    let mut gif = b"GIF89a".to_vec();
    gif.extend([0x40, 0x01, 0xC8, 0x00]);
    assert_eq!((image_mime(&gif), dimensions(&gif, "image/gif")), (Some("image/gif"), Some((320, 200))));
    // An APP0 segment before the baseline frame header, which holds the height before the width
    let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
    jpeg.extend([0u8; 14]);
    jpeg.extend([0xFF, 0xC0, 0x00, 0x11, 0x08, 0x01, 0xE0, 0x02, 0x80, 0x03]);
    assert_eq!((image_mime(&jpeg), dimensions(&jpeg, "image/jpeg")), (Some("image/jpeg"), Some((640, 480))));
    let webp = |chunk: &[u8], body: &[u8]| [b"RIFF\0\0\0\0WEBP".as_slice(), chunk, b"\0\0\0\0", body].concat();
    let extended = webp(b"VP8X", &[0, 0, 0, 0, 0x7F, 0x07, 0x00, 0x37, 0x04, 0x00]);
    assert_eq!((image_mime(&extended), dimensions(&extended, "image/webp")), (Some("image/webp"), Some((1920, 1080))));
    let bits = (99u32 | (49 << 14)).to_le_bytes();
    let lossless = webp(b"VP8L", &[[0x2F].as_slice(), &bits].concat());
    assert_eq!(dimensions(&lossless, "image/webp"), Some((100, 50)));
    let lossy = webp(b"VP8 ", &[0, 0, 0, 0x9D, 0x01, 0x2A, 0x20, 0x03, 0x58, 0x02]);
    assert_eq!(dimensions(&lossy, "image/webp"), Some((800, 600)));
    // A cut header names no size; text and other binaries are no images
    assert_eq!(dimensions(&png(640, 480)[..18], "image/png"), None);
    assert_eq!(dimensions(&[0xFF, 0xD8, 0xFF, 0xE0], "image/jpeg"), None);
    assert_eq!(image_mime(b"plain text"), None);
    assert_eq!(image_mime(b"RIFF\0\0\0\0WAVE"), None);
  }

  #[test]
  fn an_image_file_is_read_as_its_picture() {
    let dir = tempfile::tempdir().unwrap();
    let shot = dir.path().join("frame.jpg");
    // The signature decides, not the extension
    std::fs::write(&shot, png(640, 480)).unwrap();
    let file = |p: &Path| vec![Slice { path: p.to_owned(), offset: 1, limit: DEFAULT_LIMIT }];
    let image_of = |out: &Output| out.content.iter().find(|c| c.pointer("/content/type").and_then(Value::as_str) == Some("image")).cloned();

    let seen = run(&file(&shot), true, &ctx(dir.path()));
    assert!(!seen.is_error && seen.model.contains("PNG image, 640x480 pixels") && seen.model.contains("attached"), "{}", seen.model);
    let block = image_of(&seen).expect("the picture is on the card");
    assert_eq!(block["content"]["mimeType"], "image/png");
    assert_eq!(super::super::mcp::model_images(&seen.content).len(), 1, "and goes to the model");

    let text_only = run(&file(&shot), false, &ctx(dir.path()));
    assert!(text_only.model.contains("does not accept images"), "{}", text_only.model);
    assert!(image_of(&text_only).is_some(), "the user still sees it on the card");
    assert!(super::super::mcp::model_images(&text_only.content).is_empty());

    let huge = dir.path().join("huge.png");
    std::fs::write(&huge, png(9000, 200)).unwrap();
    let out = run(&file(&huge), true, &ctx(dir.path()));
    assert!(out.model.contains("over 8000 pixels") && super::super::mcp::model_images(&out.content).is_empty(), "{}", out.model);

    // Among text files, each picture goes on the card in order and its note takes its place in the text
    let notes = dir.path().join("notes.txt");
    std::fs::write(&notes, "frame notes\n").unwrap();
    let files = slices(json!({ "path": "notes.txt", "paths": ["frame.jpg", "huge.png"] }), dir.path());
    let out = run(&files, true, &ctx(dir.path()));
    assert!(out.model.contains("     1\tframe notes") && out.model.contains("640x480") && out.model.contains("9000x200"), "{}", out.model);
    assert_eq!((out.content.len(), super::super::mcp::model_images(&out.content).len()), (2, 1));

    // A picture the providers do not take says how to get one they do
    let tiff = dir.path().join("scan.tiff");
    std::fs::write(&tiff, b"II*\0\x08\0\0\0").unwrap();
    let out = run(&file(&tiff), true, &ctx(dir.path()));
    assert!(out.is_error && out.model.contains("convert it to PNG or JPEG"), "{}", out.model);
  }
}
