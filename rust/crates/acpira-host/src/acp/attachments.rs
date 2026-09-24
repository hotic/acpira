//! Prompt staging (mirror of src/host/acp/attachments.ts): composer text + drafts → the wire prompt and the transcript attachments

use base64::Engine;
use serde_json::{Value, json};

use acpira_shared::attachments::{MAX_IMAGE_BYTES, MAX_TEXT_BYTES, base64_bytes, ext_of_mime, image_mime_of};
use acpira_shared::transcript::{Attachment, Draft};

use super::agent_registry::AgentDef;
use super::normalize::file_url_to_path;
use crate::i18n::{t, tp};
use crate::json::basename;
use crate::store::transcript_store::TranscriptStore;

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PreparedPrompt {
  pub blocks: Vec<Value>,
  pub attachments: Vec<Attachment>,
  pub problems: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PromptCaps {
  pub embedded_context: bool,
  pub image: bool,
  pub images_regardless: bool,
}

/// The advertised set, defaulting to permitted when the agent said nothing, overridden by the registry
pub fn prompt_caps_of(init: Option<&Value>, def: &AgentDef) -> PromptCaps {
  let p = init.and_then(|i| i.get("agentCapabilities")).and_then(|c| c.get("promptCapabilities"));
  let flag = |k: &str| p.and_then(|p| p.get(k)).and_then(Value::as_bool).unwrap_or(true);
  PromptCaps { embedded_context: flag("embeddedContext"), image: flag("image"), images_regardless: def.images_regardless }
}

pub async fn prepare_prompt(
  session_id: &str,
  text: &str,
  drafts: &[Draft],
  blobs: &TranscriptStore,
  caps: Option<PromptCaps>,
) -> PreparedPrompt {
  let mut out =
    PreparedPrompt { blocks: if text.is_empty() { vec![] } else { vec![json!({ "type": "text", "text": text })] }, ..Default::default() };
  let no_images = caps.is_some_and(|c| !c.image && !c.images_regardless);
  for d in drafts {
    match d {
      Draft::Image { mime_type, data, name } => {
        let label = name.clone().unwrap_or_else(|| t("common.image"));
        if no_images {
          out.problems.push(tp("host.imageUnsupported", &[("name", &label)]));
          continue;
        }
        if base64_bytes(data) > MAX_IMAGE_BYTES {
          out.problems.push(tp("host.imageTooBig", &[("name", &label), ("mb", &(MAX_IMAGE_BYTES >> 20).to_string())]));
          continue;
        }
        let bytes = B64.decode(data.as_bytes()).unwrap_or_default();
        let saved = stage(&mut out, blobs, session_id, ext_of_mime(mime_type), &bytes, &label).await;
        out.blocks.push(json!({ "type": "image", "mimeType": mime_type, "data": data }));
        out.attachments.push(Attachment::Image { blob: saved.map(|s| s.0), mime_type: mime_type.clone(), name: name.clone() });
      }
      Draft::Text { name, text } => {
        if text.len() > MAX_TEXT_BYTES {
          out.problems.push(tp("attach.tooBigText", &[("name", name), ("kb", &(MAX_TEXT_BYTES >> 10).to_string())]));
          continue;
        }
        let saved = stage(&mut out, blobs, session_id, ".txt", text.as_bytes(), name).await;
        if caps.is_some_and(|c| !c.embedded_context) {
          out.blocks.push(json!({ "type": "text", "text": format!("[Attachment: {name}]\n{text}\n[End of attachment: {name}]") }));
        } else {
          let uri = match &saved {
            Some((_, path)) => path_to_file_url(path),
            None => format!("attachment:///{}", encode_uri_component(name)),
          };
          out.blocks.push(json!({ "type": "resource", "resource": { "uri": uri, "mimeType": "text/plain", "text": text } }));
        }
        out.attachments.push(Attachment::Text { blob: saved.map(|s| s.0), name: name.clone() });
      }
      Draft::File { uri, name } => {
        if let Some((mime, bytes)) = read_image_file(uri).await {
          if no_images {
            out.problems.push(tp("host.imageUnsupported", &[("name", name)]));
            continue;
          }
          let saved = stage(&mut out, blobs, session_id, ext_of_mime(mime), &bytes, name).await;
          out.blocks.push(json!({ "type": "image", "mimeType": mime, "data": B64.encode(&bytes) }));
          out.attachments.push(Attachment::Image { blob: saved.map(|s| s.0), mime_type: mime.to_owned(), name: Some(name.clone()) });
        } else {
          let link_name = if name.is_empty() { basename(uri) } else { name.clone() };
          out.blocks.push(json!({ "type": "resource_link", "uri": uri, "name": link_name }));
          out.attachments.push(Attachment::File { uri: uri.clone(), name: name.clone() });
        }
      }
    }
  }
  out
}

async fn stage(
  out: &mut PreparedPrompt,
  blobs: &TranscriptStore,
  session_id: &str,
  ext: &str,
  bytes: &[u8],
  label: &str,
) -> Option<(String, String)> {
  match blobs.save_blob(session_id, ext, bytes).await {
    Ok((name, path)) => Some((name, path.to_string_lossy().into_owned())),
    Err(e) => {
      out.problems.push(tp("host.attachStageFailed", &[("label", label), ("error", &e.to_string())]));
      None
    }
  }
}

/// The inverse of prepare_prompt, for sending a persisted user turn again
pub async fn restore_drafts(session_id: &str, attachments: &[Attachment], blobs: &TranscriptStore) -> anyhow::Result<Vec<Draft>> {
  let mut out = vec![];
  for a in attachments {
    match a {
      Attachment::File { uri, name } => out.push(Draft::File { uri: uri.clone(), name: name.clone() }),
      Attachment::Image { blob: Some(b), mime_type, name } => {
        let bytes = blobs.read_blob(session_id, b).await?;
        out.push(Draft::Image { mime_type: mime_type.clone(), data: B64.encode(bytes), name: name.clone() });
      }
      Attachment::Text { blob: Some(b), name } => {
        let bytes = blobs.read_blob(session_id, b).await?;
        out.push(Draft::Text { name: name.clone(), text: String::from_utf8_lossy(&bytes).into_owned() });
      }
      _ => {}
    }
  }
  Ok(out)
}

async fn read_image_file(uri: &str) -> Option<(&'static str, Vec<u8>)> {
  let mime = image_mime_of(uri)?;
  if !uri.starts_with("file:") {
    return None;
  }
  let path = file_url_to_path(uri)?;
  let meta = tokio::fs::metadata(&path).await.ok()?;
  if meta.len() as usize > MAX_IMAGE_BYTES {
    return None;
  }
  Some((mime, tokio::fs::read(&path).await.ok()?))
}

/// One-line description of what a prompt carried
pub fn describe_drafts(items: &[Attachment]) -> String {
  let images = items.iter().filter(|d| matches!(d, Attachment::Image { .. })).count();
  let mut parts: Vec<String> = vec![];
  if images > 0 {
    parts.push(tp("host.images", &[("n", &images.to_string())]));
  }
  for d in items {
    match d {
      Attachment::Text { name, .. } | Attachment::File { name, .. } if !name.is_empty() => parts.push(name.clone()),
      _ => {}
    }
  }
  parts.join(&t("common.listSep"))
}

pub fn describe_draft_list(items: &[Draft]) -> String {
  let images = items.iter().filter(|d| matches!(d, Draft::Image { .. })).count();
  let mut parts: Vec<String> = vec![];
  if images > 0 {
    parts.push(tp("host.images", &[("n", &images.to_string())]));
  }
  for d in items {
    match d {
      Draft::Text { name, .. } | Draft::File { name, .. } if !name.is_empty() => parts.push(name.clone()),
      _ => {}
    }
  }
  parts.join(&t("common.listSep"))
}

/// url.pathToFileURL(p).href
pub fn path_to_file_url(p: &str) -> String {
  let path = if cfg!(windows) { format!("/{}", p.replace('\\', "/")) } else { p.to_owned() };
  let mut out = String::from("file://");
  for b in path.bytes() {
    let keep = b.is_ascii_alphanumeric() || b"/-._~!$&'()*+,;=:@".contains(&b);
    if keep {
      out.push(b as char);
    } else {
      out.push_str(&format!("%{b:02X}"));
    }
  }
  out
}

/// encodeURIComponent
pub fn encode_uri_component(s: &str) -> String {
  let mut out = String::new();
  for b in s.bytes() {
    if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
      out.push(b as char);
    } else {
      out.push_str(&format!("%{b:02X}"));
    }
  }
  out
}
