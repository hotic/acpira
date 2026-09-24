//! Attachment helpers shared by host and webview (mirror of src/shared/attachments.ts)

pub const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;
pub const MAX_TEXT_BYTES: usize = 256 * 1024;

const IMAGE_MIME: [(&str, &str); 5] =
  [(".png", "image/png"), (".jpg", "image/jpeg"), (".jpeg", "image/jpeg"), (".gif", "image/gif"), (".webp", "image/webp")];

/// MIME type for an image file name, None for anything that isn't an image the models accept
pub fn image_mime_of(name: &str) -> Option<&'static str> {
  let tail = name.rsplit(['/', '\\']).next().unwrap_or(name);
  let dot = tail.rfind('.')?;
  let ext = tail[dot..].to_lowercase();
  if ext.len() < 2 {
    return None;
  }
  IMAGE_MIME.iter().find(|(e, _)| *e == ext).map(|(_, m)| *m)
}

/// File extension to persist a blob of the given MIME type under
pub fn ext_of_mime(mime: &str) -> &'static str {
  IMAGE_MIME.iter().find(|(_, m)| *m == mime).map(|(e, _)| *e).unwrap_or(".bin")
}

/// Base64 payload size in bytes (without decoding)
pub fn base64_bytes(data: &str) -> usize {
  let pad = if data.ends_with("==") {
    2
  } else if data.ends_with('=') {
    1
  } else {
    0
  };
  (data.len() * 3 / 4).saturating_sub(pad)
}
