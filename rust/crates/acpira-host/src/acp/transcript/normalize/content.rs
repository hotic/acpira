//! What a tool row carries as output: text, exit codes and images (ACP image content, local resource links, the files
//! show_image / show_widget name, saved images), and the local images a reply embeds with Markdown

use std::collections::HashSet;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value};

use acpira_shared::attachments::{base64_bytes, image_mime_of};
use acpira_shared::transcript::*;

use crate::acp::transcript::diff::diff_lines;
use crate::json::{slice16, str_of, text_of};
use crate::limits::{MAX_OUT_IMAGE_BYTES, TOOL_OUTPUT_MAX};
use crate::util::js_num;

use super::tool_kind::{DATA_URL, SAVED_IMAGE_PATH};
use super::tool_target::json_strings;
use super::{ToolCtx, text_of_content};
use crate::platform::file_url::file_url_to_path;

/// A Markdown image (`![alt](src)`, `![alt](<src with spaces>)`, optional `"title"`); group 1 or 2 is the source as written
static MARKDOWN_IMAGE: LazyLock<Regex> =
  LazyLock::new(|| acpira_shared::re::compile(r#"!\[[^\]\n]*\]\(\s*(?:<([^>\n]+)>|([^\s()]+))(?:\s+"[^"\n]*")?\s*\)"#));
/// How far back a new chunk's scan starts: an image whose `)` arrives in the chunk may have opened in an earlier one
const MARKDOWN_IMAGE_LOOKBACK: usize = 1024;

/// Local image files a reply embeds with Markdown image syntax are read into the blob store, so the webview can show them
/// (it may only load blobs). Each source is tried once: a missing or refused file is recorded without a blob and renders as
/// a named placeholder. Web and data URLs are left to the renderer.
pub(super) fn resolve_markdown_images(b: &mut TextBlock, from: usize, ctx: &ToolCtx) {
  let Some(save) = ctx.save_image_file.as_ref() else { return };
  let mut start = from.saturating_sub(MARKDOWN_IMAGE_LOOKBACK);
  while !b.markdown.is_char_boundary(start) {
    start -= 1;
  }
  let found: Vec<String> = MARKDOWN_IMAGE
    .captures_iter(b.markdown.get(start..).unwrap_or_default())
    .filter_map(|m| m.get(1).or_else(|| m.get(2)).map(|x| x.as_str().trim().to_owned()))
    .collect();
  for src in found {
    if b.images.iter().flatten().any(|i| i.uri.as_deref() == Some(src.as_str())) {
      continue;
    }
    let Some(path) = markdown_image_path(&src, ctx.cwd.as_deref()) else { continue };
    let Some(mime) = image_mime_of(&path) else { continue };
    let blob = save(&path);
    b.images.get_or_insert_with(Vec::new).push(ImageRef { blob, mime_type: mime.to_owned(), uri: Some(src) });
  }
}

/// The file behind a Markdown image source: file:// URLs, absolute paths, and paths relative to the session cwd.
/// Anything with a scheme (https:, data:, blob:) is not a local file
fn markdown_image_path(src: &str, cwd: Option<&str>) -> Option<String> {
  if src.get(..5).is_some_and(|p| p.eq_ignore_ascii_case("file:")) {
    return file_url_to_path(src);
  }
  let decoded = percent_decode(src).unwrap_or_else(|| src.to_owned());
  let path = std::path::Path::new(&decoded);
  if path.is_absolute() {
    return Some(decoded);
  }
  if URL_SCHEME.is_match(src) || src.starts_with('#') || src.starts_with("//") {
    return None;
  }
  Some(std::path::Path::new(cwd?).join(path).to_string_lossy().into_owned())
}
static URL_SCHEME: LazyLock<Regex> = LazyLock::new(|| acpira_shared::re::compile(r"^[A-Za-z][A-Za-z0-9+.-]*:"));

/// `path` or `paths` from the tool's arguments, in order
pub(super) fn show_image_paths(args: &Map<String, Value>) -> Vec<String> {
  let one = args.get("path").and_then(Value::as_str).into_iter();
  let many = args.get("paths").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str);
  one.chain(many).map(str::trim).filter(|p| !p.is_empty()).map(str::to_owned).collect()
}

/// A finished show_image call: the files it named (kept as locations while it ran) become its images
pub(super) fn attach_shown_images(b: &mut ToolCallBlock, ctx: Option<&ToolCtx>) {
  let is_image = |c: &ToolContent| matches!(c, ToolContent::Image(_));
  if b.status != ToolStatus::Completed || b.content.as_ref().is_some_and(is_image) || b.contents.as_ref().is_some_and(|l| l.iter().any(is_image)) {
    return;
  }
  let Some(save) = ctx.and_then(|c| c.save_image_file.as_ref()) else { return };
  let images: Vec<ToolContent> = b
    .locations
    .iter()
    .flatten()
    .filter_map(|l| {
      let path = local_path_of(&l.path)?;
      let mime = image_mime_of(&path)?;
      Some(ToolContent::Image(ImageRef { blob: Some(save(&path)?), mime_type: mime.to_owned(), uri: Some(l.path.clone()) }))
    })
    .collect();
  if images.is_empty() {
    return;
  }
  // The images come first: the tool's receipt text ("Shown to the user") is for the model, not the reader
  let mut list = images;
  list.extend(b.contents.take().unwrap_or_else(|| b.content.iter().cloned().collect()));
  b.content = list.first().cloned();
  b.contents = if list.len() > 1 { Some(list) } else { None };
}

/// A finished show_widget call: its HTML file is read into the blob store as a `text/html` image content ahead of the
/// receipt, which older engines still read (a new content type would make them refuse the whole record). The webview
/// runs it in a sandboxed frame (`AgentWidget.tsx`). Read once, when the call completes: a later edit of the file is a
/// new call
pub(super) fn attach_shown_widget(b: &mut ToolCallBlock, ctx: Option<&ToolCtx>) {
  let is_widget = |c: &ToolContent| matches!(c, ToolContent::Image(i) if i.mime_type == WIDGET_MIME);
  if b.status != ToolStatus::Completed || b.content.as_ref().is_some_and(is_widget) || b.contents.as_ref().is_some_and(|l| l.iter().any(is_widget)) {
    return;
  }
  let Some(save) = ctx.and_then(|c| c.save_image_file.as_ref()) else { return };
  let Some(widget) = b.locations.iter().flatten().find_map(|l| {
    let path = local_path_of(&l.path)?;
    crate::host_mcp::widget_mime_of(&path)?;
    Some(ToolContent::Image(ImageRef { blob: Some(save(&path)?), mime_type: WIDGET_MIME.to_owned(), uri: Some(l.path.clone()) }))
  }) else {
    return;
  };
  let mut list = vec![widget];
  list.extend(b.contents.take().unwrap_or_else(|| b.content.iter().cloned().collect()));
  b.content = list.first().cloned();
  b.contents = if list.len() > 1 { Some(list) } else { None };
}

/// The MIME type a widget's HTML is stored under
pub const WIDGET_MIME: &str = "text/html";

/// An image generation tool that answers with the saved file's path instead of an image block (Grok's `image_gen` returns the
/// absolute path as text): the files named in its text results or rawOutput are read into the blob store and appended as images
pub(super) fn attach_saved_images(b: &mut ToolCallBlock, u: &Value, ctx: Option<&ToolCtx>) {
  let is_image = |c: &ToolContent| matches!(c, ToolContent::Image(_));
  if b.content.as_ref().is_some_and(is_image) || b.contents.as_ref().is_some_and(|l| l.iter().any(is_image)) {
    return;
  }
  let Some(save) = ctx.and_then(|c| c.save_image_file.as_ref()) else { return };
  let mut texts: Vec<&str> = vec![];
  for c in b.contents.iter().flatten().chain(b.content.iter()) {
    if let ToolContent::Text { text } = c {
      texts.push(text);
    }
  }
  if let Some(out) = u.get("rawOutput") {
    json_strings(out, &mut texts);
  }
  let mut seen = HashSet::new();
  let images: Vec<ToolContent> = texts
    .iter()
    .flat_map(|x| SAVED_IMAGE_PATH.captures_iter(x).map(|m| m[1].to_owned()).collect::<Vec<_>>())
    .filter(|uri| seen.insert(uri.clone()))
    .filter_map(|uri| {
      let path = local_path_of(&uri)?;
      let mime = image_mime_of(&path)?;
      let blob = save(&path)?;
      Some(ToolContent::Image(ImageRef { blob: Some(blob), mime_type: mime.to_owned(), uri: Some(uri) }))
    })
    .collect();
  if images.is_empty() {
    return;
  }
  let mut list = b.contents.take().unwrap_or_else(|| b.content.iter().cloned().collect());
  list.extend(images);
  b.content = list.first().cloned();
  b.contents = if list.len() > 1 { Some(list) } else { None };
}

pub(super) fn append_exit_code(prev: &str, code: &str) -> String {
  let base = prev.trim_end_matches('\n');
  if base.is_empty() { format!("exit code {code}") } else { format!("{base}\nexit code {code}") }
}

/// codex-acp's { formatted_output, exit_code } receipt
pub(super) fn formatted_output(raw: &Value) -> Option<(String, Option<String>)> {
  let o = raw.as_object()?;
  let text = o.get("formatted_output")?.as_str()?.to_owned();
  let code = o.get("exit_code").and_then(Value::as_f64).filter(|c| *c != 0.0).map(js_num);
  Some((text, code))
}

const SHOWABLE_IMAGE: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];

