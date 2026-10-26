//! A PDF's text for `read`, page by page (2026-10-24). pdf-extract reads each page's text through its fonts' ToUnicode
//! maps, which CJK documents need (lopdf's own extraction turned a Chinese page into mojibake). Only the pages a call
//! asks for are parsed, and only until the call's budget is full. A page without text (a scan) or whose fonts carry no
//! usable encoding is named as such, so the model turns to rendering the pages rather than trusting an empty read

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use pdf_extract::{Document, PlainTextOutput, output_doc_page};
use serde::{Deserialize, Serialize};

/// What the format puts at the start of the file (a few bytes of junk before it are allowed, as readers do)
pub fn is_pdf(bytes: &[u8]) -> bool {
  bytes[..bytes.len().min(1024)].windows(5).any(|w| w == b"%PDF-")
}

/// The pages of one window
#[derive(Debug, Serialize, Deserialize)]
pub struct Pages {
  /// Pages in the document
  pub total: usize,
  /// One entry per page read, from the window's first: a marker line, then the page's text
  pub rows: Vec<String>,
  /// No page read had usable text
  pub textless: bool,
}

/// Where to look when a PDF has no text to give
pub const RENDER_HINT: &str = "render its pages to PNG with a command (pdftoppm -png -r 110, mutool draw, magick, or on macOS sips -s format png for \
                               the first page) and read the images";

/// What a parser crash reads as
const FAILED: &str = "the PDF parser failed on this file";

/// The subcommand `run_child` answers (`acpira pdf-pages`, also the Windows desktop exe's)
pub const CHILD_COMMAND: &str = "pdf-pages";

/// How long a child may parse before it is ended (a large PDF takes seconds, a pathological one forever)
const CHILD_TIMEOUT: Duration = Duration::from_secs(120);

/// Pages `first` (1-based) on, at most `limit` of them, until their text passes `max_bytes`. pdf-extract unwraps on some
/// malformed files, and a panic is a file that cannot be read, not a dead agent: where panics unwind (tests, development
/// builds) it is caught here; shipped builds abort on a panic, so they parse in a child process of this binary, whose
/// crash ends only the child
pub fn pages(bytes: &[u8], first: usize, limit: usize, max_bytes: usize) -> Result<Pages, String> {
  if cfg!(panic = "unwind") {
    return std::panic::catch_unwind(|| read_pages(bytes, first, limit, max_bytes)).unwrap_or_else(|_| Err(FAILED.into()));
  }
  let exe = std::env::current_exe().map_err(|e| format!("the PDF parser cannot start: {e}"))?;
  in_child(&exe, bytes, first, limit, max_bytes)
}

/// `pages` in a child: `<exe> pdf-pages FIRST LIMIT MAX_BYTES` with the PDF on stdin, the answer as JSON on stdout. A
/// child that crashes, prints nothing usable or outlives `CHILD_TIMEOUT` reads as a file the parser failed on
pub fn in_child(exe: &Path, bytes: &[u8], first: usize, limit: usize, max_bytes: usize) -> Result<Pages, String> {
  let mut cmd = Command::new(exe);
  cmd.args([CHILD_COMMAND, &first.to_string(), &limit.to_string(), &max_bytes.to_string()]);
  cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
  #[cfg(windows)]
  {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
  }
  let mut child = cmd.spawn().map_err(|e| format!("the PDF parser cannot start: {e}"))?;
  // Both pipes on threads of their own, so the wait below can give up on a child that stops reading or never answers
  if let Some(mut stdin) = child.stdin.take() {
    let input = bytes.to_vec();
    std::thread::spawn(move || {
      let _ = stdin.write_all(&input);
    });
  }
  let reader = child.stdout.take().map(|mut out| {
    std::thread::spawn(move || {
      let mut answer = vec![];
      let _ = out.read_to_end(&mut answer);
      answer
    })
  });
  let deadline = Instant::now() + CHILD_TIMEOUT;
  let status = loop {
    match child.try_wait() {
      Ok(Some(status)) => break Some(status),
      Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
      _ => {
        let _ = child.kill();
        let _ = child.wait();
        break None;
      }
    }
  };
  let answer = reader.and_then(|r| r.join().ok()).unwrap_or_default();
  match status {
    None => Err("the PDF parser took too long on this file".into()),
    Some(s) if s.success() => serde_json::from_slice::<Result<Pages, String>>(&answer).unwrap_or_else(|_| Err(FAILED.into())),
    Some(_) => Err(FAILED.into()),
  }
}

