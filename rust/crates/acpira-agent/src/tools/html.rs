//! HTML to markdown (or plain text) for `webfetch`. A tolerant single-pass tokenizer, not a full HTML parser: it keeps
//! what a model reads a page for (headings, paragraphs, lists, links, code, tables as `|` rows), drops scripts, styles
//! and page chrome (nav, footer, aside, forms), and prefers the page's `<main>` / `<article>` when that holds most of the
//! text. The `<title>` and the description meta tags are kept apart, since a script-rendered page (X, most SPAs) often
//! carries its only server-side text there

use super::web;

/// What a page gives the model
#[derive(Debug, Default, PartialEq)]
pub struct Page {
  pub title: Option<String>,
  /// `description`, `og:description` or `twitter:description`, the first one present
  pub description: Option<String>,
  pub body: String,
}

/// Elements whose whole content is dropped
const SKIP: &[&str] = &[
  "script", "style", "noscript", "template", "svg", "math", "iframe", "object", "embed", "canvas", "nav", "footer", "aside", "form",
  "button", "select", "dialog", "head",
];
/// Elements that hold raw text up to their end tag (no markup inside)
const RAW: &[&str] = &["script", "style", "textarea", "title", "xmp"];
const VOID: &[&str] = &["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr"];
/// A paragraph break before and after
const BLOCK2: &[&str] =
  &["p", "pre", "blockquote", "ul", "ol", "table", "figure", "section", "article", "main", "header", "details", "dl", "address", "center"];
/// A line break before and after
const BLOCK1: &[&str] = &["div", "li", "tr", "dt", "dd", "summary", "figcaption", "caption"];

/// Convert a page; `base` is the URL it came from (relative links are made absolute against it, or its `<base href>`)
pub fn convert(html: &str, base: &str, plain: bool) -> Page {
  let mut r = Renderer { plain, base: base.to_owned(), ..Default::default() };
  let mut i = 0;
  let bytes = html.as_bytes();
  while i < bytes.len() {
    if bytes[i] != b'<' {
      let end = html.get(i..).and_then(|t| t.find('<')).map_or(html.len(), |n| i + n);
      r.text(&decode_entities(html.get(i..end).unwrap_or_default()));
      i = end;
      continue;
    }
    let Some(rest) = html.get(i..) else { break };
    if rest.starts_with("<!--") {
      i = rest.find("-->").map_or(html.len(), |n| i + n + 3);
    } else if rest.starts_with("<!") || rest.starts_with("<?") {
      i = rest.find('>').map_or(html.len(), |n| i + n + 1);
    } else if let Some(tag) = parse_tag(rest) {
      i += tag.len;
      if tag.end {
        r.close(&tag.name);
      } else if RAW.contains(&tag.name.as_str()) && !tag.self_closing {
        // Raw text runs to the matching end tag, whatever it contains
        let close = format!("</{}", tag.name);
        let end = find_ci(html.get(i..).unwrap_or_default(), &close).map_or(html.len(), |n| i + n);
        r.raw(&tag.name, html.get(i..end).unwrap_or_default());
        i = html.get(end..).and_then(|t| t.find('>')).map_or(html.len(), |n| end + n + 1);
      } else {
        r.open(&tag);
        if tag.self_closing || VOID.contains(&tag.name.as_str()) {
          r.close(&tag.name);
        }
      }
    } else {
      // A `<` that starts no tag (`a < b`) is text
      r.text("<");
      i += 1;
    }
  }
  r.finish()
}

#[derive(Debug)]
struct Tag {
  name: String,
  attrs: Vec<(String, String)>,
  end: bool,
  self_closing: bool,
  /// Bytes the tag takes in the source, `<` to `>`
  len: usize,
}

impl Tag {
  fn attr(&self, name: &str) -> Option<&str> {
    self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
  }
}

