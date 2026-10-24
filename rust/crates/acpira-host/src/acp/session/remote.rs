//! A session whose folder is on an SSH host (`ssh://<alias>/<path>`, docs/dev/remote.md). Under local run only the
//! built-in agent can work there: it runs here, and its project tools come to the engine as `_acpira/remoteTool`
//! requests, which go to the host's helper (`remote::helper`, the `tool` op) over the host's connection. The helper's
//! progress chunks go back to the agent as `_acpira/remoteProgress` notifications, its spilled outputs ride in the
//! answer. A turn's cancel reaches the agent's request as `$/cancel_request`, which drops the helper call and with it
//! the command's process group on the host

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use acpira_shared::remote::{RemoteFolder, parse_remote_folder};
use acpira_shared::transcript::ToolKind;
use serde_json::{Value, json};

use crate::acp::session::AcpSession;
use crate::acp::transport::cancel::Cancel;
use crate::acp::transport::rpc::RpcError;
use crate::remote::client::Event;

/// The agent's request (`acpira_agent::remote::TOOL`)
pub const TOOL: &str = "_acpira/remoteTool";
/// The progress notification back to the agent (`acpira_agent::remote::PROGRESS`)
pub const PROGRESS: &str = "_acpira/remoteProgress";
/// The `_meta` key of session requests naming the host (`acpira_agent::remote::META`)
pub const META: &str = "acpira/remote";
/// How long a tool call waits for the host's connection to come (back) up
const CONNECT_WAIT: Duration = Duration::from_secs(60);
/// How long opening a session waits for the host to read its prompt files
const PLACES_WAIT: Duration = Duration::from_secs(30);

/// A notification to the agent process a request came from
pub type Notify = Arc<dyn Fn(&str, Value) + Send + Sync>;

impl AcpSession {
  /// The session's folder on an SSH host, when it is one
  pub fn remote_folder(&self) -> Option<RemoteFolder> {
    parse_remote_folder(&self.cwd)
  }

  /// Where the agent process starts and reads files of its own (model facts, a warm process): the session folder, or
  /// for a folder on an SSH host this machine's home
  pub(crate) fn process_cwd(&self) -> String {
    match self.remote_folder() {
      Some(_) => crate::store::data_dir::home_dir().to_string_lossy().into_owned(),
      None => self.cwd.clone(),
    }
  }

  /// Refuse an agent that cannot work in the session's folder: under local run only the built-in agent's tools go to
  /// the host
  pub(crate) fn check_remote_agent(&self, name: &str) -> anyhow::Result<()> {
    match self.remote_folder() {
      Some(f) if self.agent != crate::acp::agents::registry::SELF_AGENT_ID => {
        anyhow::bail!(crate::i18n::tp("host.remoteBuiltinOnly", &[("agent", name), ("host", &f.alias)]))
      }
      _ => Ok(()),
    }
  }

  /// A session request (`session/new`, load, resume) for the session's folder: a folder on an SSH host goes as the
  /// host's own path, with the host named in `_meta` (`acpira_agent::remote`) together with the folder's prompt files
  /// as the host reads them (AGENTS.md, prompt overrides, skills). A host that cannot be asked leaves them out: the
  /// session then has none, and its tool calls wait for the connection as usual
  pub(crate) async fn with_remote(&self, mut req: Value) -> Value {
    let Some(f) = self.remote_folder() else { return req };
    req["cwd"] = json!(f.path);
    if !req.get("_meta").is_some_and(Value::is_object) {
      req["_meta"] = json!({});
    }
    let mut meta = json!({ "alias": f.alias });
    match self.host_places(&f).await {
      Ok(places) => meta["places"] = places,
      Err(e) => self.log(&format!("remote: the prompt files on {} could not be read: {e:#}", f.alias)),
    }
    req["_meta"][META] = meta;
    req
  }

  async fn host_places(&self, f: &RemoteFolder) -> anyhow::Result<Value> {
    let connections = self.deps.remote.clone().ok_or_else(|| anyhow::anyhow!("this engine has no SSH connections"))?;
    let client = connections.ensure(&f.alias, PLACES_WAIT).await?;
    Ok(client.call("places", json!({ "cwd": f.path })).await?.value)
  }