/// The child side of `in_child`: `pdf-pages FIRST LIMIT MAX_BYTES`, the PDF on stdin; the exit code
pub fn run_child(args: &[String]) -> i32 {
  let num = |i: usize| args.get(i).and_then(|a| a.parse::<usize>().ok());
  let (Some(first), Some(limit), Some(max_bytes)) = (num(0).filter(|f| *f >= 1), num(1), num(2)) else {
    eprintln!("usage: acpira {CHILD_COMMAND} FIRST LIMIT MAX_BYTES < file.pdf");
    return 2;
  };
  let mut bytes = vec![];
  if std::io::stdin().read_to_end(&mut bytes).is_err() {
    return 1;
  }
  let answer = read_pages(&bytes, first, limit, max_bytes);
  match serde_json::to_vec(&answer) {
    Ok(json) if std::io::stdout().write_all(&json).is_ok() => 0,
    _ => 1,
  }
}

fn read_pages(bytes: &[u8], first: usize, limit: usize, max_bytes: usize) -> Result<Pages, String> {
  let mut doc = Document::load_mem(bytes).map_err(|e| e.to_string())?;
  // Most "encrypted" PDFs only restrict printing or copying and open with the empty password
  if doc.is_encrypted() && doc.decrypt("").is_err() {
    return Err("it is encrypted with a password".into());
  }
  let numbers: Vec<u32> = doc.get_pages().keys().copied().collect();
  let total = numbers.len();
  if first > total.max(1) {
    return Err(format!("offset {first} is past the end: the PDF has {total} pages"));
  }
  let (mut rows, mut size, mut textless) = (vec![], 0usize, true);
  for (i, n) in numbers.iter().enumerate().skip(first - 1).take(limit) {
    let mut raw = String::new();
    let extracted = output_doc_page(&doc, &mut PlainTextOutput::new(&mut raw), *n);
    let body = match (extracted, clean(&raw)) {
      (Err(e), _) => format!("(the text of this page could not be read: {e})"),
      (Ok(()), Some(text)) => {
        textless = false;
        text
      }
      (Ok(()), None) if raw.trim().is_empty() => "(no text on this page)".to_owned(),
      (Ok(()), None) => "(this page's text could not be decoded: its fonts carry no usable encoding)".to_owned(),
    };
    let row = format!("--- page {} ---\n{}\n", i + 1, crate::budget::cut(&body, max_bytes));
    size += row.len();
    rows.push(row);
    if size > max_bytes {
      break;
    }
  }
  Ok(Pages { total, rows, textless })
}

/// A page's text with layout noise removed, or None when there is none or it is mostly undecodable (control
/// characters, U+FFFD or private-use code points, what a font without an encoding comes out as)
fn clean(raw: &str) -> Option<String> {
  let (mut seen, mut bad) = (0usize, 0usize);
  let mapped: String = raw
    .chars()
    .map(|c| {
      let control = c.is_control() && !matches!(c, '\n' | '\t' | '\r');
      if control || !c.is_whitespace() {
        seen += 1;
      }
      if control || c == '\u{FFFD}' || ('\u{E000}'..='\u{F8FF}').contains(&c) {
        bad += 1;
      }
      if control { ' ' } else { c }
    })
    .collect();
  if seen == 0 || bad * 10 >= seen * 3 {
    return None;
  }
  // Trailing spaces off every line, runs of spaces inside one (the gaps pdf-extract fills between text runs) and runs
  // of blank lines down to one; a line's indent stays
  let mut out = String::new();
  let mut blank = 0;
  for line in mapped.lines().map(str::trim_end) {
    if line.trim().is_empty() {
      blank += 1;
      continue;
    }
    if blank > 0 && !out.is_empty() {
      out.push('\n');
    }
    blank = 0;
    let body = line.trim_start();
    out.push_str(line.strip_suffix(body).unwrap_or_default());
    let mut space = false;
    for c in body.chars() {
      if c == ' ' {
        if !space {
          out.push(' ');
        }
        space = true;
      } else {
        out.push(c);
        space = false;
      }
    }
    out.push('\n');
  }
  Some(out.trim_end().to_owned())
}

#[cfg(test)]
pub(crate) mod tests {
  use super::*;
  use pdf_extract::{Dictionary, Object, Stream};

