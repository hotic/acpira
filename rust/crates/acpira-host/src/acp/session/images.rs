//! Agent images into the session blob store: base64 payloads and local image files a tool links to

use std::sync::{Arc, Weak};

use crate::acp::session::AcpSession;
use crate::acp::transcript::normalize::{FileImageSaver, ImageSaver};
use crate::store::transcript_store::blob_name;

/// An agent-emitted image payload → the session blob store; the content-hash name is known before the write lands
pub(crate) fn image_saver(me: Weak<AcpSession>) -> ImageSaver {
  use base64::Engine;
  Arc::new(move |data: &str, mime: &str| {
    let s = me.upgrade()?;
    let bytes = match base64::engine::general_purpose::STANDARD.decode(data.as_bytes()) {
      Ok(b) => b,
      Err(e) => {
        s.log(&format!("image payload rejected: {e}"));
        return None;
      }
    };
    let ext = acpira_shared::attachments::ext_of_mime(mime);
    let name = blob_name(ext, &bytes);
    let blobs = s.deps.blobs.clone();
    let (id, n2) = (s.id.clone(), name.clone());
    tokio::spawn(async move {
      if let Err(e) = blobs.save_blob(&id, ext, &bytes).await {
        s.log(&format!("image blob {n2}: {e}"));
      }
    });
    Some(name)
  })
}

/// A tool's resource_link to a local image: read synchronously so the blob name is known before the block renders
pub(crate) fn file_image_saver(me: Weak<AcpSession>) -> FileImageSaver {
  Arc::new(move |path: &str| {
    let s = me.upgrade()?;
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || acpira_shared::attachments::image_mime_of(path).is_none() || meta.len() > crate::limits::MAX_OUT_IMAGE_BYTES {
      return None;
    }
    let ext = std::path::Path::new(path).extension().map(|e| format!(".{}", e.to_string_lossy().to_lowercase())).unwrap_or_default();
    let bytes = match std::fs::read(path) {
      Ok(b) => b,
      Err(e) => {
        s.log(&format!("image file {path}: {e}"));
        return None;
      }
    };
    let name = blob_name(&ext, &bytes);
    let blobs = s.deps.blobs.clone();
    let (id, n2) = (s.id.clone(), name.clone());
    tokio::spawn(async move {
      if let Err(e) = blobs.save_blob(&id, &ext, &bytes).await {
        s.log(&format!("image blob {n2}: {e}"));
      }
    });
    Some(name)
  })
}