/// A tag at the start of `s` (which starts with `<`); None when it is not one
fn parse_tag(s: &str) -> Option<Tag> {
  let b = s.as_bytes();
  let mut i = 1;
  let end = b.get(1) == Some(&b'/');
  if end {
    i += 1;
  }
  let start = i;
  while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'-' || b[i] == b':') {
    i += 1;
  }
  if i == start || !b[start].is_ascii_alphabetic() {
    return None;
  }
  let name = s.get(start..i)?.to_ascii_lowercase();
  let mut attrs = vec![];
  let mut self_closing = false;
  loop {
    while i < b.len() && b[i].is_ascii_whitespace() {
      i += 1;
    }
    match b.get(i) {
      None => return None,
      Some(b'>') => return Some(Tag { name, attrs, end, self_closing, len: i + 1 }),
      Some(b'/') => {
        self_closing = true;
        i += 1;
        continue;
      }
      _ => {}
    }
    let k0 = i;
    while i < b.len() && !b[i].is_ascii_whitespace() && !matches!(b[i], b'=' | b'>' | b'/') {
      i += 1;
    }
    let key = s.get(k0..i)?.to_ascii_lowercase();
    while i < b.len() && b[i].is_ascii_whitespace() {
      i += 1;
    }
    let mut value = String::new();
    if b.get(i) == Some(&b'=') {
      i += 1;
      while i < b.len() && b[i].is_ascii_whitespace() {
        i += 1;
      }
      match b.get(i) {
        Some(&q @ (b'"' | b'\'')) => {
          let close = s.get(i + 1..)?.find(q as char)? + i + 1;
          value = decode_entities(s.get(i + 1..close)?);
          i = close + 1;
        }
        _ => {
          let v0 = i;
          while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'>' {
            i += 1;
          }
          value = decode_entities(s.get(v0..i)?);
        }
      }
    }
    if key.is_empty() {
      // Stray character in the tag (`<a "x">`): step over it
      i += 1;
    } else {
      self_closing = false;
      attrs.push((key, value));
    }
  }
}

/// Byte offset of `needle` in `hay`, ASCII case-insensitive
fn find_ci(hay: &str, needle: &str) -> Option<usize> {
  let (h, n) = (hay.as_bytes(), needle.as_bytes());
  (0..h.len().saturating_sub(n.len() - 1)).find(|&i| h[i..i + n.len()].eq_ignore_ascii_case(n))
}

#[derive(Default)]
struct Renderer {
  plain: bool,
  base: String,
  out: String,
  title: Option<String>,
  description: Option<String>,
  /// Name and nesting count of the element being skipped
  skip: Option<(String, usize)>,
  pre: usize,
  quote: usize,
  /// Open lists: ordered, and the next number
  lists: Vec<(bool, usize)>,
  /// Open links: target and where their text starts in `out`
  links: Vec<(Option<String>, usize)>,
  /// Open emphasis: marker and where it was written
  marks: Vec<(&'static str, usize)>,
  /// Nesting of `<main>` / `<article>`, and the output spans they produced
  main: (usize, Vec<(usize, usize)>, usize),
  article: (usize, Vec<(usize, usize)>, usize),
  /// The output is at the start of a line (the quote prefix is due before more text)
  line_start: bool,
}

impl Renderer {
  fn text(&mut self, s: &str) {
    if self.skip.is_some() || s.is_empty() {
      return;
    }
    if self.pre > 0 {
      self.out.push_str(s);
      self.line_start = s.ends_with('\n');
      return;
    }
    for (n, word) in s.split_ascii_whitespace().enumerate() {
      let spaced = n > 0 || s.starts_with(|c: char| c.is_ascii_whitespace());
      if spaced && !self.line_start && !self.out.ends_with([' ', '\n']) {
        self.out.push(' ');
      }
      self.prefix();
      self.out.push_str(word);
    }
    if s.ends_with(|c: char| c.is_ascii_whitespace()) && !self.line_start && !self.out.ends_with([' ', '\n']) && !self.out.is_empty() {
      self.out.push(' ');
    }
  }

