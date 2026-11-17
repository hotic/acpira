//! Blobs of host sessions: fetched from the host as its views name them, a few at a time, and put out once they land

use std::collections::HashSet;
use std::sync::Arc;

use anyhow::{Result, anyhow, bail};
use serde_json::value::RawValue;
use serde_json::{Value, json};

use acpira_shared::protocol::HostMsg;
use acpira_shared::session_patch::{LastTurnParts, ViewParts};

use super::{AUTO_BLOB_MAX, BLOB_SETTLE, HostRun, PARTIAL_KEEP, SessionManager};
use crate::session_manager::remote_view;
use crate::remote::client::Event;

impl SessionManager {
  /// Bring the blobs a host session's view shows into this machine's store, where every page loads them from (the
  /// webview's blob URLs, a paired phone's blob requests). True while the session's view is held for a fetch, its own
  /// or one already running; the latest view goes out once the last one ends
  /// Bring blobs a host session's view names into this machine's store, in the background: the view went out without
  /// them (`Shown::publish`), and it goes out again as they land (`blob_landed`)
  pub(super) fn fetch_host_blobs(self: &Arc<Self>, run: &Arc<HostRun>, id: &str, names: Vec<String>) {
    if names.is_empty() {
      return;
    }
    let Some(client) = self.deps.remote.as_ref().and_then(|c| c.client(&run.alias)) else { return };
    for name in names {
      let key = format!("{id}/{name}");
      if !run.blobs.lock().insert(key.clone()) {
        continue;
      }
      let (weak, run, id, client, store) = (Arc::downgrade(self), run.clone(), id.to_owned(), client.clone(), self.deps.store.clone());
      tokio::spawn(async move {
        let fetched = match run.slots.clone().acquire_owned().await {
          Ok(_slot) => fetch_host_blob(&client, &store, &id, &name).await,
          Err(_) => return,
        };
        run.blobs.lock().remove(&key);
        let Some(m) = weak.upgrade() else { return };
        match fetched {
          Ok(true) => m.blob_landed(&run, &id, &name),
          Ok(false) => m.log(&format!("remote run: {id}/{name} on {} is over {} MB, left there", run.alias, AUTO_BLOB_MAX >> 20)),
          Err(e) => m.log(&format!("remote run: {id}/{name} from {}: {e:#}", run.alias)),
        }
      });
    }
  }

  /// A blob of a shown host session is on this machine now: its view goes out again with it, once for the blobs that
  /// land within `BLOB_SETTLE` (each rebuild touches the whole view)
  fn blob_landed(self: &Arc<Self>, run: &Arc<HostRun>, id: &str, name: &str) {
    {
      let mut shown = run.shown.lock();
      let Some(s) = shown.get_mut(id) else { return };
      if !s.absent.remove(name) {
        return;
      }
    }
    if !run.settling.lock().insert(id.to_owned()) {
      return;
    }
    let (weak, run, id) = (Arc::downgrade(self), run.clone(), id.to_owned());
    tokio::spawn(async move {
      tokio::time::sleep(BLOB_SETTLE).await;
      run.settling.lock().remove(&id);
      let Some(m) = weak.upgrade() else { return };
      let msg = {
        let mut shown = run.shown.lock();
        let Some(s) = shown.get_mut(&id) else { return };
        let Some(parts) = s.parts.clone() else { return };
        let last = parts.0.last.as_ref().map(|l| LastTurnParts { head: l.head.clone(), blocks: l.blocks.clone() });
        s.publish(&id, &parts.0.head, m.next_mirror_rev(), parts.0.turns.clone(), last)
      };
      if let Some(msg) = msg {
        for v in m.viewers_on(&id, None) {
          v.emit(msg.clone());
        }
      }
    });
  }
}

/// Every `blob` a view (or a patch of one) names, whichever block holds it: images, attachments, embedded files
pub(super) fn blob_refs(v: &Value, session_id: &str, out: &mut Vec<String>) {
  match v {
    Value::Object(m) => {
      // Staged file attachments keep a file URI instead of a `blob` field. Only a file in this session's blob folder
      // is copied; workspace file links stay on the host.
      if m.get("kind").and_then(Value::as_str) == Some("file")
        && let Some(uri) = m.get("uri").and_then(Value::as_str)
        && let Some(path) = crate::platform::file_url::file_url_to_path(uri).map(std::path::PathBuf::from)
        && let Some(parent) = path.parent()
        && parent.file_name().is_some_and(|n| n == session_id)
        && parent.parent().and_then(|p| p.file_name()).is_some_and(|n| n == "sessions")
        && let Some(name) = path.file_name().and_then(|n| n.to_str())
      {
        out.push(name.to_owned());
      }
      for (k, x) in m {
        match (k.as_str(), x) {
          ("blob", Value::String(name)) => out.push(name.clone()),
          _ => blob_refs(x, session_id, out),
        }
      }
    }
    Value::Array(a) => a.iter().for_each(|x| blob_refs(x, session_id, out)),
    _ => {}
  }
}