/// ACP image content → a transcript image whose pixels go to the blob store; degrades to a note otherwise
pub(super) fn image_content(c: &Value, ctx: Option<&ToolCtx>) -> Option<ToolContent> {
  if str_of(c, "type") != Some("image") {
    return None;
  }
  let mut mime = str_of(c, "mimeType").unwrap_or("image/png").to_owned();
  let mut uri = text_of(c, "uri").map(str::to_owned);
  let mut data = text_of(c, "data").map(str::to_owned);
  let inline = data
    .as_deref()
    .or(uri.as_deref())
    .and_then(|x| DATA_URL.captures(x))
    .map(|m| (m.get(1).map(|x| x.as_str().to_owned()), m[2].to_owned()));
  if let Some((m, payload)) = inline {
    data = Some(payload);
    if let Some(m) = m {
      mime = m;
    }
    if uri.as_deref().is_some_and(|u| u.starts_with("data:")) {
      uri = None;
    }
  }
  let saver = ctx.and_then(|c| c.save_image.as_ref());
  let blob = match (&data, saver) {
    (Some(d), Some(save)) if SHOWABLE_IMAGE.contains(&mime.as_str()) && base64_bytes(d) as u64 <= MAX_OUT_IMAGE_BYTES => save(d, &mime),
    _ => None,
  };
  if blob.is_some() || (uri.is_some() && data.is_none()) {
    return Some(ToolContent::Image(ImageRef { blob, mime_type: mime, uri }));
  }
  Some(ToolContent::Text { text: if saver.is_some() { format!("[image: {mime}, not shown]") } else { "[image]".into() } })
}