  /// The blockquote prefix at the start of a line
  fn prefix(&mut self) {
    if self.line_start {
      self.line_start = false;
      if self.quote > 0 && !self.plain {
        self.out.push_str(&"> ".repeat(self.quote));
      }
    }
  }

  fn write(&mut self, s: &str) {
    self.prefix();
    self.out.push_str(s);
  }

  /// End the current line and leave `n` line breaks (1 a new line, 2 a blank line) behind it
  fn block(&mut self, n: usize) {
    while self.out.ends_with(' ') {
      self.out.pop();
    }
    if self.out.is_empty() {
      return;
    }
    let have = self.out.len() - self.out.trim_end_matches('\n').len();
    for _ in have..n {
      self.out.push('\n');
    }
    self.line_start = true;
  }

  fn raw(&mut self, name: &str, text: &str) {
    match name {
      "title" if self.title.is_none() => self.title = Some(collapse(&decode_entities(text))).filter(|t| !t.is_empty()),
      "textarea" | "xmp" if self.skip.is_none() => {
        self.block(2);
        self.text(&decode_entities(text));
        self.block(2);
      }
      _ => {}
    }
  }

  fn open(&mut self, tag: &Tag) {
    let name = tag.name.as_str();
    // Metadata is read even from the skipped <head>
    match name {
      "meta" => {
        let key = tag.attr("name").or_else(|| tag.attr("property")).unwrap_or("").to_ascii_lowercase();
        if matches!(key.as_str(), "description" | "og:description" | "twitter:description") && self.description.is_none() {
          self.description = tag.attr("content").map(collapse).filter(|d| !d.is_empty());
        }
        if key == "og:title" && self.title.is_none() {
          self.title = tag.attr("content").map(collapse).filter(|t| !t.is_empty());
        }
        return;
      }
      "base" => {
        if let Some(href) = tag.attr("href") {
          self.base = web::join(&self.base, href);
        }
        return;
      }
      _ => {}
    }
    if let Some((skipping, depth)) = &mut self.skip {
      if skipping == name {
        *depth += 1;
      }
      return;
    }
    // `hidden` / `aria-hidden` content is not on the page
    if SKIP.contains(&name) || tag.attr("hidden").is_some() || tag.attr("aria-hidden") == Some("true") {
      if !VOID.contains(&name) {
        self.skip = Some((name.to_owned(), 1));
      }
      return;
    }
    match name {
      "main" => {
        self.main.0 += 1;
        if self.main.0 == 1 {
          self.main.2 = self.out.len();
        }
      }
      "article" => {
        self.article.0 += 1;
        if self.article.0 == 1 {
          self.article.2 = self.out.len();
        }
      }
      _ => {}
    }
    // A list inside a list item continues the item: one line break, not a paragraph
    let nested = matches!(name, "ul" | "ol") && !self.lists.is_empty();
    if BLOCK2.contains(&name) && !nested {
      self.block(2);
    } else if BLOCK1.contains(&name) || nested {
      self.block(1);
    }
    match name {
      "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
        self.block(2);
        if !self.plain {
          let level = (name.as_bytes()[1] - b'0') as usize;
          self.write(&format!("{} ", "#".repeat(level)));
        }
      }
      "br" => {
        if self.pre > 0 {
          self.out.push('\n');
        } else {
          self.block(1);
        }
      }
      "hr" => {
        self.block(2);
        if !self.plain {
          self.write("---");
        }
        self.block(2);
      }
      "pre" => {
        if self.pre == 0 && !self.plain {
          self.write("```\n");
        }
        self.pre += 1;
      }
      "blockquote" => self.quote += 1,
      "ul" | "ol" => {
        let start = tag.attr("start").and_then(|s| s.trim().parse().ok()).unwrap_or(1);
        self.lists.push((name == "ol", start));
      }
      "li" => {
        let depth = self.lists.len().max(1);
        let marker = match self.lists.last_mut() {
          Some((true, n)) => {
            *n += 1;
            format!("{}. ", *n - 1)
          }
          _ => "- ".to_owned(),
        };
        self.write(&format!("{}{marker}", "  ".repeat(depth - 1)));
      }
      "td" | "th" => {
        if self.out.ends_with('\n') || self.out.is_empty() {
          self.write("| ");
        }
      }
      "a" => {
        let href = tag.attr("href").map(str::trim).filter(|h| !h.is_empty() && !h.starts_with('#') && !h.to_ascii_lowercase().starts_with("javascript:"));
        let target = href.map(|h| web::join(&self.base, h));
        self.links.push((target, self.out.len()));
      }
      "img" => {
        let alt = tag.attr("alt").map(collapse).unwrap_or_default();
        let src = tag.attr("src").filter(|s| !s.starts_with("data:"));
        if !alt.is_empty() {
          match src {
            Some(src) if !self.plain => {
              let src = web::join(&self.base, src);
              self.write(&format!("![{alt}]({src})"));
            }
            _ => self.text(&alt),
          }
        }
      }
      "strong" | "b" => self.mark("**"),
      "em" | "i" => self.mark("*"),
      "code" | "kbd" | "samp" if self.pre == 0 => self.mark("`"),
      _ => {}
    }
  }