/// The view pages get of a host session: its parts without the `absent` blobs. Every block and attachment kind takes a
/// missing `blob` for a file that could not be read, so a page shows the rest and loads the file once the next view
/// names it
pub(super) fn without_blobs(id: &str, parts: &ViewParts, absent: &HashSet<String>, rev: i64, epoch: u64) -> Option<HostMsg> {
  fn strip(v: &mut Value, absent: &HashSet<String>) {
    match v {
      Value::Object(m) => {
        if m.get("blob").and_then(Value::as_str).is_some_and(|b| absent.contains(b)) {
          m.remove("blob");
        }
        m.values_mut().for_each(|x| strip(x, absent));
      }
      Value::Array(a) => a.iter_mut().for_each(|x| strip(x, absent)),
      _ => {}
    }
  }
  // Only the pieces naming one are parsed again; the rest are shared as they are
  let mask = |raw: &RawValue| -> Option<Box<RawValue>> {
    if !absent.iter().any(|n| raw.get().contains(n.as_str())) {
      return Some(raw.to_owned());
    }
    let mut v: Value = serde_json::from_str(raw.get()).ok()?;
    strip(&mut v, absent);
    serde_json::value::to_raw_value(&v).ok()
  };
  let turns = parts.0.turns.iter().map(|t| mask(t)).collect::<Option<Vec<_>>>()?;
  let last = match &parts.0.last {
    Some(l) => Some(LastTurnParts { head: l.head.clone(), blocks: l.blocks.iter().map(|b| mask(b)).collect::<Option<Vec<_>>>()? }),
    None => None,
  };
  remote_view(id, &parts.0.head, rev, epoch, turns, last).map(|(_, msg)| msg)
}

/// One blob of a host session from the host's store into this machine's, written as it comes (`<sessions>/.partial/`)
/// and resumed from there after a cut. Blobs are named by their content, so a file that does not match the name is
/// refused. False when the host left it out as too large (`AUTO_BLOB_MAX`)
async fn fetch_host_blob(
  client: &crate::remote::client::RemoteClient,
  store: &crate::store::transcript_store::TranscriptStore,
  id: &str,
  name: &str,
) -> Result<bool> {
  use tokio::io::AsyncWriteExt;
  let ext = name.rsplit_once('.').map(|(_, e)| format!(".{e}")).unwrap_or_default();
  let dir = store.partial_dir();
  tokio::fs::create_dir_all(&dir).await?;
  let part = dir.join(format!("{id}-{name}"));
  let have = match tokio::fs::metadata(&part).await {
    Ok(m) if m.modified().ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age < PARTIAL_KEEP) => m.len(),
    _ => {
      let _ = tokio::fs::remove_file(&part).await;
      0
    }
  };
  let mut call = client.request("storeBlobGet", json!({ "id": id, "name": name, "offset": have, "max": AUTO_BLOB_MAX }));
  let mut file = tokio::fs::OpenOptions::new().create(true).append(true).open(&part).await?;
  let reply = loop {
    match call.next().await {
      Some(Event::Chunk { stream, bytes }) if stream == "data" => file.write_all(&bytes).await?,
      Some(Event::Chunk { .. }) => {}
      Some(Event::Done(r)) => break r.map_err(|e| anyhow!("{e}"))?,
      None => bail!("the connection ended"),
    }
  };
  file.flush().await?;
  drop(file);
  if reply.get("skipped").and_then(Value::as_bool) == Some(true) {
    if have == 0 {
      let _ = tokio::fs::remove_file(&part).await;
    }
    return Ok(false);
  }
  // A helper that does not resume (an older release a local build deployed) sent the whole blob after the kept piece
  if have > 0 && reply.get("offset").and_then(Value::as_u64) != Some(have) {
    let _ = tokio::fs::remove_file(&part).await;
    bail!("the host sent the blob from its start again; fetching it whole next time");
  }
  let (check, ext_of) = (part.clone(), ext.clone());
  let got = tokio::task::spawn_blocking(move || crate::store::transcript_store::blob_name_of_file(&ext_of, &check)).await??;
  if got != name {
    let _ = tokio::fs::remove_file(&part).await;
    bail!("the host's bytes do not match the name");
  }
  store.adopt_blob(id, name, part).await?;
  Ok(true)
}