  /// A PDF with one page per entry, each showing its text in Helvetica (None: a page with nothing on it)
  pub(crate) fn pdf(pages: &[Option<&str>]) -> Vec<u8> {
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let mut font = Dictionary::new();
    font.set("Type", Object::Name(b"Font".to_vec()));
    font.set("Subtype", Object::Name(b"Type1".to_vec()));
    font.set("BaseFont", Object::Name(b"Helvetica".to_vec()));
    font.set("Encoding", Object::Name(b"WinAnsiEncoding".to_vec()));
    let font_id = doc.add_object(font);
    let mut fonts = Dictionary::new();
    fonts.set("F1", Object::Reference(font_id));
    let mut resources = Dictionary::new();
    resources.set("Font", Object::Dictionary(fonts));
    let resources_id = doc.add_object(resources);
    let mut kids = vec![];
    for text in pages {
      let ops = text.map_or_else(String::new, |t| format!("BT /F1 12 Tf 72 720 Td ({t}) Tj ET"));
      let content_id = doc.add_object(Stream::new(Dictionary::new(), ops.into_bytes()));
      let mut page = Dictionary::new();
      page.set("Type", Object::Name(b"Page".to_vec()));
      page.set("Parent", Object::Reference(pages_id));
      page.set("Contents", Object::Reference(content_id));
      page.set("Resources", Object::Reference(resources_id));
      page.set("MediaBox", Object::Array(vec![0.into(), 0.into(), 612.into(), 792.into()]));
      kids.push(Object::Reference(doc.add_object(page)));
    }
    let mut tree = Dictionary::new();
    tree.set("Type", Object::Name(b"Pages".to_vec()));
    tree.set("Count", Object::Integer(kids.len() as i64));
    tree.set("Kids", Object::Array(kids));
    doc.objects.insert(pages_id, Object::Dictionary(tree));
    let mut catalog = Dictionary::new();
    catalog.set("Type", Object::Name(b"Catalog".to_vec()));
    catalog.set("Pages", Object::Reference(pages_id));
    let catalog_id = doc.add_object(catalog);
    doc.trailer.set("Root", Object::Reference(catalog_id));
    let mut out = vec![];
    doc.save_to(&mut out).unwrap();
    out
  }

  #[test]
  fn pages_come_with_markers_and_stop_at_the_budget() {
    let bytes = pdf(&[Some("Hello first page"), None, Some("Third page text")]);
    assert!(is_pdf(&bytes) && !is_pdf(b"plain text"));
    let all = pages(&bytes, 1, 100, 50_000).unwrap();
    assert_eq!(all.total, 3);
    assert!(!all.textless);
    assert!(all.rows[0].starts_with("--- page 1 ---\n") && all.rows[0].contains("Hello first page"), "{:?}", all.rows);
    assert_eq!(all.rows[1], "--- page 2 ---\n(no text on this page)\n");
    let window = pages(&bytes, 3, 1, 50_000).unwrap();
    assert_eq!(window.rows.len(), 1);
    assert!(window.rows[0].starts_with("--- page 3 ---") && window.rows[0].contains("Third page text"));
    // Parsing stops once the budget is passed
    assert_eq!(pages(&bytes, 1, 100, 10).unwrap().rows.len(), 1);
    assert!(pages(&bytes, 4, 1, 50_000).unwrap_err().contains("past the end"));
    assert!(pages(&pdf(&[None, None]), 1, 10, 50_000).unwrap().textless);
    assert!(pages(b"%PDF-1.5 not really", 1, 10, 50_000).is_err());
  }

  #[test]
  fn undecodable_text_is_no_text() {
    assert_eq!(clean("  a \n\n\n\nb  \n").as_deref(), Some("  a\n\nb"));
    assert_eq!(clean("  聚焦   windsuf（等同于   Windsurf）").as_deref(), Some("  聚焦 windsuf（等同于 Windsurf）"));
    // A word separator some fonts map to U+0001 is a space, not noise, when it is rare
    assert_eq!(clean("死亡\u{1}本文我们来聊聊有关开拓者中的死亡设计").as_deref(), Some("死亡 本文我们来聊聊有关开拓者中的死亡设计"));
    assert_eq!(clean("\u{1}\u{2}\u{3}\u{4}\u{2}\u{5}ab"), None);
    assert_eq!(clean("\u{E001}\u{E002}x"), None);
    assert_eq!(clean(" \n "), None);
  }
}