  fn mark(&mut self, m: &'static str) {
    if self.plain {
      return;
    }
    if !self.line_start && !self.out.is_empty() && !self.out.ends_with([' ', '\n', '(', '[']) {
      // Keep a word boundary in front so the marker reads as one
      self.out.push(' ');
    }
    self.write(m);
    self.marks.push((m, self.out.len()));
  }

  fn close(&mut self, name: &str) {
    let nested = matches!(name, "ul" | "ol") && self.lists.len() > 1;
    if let Some((skipping, depth)) = &mut self.skip {
      if skipping == name {
        *depth -= 1;
        if *depth == 0 {
          self.skip = None;
        }
      }
      return;
    }
    match name {
      "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => self.block(2),
      "pre" => {
        self.pre = self.pre.saturating_sub(1);
        if self.pre == 0 && !self.plain {
          if !self.out.ends_with('\n') {
            self.out.push('\n');
          }
          self.out.push_str("```");
        }
      }
      "blockquote" => self.quote = self.quote.saturating_sub(1),
      "ul" | "ol" => {
        self.lists.pop();
      }
      "td" | "th" => {
        while self.out.ends_with(' ') {
          self.out.pop();
        }
        self.write(" | ");
      }
      "a" => {
        if let Some((target, start)) = self.links.pop() {
          // `start` can fall inside a character the output was rewritten with since; no link then
          let inner = self.out.get(start.min(self.out.len())..).unwrap_or_default().trim().to_owned();
          if let Some(target) = target.filter(|_| !self.plain && !inner.is_empty() && !inner.contains('\n') && start <= self.out.len()) {
            let lead = self.out.get(start..).unwrap_or_default();
            let pad = lead.len() - lead.trim_start().len();
            self.out.truncate(start + pad);
            self.out.push_str(&format!("[{inner}]({target})"));
          }
        }
      }
      "strong" | "b" | "em" | "i" | "code" | "kbd" | "samp" if !self.plain && self.pre == 0 => {
        let want: &str = match name {
          "strong" | "b" => "**",
          "em" | "i" => "*",
          _ => "`",
        };
        if let Some(pos) = self.marks.iter().rposition(|(m, _)| *m == want) {
          let (m, at) = self.marks.remove(pos);
          let trailing = self.out.ends_with(' ');
          while self.out.ends_with(' ') {
            self.out.pop();
          }
          if self.out.len() <= at {
            // Nothing inside: drop the opening marker
            self.out.truncate(at - m.len());
          } else {
            self.out.push_str(m);
          }
          if trailing {
            self.out.push(' ');
          }
        }
      }
      _ => {}
    }
    match name {
      "main" if self.main.0 > 0 => {
        self.main.0 -= 1;
        if self.main.0 == 0 {
          let span = (self.main.2, self.out.len());
          self.main.1.push(span);
        }
      }
      "article" if self.article.0 > 0 => {
        self.article.0 -= 1;
        if self.article.0 == 0 {
          let span = (self.article.2, self.out.len());
          self.article.1.push(span);
        }
      }
      _ => {}
    }
    if BLOCK2.contains(&name) && !nested {
      self.block(2);
    } else if BLOCK1.contains(&name) || nested {
      self.block(1);
    }
  }