  /// Stop the background jobs the agent's sessions (`sids`: the session's own and its summoned children's) left on the
  /// host, as a local agent's jobs end with its process. A host not connected now has no helper holding them
  pub(crate) async fn close_host_jobs(&self, sids: Vec<String>) {
    let (Some(f), Some(connections)) = (self.remote_folder(), self.deps.remote.as_ref()) else { return };
    let Some(client) = connections.client(&f.alias) else { return };
    for sid in sids {
      let call = client.call("tool", json!({ "phase": "close", "sessionId": sid }));
      let failed = match tokio::time::timeout(super::lifecycle::CLOSE_GRACE, call).await {
        Ok(Ok(_)) => continue,
        Ok(Err(e)) => e.to_string(),
        Err(_) => "timed out".to_owned(),
      };
      self.log(&format!("remote: the jobs of {sid} on {} could not be stopped: {failed}", f.alias));
    }
  }

  /// `beforeEdit` for a write about to run on the host that no permission request showed the gate (full access, an
  /// always-allow): every write the agent makes there is asked, not only audited at the end of the turn. Some: the
  /// answer that refuses the call, with the hook's reason for the model
  async fn remote_write_gate(&self, req: &Value) -> Option<Value> {
    if req.get("phase").and_then(Value::as_str) != Some("run") {
      return None;
    }
    let files: Vec<String> = req.pointer("/action/write/files")?.as_array()?.iter().filter_map(|f| f.get("path")?.as_str().map(str::to_owned)).collect();
    let id = req.get("callId").and_then(Value::as_str)?.to_owned();
    if files.is_empty() || self.core.lock().hooks.asked.contains(&id) {
      return None;
    }
    let input = json!({ "file_path": files.first(), "files": files });
    let reason = self.hooks_check_edit(&id, ToolKind::Edit, files, Some(input)).await?;
    let text = format!("Blocked by the workspace's beforeEdit hook: {reason}. The file was not changed; do not retry the change as it is.");
    Some(json!({ "output": { "model": text, "isError": true, "content": [{ "type": "content", "content": { "type": "text", "text": text } }] } }))
  }

  /// Carry out one `_acpira/remoteTool` request on the session's host; `notify` reaches the agent process that asked
  /// (the session's own, or a relayed child's)
  pub(crate) async fn on_remote_tool(self: &Arc<Self>, req: Value, cancel: Cancel, notify: Notify) -> Result<Value, RpcError> {
    let folder = self.remote_folder().ok_or_else(|| RpcError::new(-32602, "this session's folder is not on an SSH host"))?;
    let connections = self.deps.remote.clone().ok_or_else(|| RpcError::new(-32603, "this engine has no SSH connections"))?;
    let client = tokio::select! {
      c = connections.ensure(&folder.alias, CONNECT_WAIT) => c.map_err(|e| RpcError::new(-32603, format!("{e:#}")))?,
      () = cancel.cancelled() => return Err(RpcError::cancelled()),
    };
    if let Some(blocked) = self.remote_write_gate(&req).await {
      return Ok(blocked);
    }
    let mut a = req.clone();
    // The host's folder is the session's, whatever the agent sent
    a["cwd"] = json!(folder.path);
    let (sid, call_id) = (req.get("sessionId").cloned().unwrap_or(Value::Null), req.get("callId").cloned().unwrap_or(Value::Null));
    let mut call = client.request("tool", a);
    let mut progress: Vec<u8> = vec![];
    let mut spills: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    loop {
      let ev = tokio::select! {
        ev = call.next() => ev,
        // Dropping the call cancels it on the host
        () = cancel.cancelled() => return Err(RpcError::cancelled()),
      };
      match ev {
        Some(Event::Chunk { stream, bytes }) if stream == "progress" => {
          progress.extend_from_slice(&bytes);
          while let Some(end) = progress.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = progress.drain(..=end).collect();
            if let Ok(update) = serde_json::from_slice::<Value>(&line) {
              notify(PROGRESS, json!({ "sessionId": sid, "toolCallId": call_id, "update": update }));
            }
          }
        }
        Some(Event::Chunk { stream, bytes }) => {
          if let Some(name) = stream.strip_prefix("spill:") {
            spills.entry(name.to_owned()).or_default().extend_from_slice(&bytes);
          }
        }
        Some(Event::Done(Ok(mut value))) => {
          if !spills.is_empty() {
            value["spills"] = spills.iter().map(|(name, b)| json!({ "name": name, "text": String::from_utf8_lossy(b) })).collect();
          }
          return Ok(value);
        }
        Some(Event::Done(Err(e))) => return Err(RpcError::new(-32603, e.to_string())),
        None => return Err(RpcError::new(-32603, "the connection to the host ended before the answer; the outcome is unknown")),
      }
    }
  }
}