/// A local path behind a resource_link's uri
fn local_path_of(uri: &str) -> Option<String> {
  if crate::platform::file_url::is_file_url(uri) {
    return file_url_to_path(uri);
  }
  std::path::Path::new(uri).is_absolute().then(|| uri.to_owned())
}

fn percent_decode(s: &str) -> Option<String> {
  crate::platform::file_url::percent_decode(s, cfg!(windows))
}

fn file_image_content(c: &Value, ctx: Option<&ToolCtx>) -> Option<ToolContent> {
  if str_of(c, "type") != Some("resource_link") {
    return None;
  }
  let uri = str_of(c, "uri")?;
  let path = local_path_of(uri)?;
  let mime = image_mime_of(&path)?;
  let blob = ctx.and_then(|x| x.save_image_file.as_ref()).and_then(|save| save(&path))?;
  Some(ToolContent::Image(ImageRef { blob: Some(blob), mime_type: mime.to_owned(), uri: Some(uri.to_owned()) }))
}

/// Every renderable content item, in wire order
pub(super) fn tool_contents(items: &[Value], ctx: Option<&ToolCtx>) -> Vec<ToolContent> {
  let mut out: Vec<ToolContent> = vec![];
  fn push_text(out: &mut Vec<ToolContent>, text: &str) {
    if text.is_empty() {
      return;
    }
    if let Some(ToolContent::Text { text: last }) = out.last_mut() {
      *last = slice16(&format!("{last}\n{text}"), TOOL_OUTPUT_MAX);
    } else {
      out.push(ToolContent::Text { text: slice16(text, TOOL_OUTPUT_MAX) });
    }
  }
  for item in items {
    match str_of(item, "type") {
      Some("diff") => {
        let old = str_of(item, "oldText").unwrap_or("").to_owned();
        let new = str_of(item, "newText").unwrap_or("").to_owned();
        out.push(ToolContent::Diff {
          lines: diff_lines(&old, &new),
          source: Some(DiffSource { path: str_of(item, "path").unwrap_or("").to_owned(), old_text: old, new_text: new, omitted: false }),
        });
      }
      Some("content") => {
        let content = item.get("content").unwrap_or(&Value::Null);
        match image_content(content, ctx).or_else(|| file_image_content(content, ctx)) {
          Some(ToolContent::Text { text }) => push_text(&mut out, &text),
          Some(img) => out.push(img),
          None => push_text(&mut out, &text_of_content(content)),
        }
      }
      _ => {}
    }
  }
  out
}

#[cfg(test)]
mod local_path_tests {
  use super::*;

  #[test]
  fn unicode_resource_paths_do_not_panic() {
    assert_eq!(local_path_of("中文图片.png"), None);
  }

  #[test]
  fn file_urls_do_not_include_the_query_or_fragment_in_the_filename() {
    assert_eq!(
      file_url_to_path("file:///C:/work/a%23b.png?raw=1#L20"),
      Some(if cfg!(windows) { "C:\\work\\a#b.png" } else { "/C:/work/a#b.png" }.into())
    );
  }
}