  fn finish(mut self) -> Page {
    // A page whose main content is marked takes only that, when it is most of the text: the rest is chrome
    let whole = text_len(&self.out);
    let spans = if !self.main.1.is_empty() { &self.main.1 } else { &self.article.1 };
    let picked: String = spans.iter().map(|(a, b)| self.out.get(*a..*b).unwrap_or("")).collect::<Vec<_>>().join("\n\n");
    let n = text_len(&picked);
    if n >= 200 && n * 2 >= whole {
      self.out = picked;
    }
    Page { title: self.title, description: self.description, body: tidy(&self.out) }
  }
}

fn text_len(s: &str) -> usize {
  s.chars().filter(|c| !c.is_whitespace()).count()
}

/// Trailing spaces off every line, at most one blank line in a row, outer blank lines gone
fn tidy(s: &str) -> String {
  let mut out = String::with_capacity(s.len());
  let mut blank = 0;
  let mut fence = false;
  for line in s.lines() {
    let line = line.trim_end();
    if line.starts_with("```") {
      fence = !fence;
    }
    if line.is_empty() && !fence {
      blank += 1;
      continue;
    }
    if !out.is_empty() && blank > 0 {
      out.push('\n');
    }
    blank = 0;
    out.push_str(line);
    out.push('\n');
  }
  out.trim_end().to_owned()
}

