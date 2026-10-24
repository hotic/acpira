//! Where a project's hooks file is read and its gate runs: this machine, or the SSH host its folder is on. On a host
//! every step goes through the helper (`remote/helper.rs`: `hooksLoad`, `hook`, `snapshot`, `read`), which runs the
//! same code there, so a hook script, its git snapshots and its context files are the host's own (docs/dev/remote.md)

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::run::{PromptVerdict, Verdict, prompt_verdict_of, verdict_of};
use super::snapshot::Snapshot;
use super::{Hook, HooksConfig};
use crate::remote::client::RemoteClient;

#[derive(Clone)]
pub enum HookHost {
  Local,
  Remote(RemoteClient),
}

impl HookHost {
  /// The hooks of the project `cwd` belongs to, as `super::load` reads them
  pub async fn load(&self, cwd: &str) -> Option<Result<HooksConfig, String>> {
    match self {
      HookHost::Local => super::load(cwd),
      HookHost::Remote(client) => match client.call("hooksLoad", json!({ "cwd": cwd })).await {
        Err(e) => Some(Err(format!("the hooks file on the host could not be read: {e}"))),
        Ok(r) => {
          if let Some(e) = r.value.get("error").and_then(Value::as_str) {
            return Some(Err(e.to_owned()));
          }
          let root = r.value.get("root").and_then(Value::as_str)?;
          let text = r.value.get("text").and_then(Value::as_str)?;
          let path = Path::new(root).join(super::HOOKS_FILE);
          Some(super::parse(Path::new(root), text).map_err(|e| format!("{}: {e}", path.display())))
        }
      },
    }
  }

  pub async fn run(&self, hook: &Hook, root: &Path, payload: &Value, env: &[(&str, String)]) -> Verdict {
    match self {
      HookHost::Local => super::run::run(hook, root, payload, env).await,
      HookHost::Remote(client) => match remote_output(client, hook, root, payload, env).await {
        Ok((code, out, err)) => verdict_of(code, &out, &err),
        Err(e) => Verdict::Broken(e),
      },
    }
  }

  pub async fn run_prompt(&self, hook: &Hook, root: &Path, payload: &Value, env: &[(&str, String)]) -> PromptVerdict {
    match self {
      HookHost::Local => super::run::run_prompt(hook, root, payload, env).await,
      HookHost::Remote(client) => match remote_output(client, hook, root, payload, env).await {
        Ok((code, out, err)) => prompt_verdict_of(code, &out, &err),
        Err(e) => PromptVerdict::Broken(e),
      },
    }
  }

  /// The dirty files of the watched work trees. A host that cannot be asked gives an empty snapshot, as a folder
  /// outside any work tree does
  pub async fn snapshot(&self, watch: &[PathBuf]) -> Snapshot {
    match self {
      HookHost::Local => Snapshot::take(watch).await,
      HookHost::Remote(client) => match client.call("snapshot", json!({ "watch": watch })).await {
        Ok(r) => Snapshot::from_json(&r.value),
        Err(_) => Snapshot::default(),
      },
    }
  }

  pub async fn in_work_tree(&self, dir: &Path) -> bool {
    match self {
      HookHost::Local => super::snapshot::in_work_tree(dir).await,
      HookHost::Remote(client) => client.call("snapshot", json!({ "probe": dir })).await.is_ok_and(|r| r.value.get("workTree").and_then(Value::as_bool) == Some(true)),
    }
  }

  /// A context file's text
  pub async fn read_text(&self, path: &Path) -> Option<String> {
    match self {
      HookHost::Local => tokio::fs::read_to_string(path).await.ok(),
      HookHost::Remote(client) => {
        let r = client.call("read", json!({ "path": path })).await.ok()?;
        String::from_utf8(r.data).ok()
      }
    }
  }
}

async fn remote_output(client: &RemoteClient, hook: &Hook, root: &Path, payload: &Value, env: &[(&str, String)]) -> Result<(Option<i32>, String, String), String> {
  let env: serde_json::Map<String, Value> = env.iter().map(|(k, v)| ((*k).to_owned(), json!(v))).collect();
  let a = json!({ "run": hook.run, "timeoutMs": hook.timeout.as_millis() as u64, "root": root, "payload": payload, "env": env });
  let r = client.call("hook", a).await.map_err(|e| format!("`{}` could not run on the host: {e}", hook.run))?;
  if let Some(e) = r.value.get("broken").and_then(Value::as_str) {
    return Err(e.to_owned());
  }
  let text = |k: &str| r.value.get(k).and_then(Value::as_str).unwrap_or("").to_owned();
  Ok((r.value.get("code").and_then(Value::as_i64).map(|c| c as i32), text("stdout"), text("stderr")))
}