/// Runs of whitespace as one space, trimmed
fn collapse(s: &str) -> String {
  s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// `&amp;`, `&#39;`, `&#x27;` and the common named entities; an unknown one stays as written
pub fn decode_entities(s: &str) -> String {
  if !s.contains('&') {
    return s.to_owned();
  }
  let mut out = String::with_capacity(s.len());
  let mut rest = s;
  while let Some(at) = rest.find('&') {
    out.push_str(rest.get(..at).unwrap_or_default());
    rest = rest.get(at..).unwrap_or_default();
    let end = rest.get(1..).and_then(|t| t.find(|c: char| !(c.is_ascii_alphanumeric() || c == '#'))).map(|n| n + 1);
    let decoded = end.filter(|e| *e > 1 && *e <= 32).and_then(|e| entity(rest.get(1..e)?).map(|c| (c, e)));
    match decoded {
      Some((c, e)) => {
        out.push(c);
        // The `;` is optional for the entities written without one (`&amp` in old markup)
        let after = rest.get(e..).unwrap_or_default();
        rest = after.strip_prefix(';').unwrap_or(after);
      }
      None => {
        out.push('&');
        rest = rest.get(1..).unwrap_or_default();
      }
    }
  }
  out.push_str(rest);
  out
}

fn entity(name: &str) -> Option<char> {
  if let Some(num) = name.strip_prefix('#') {
    let code = match num.strip_prefix(['x', 'X']) {
      Some(hex) => u32::from_str_radix(hex, 16).ok()?,
      None => num.parse().ok()?,
    };
    return char::from_u32(code).filter(|c| *c != '\0');
  }
  Some(match name {
    "amp" => '&',
    "lt" => '<',
    "gt" => '>',
    "quot" => '"',
    "apos" => '\'',
    "nbsp" => ' ',
    "ndash" => '–',
    "mdash" => '—',
    "hellip" => '…',
    "lsquo" => '‘',
    "rsquo" => '’',
    "ldquo" => '“',
    "rdquo" => '”',
    "laquo" => '«',
    "raquo" => '»',
    "middot" => '·',
    "bull" => '•',
    "copy" => '©',
    "reg" => '®',
    "trade" => '™',
    "times" => '×',
    "rarr" => '→',
    "larr" => '←',
    "deg" => '°',
    "euro" => '€',
    "pound" => '£',
    "yen" => '¥',
    "zwj" => '\u{200d}',
    "zwnj" => '\u{200c}',
    _ => return None,
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  fn md(html: &str) -> String {
    convert(html, "https://example.com/docs/page", false).body
  }

  #[test]
  fn structure_becomes_markdown() {
    let html = r#"<html><head><title>T &amp; co</title><meta name="description" content=" About  it "><style>p{}</style></head>
      <body><nav><a href="/">Home</a></nav><h1>Hello <em>there</em></h1>
      <p>Some <strong>bold</strong> text and <a href="../api?x=1">a link</a>.</p>
      <ul><li>one</li><li>two<ol start="3"><li>three</li><li>four</li></ol></li></ul>
      <pre><code>fn main() {
    let a = 1 &lt; 2;
}</code></pre><p>Use <code>cargo test</code> &mdash; done</p>
      <script>alert(1)</script><footer>(c) 2026</footer></body></html>"#;
    let page = convert(html, "https://example.com/docs/page", false);
    assert_eq!(page.title.as_deref(), Some("T & co"));
    assert_eq!(page.description.as_deref(), Some("About it"));
    assert_eq!(
      page.body,
      "# Hello *there*\n\nSome **bold** text and [a link](https://example.com/api?x=1).\n\n- one\n- two\n  3. three\n  4. four\n\n```\nfn main() {\n    let a = 1 < 2;\n}\n```\n\nUse `cargo test` — done"
    );
  }

  #[test]
  fn plain_text_drops_the_markers() {
    let page = convert("<h2>A</h2><p>b <a href='/x'>c</a> <b>d</b></p>", "https://e.com/", true);
    assert_eq!(page.body, "A\n\nb c d");
  }

  #[test]
  fn main_content_wins_over_page_chrome() {
    let filler = "word ".repeat(60);
    let html = format!("<div>Sidebar links and more sidebar</div><main><h2>Guide</h2><p>{filler}</p></main><div>Related posts</div>");
    let body = md(&html);
    assert!(body.starts_with("## Guide") && !body.contains("Sidebar") && !body.contains("Related"), "{body}");
    // A short <main> next to a long body is not trusted
    let html = format!("<main>tiny</main><p>{filler}</p>");
    assert!(md(&html).contains("tiny"));
  }

  #[test]
  fn tables_quotes_and_odd_markup() {
    let body = md("<table><tr><th>k</th><th>v</th></tr><tr><td>a</td><td>1</td></tr></table><blockquote><p>q1</p><p>q2</p></blockquote>");
    assert_eq!(body, "| k | v |\n| a | 1 |\n\n> q1\n\n> q2");
    // Unclosed tags, a stray `<`, uppercase names, an unknown entity, comments
    let body = md("<P>a < b &unknown; c<BR>d<!-- x --><div hidden>gone</div><span aria-hidden=\"true\">gone</span>");
    assert_eq!(body, "a < b &unknown; c\nd");
    assert_eq!(decode_entities("&#39;&#x4e2d;&amp&lt;"), "'中&<");
  }

  #[test]
  fn links_and_images_resolve_against_the_base() {
    let body = md(r#"<base href="https://cdn.example.org/a/"><a href="b.html">B</a> <a href="javascript:void(0)">J</a> <img alt="Logo" src="//img.example.org/l.png"><img src="x.png">"#);
    assert_eq!(body, "[B](https://cdn.example.org/a/b.html) J ![Logo](https://img.example.org/l.png)");
    // An empty `<b>` dropped from around a link's start leaves that start inside the wide text written after it
    assert!(md(r#"<p>文<b><a href="/x"></b>中文</a></p>"#).contains("中文"));
  }
}
