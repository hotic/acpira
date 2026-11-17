//! What a plugin process may ask of the engine: its store, a log line, the composer, and whatever `HostCall` serves

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use acpira_rpc::cancel::Cancel;
use acpira_rpc::rpc::{BoxFuture, Inbound, RpcError};

use super::kv::Kv;
use super::{Runtimes, Want};


/// How long a call to the engine waits for the session API to come up (the engine starts processes first)
const HOST_CALL_WAIT: Duration = Duration::from_secs(10);

/// Longest text a process may put into the composer
const COMPOSER_MAX_CHARS: usize = 100_000;

/// What a process may ask of the engine: its store, a log line, and whatever `HostCall` serves
pub(super) struct PluginInbound {
  pub(super) id: String,
  pub(super) kv: Arc<Kv>,
  pub(super) runtimes: std::sync::Weak<Runtimes>,
  /// The version this process runs: what it may call is what that version's card showed, not the newest version's
  pub(super) want: Want,
}

impl Inbound for PluginInbound {
  fn notification(&self, method: &str, params: Value) {
    if method == "log"
      && let Some(rt) = self.runtimes.upgrade()
    {
      let text = params.get("message").and_then(Value::as_str).unwrap_or_default();
      (rt.log)(&format!("plugin {}: {}", self.id, text.chars().take(2000).collect::<String>()));
    }
  }

  fn request(&self, method: String, params: Value, _cancel: Cancel) -> BoxFuture<Result<Value, RpcError>> {
    let key = || params.get("key").and_then(Value::as_str).map(str::to_owned).ok_or_else(|| RpcError::new(-32602, "`key` is needed"));
    let r = match method.as_str() {
      "storage/get" => key().map(|k| json!({ "value": self.kv.get(&k) })),
      // Under the store's file lock, on what the file holds then (another engine may have written it)
      "storage/set" | "storage/delete" => {
        let (kv, value, set) = (self.kv.clone(), params.get("value").cloned().unwrap_or(Value::Null), method == "storage/set");
        let key = key();
        return Box::pin(async move {
          let key = key?;
          kv.change(move |m| {
            if set {
              m.insert(key, value);
            } else {
              m.remove(&key);
            }
          })
          .await
          .map(|()| json!({}))
          .map_err(|e| RpcError::new(-32000, e))
        });
      }
      // Compare and set under the same lock: `value` goes in only while the key still holds `expect` (null: absent).
      // What the SDK's migration lock is made of, so two engines starting one plugin migrate its data once
      "storage/swap" => {
        let (kv, expect, value) = (self.kv.clone(), params.get("expect").cloned().unwrap_or(Value::Null), params.get("value").cloned().unwrap_or(Value::Null));
        let key = key();
        return Box::pin(async move {
          let key = key?;
          let swapped = Arc::new(AtomicBool::new(false));
          let done = swapped.clone();
          kv.change(move |m| {
            if m.get(&key).cloned().unwrap_or(Value::Null) == expect {
              if value.is_null() {
                m.remove(&key);
              } else {
                m.insert(key, value);
              }
              done.store(true, Ordering::Relaxed);
            }
          })
          .await
          .map(|()| json!({ "swapped": swapped.load(Ordering::Relaxed) }))
          .map_err(|e| RpcError::new(-32000, e))
        });
      }
      "storage/list" => Ok(json!({ "keys": self.kv.keys(params.get("prefix").and_then(Value::as_str).unwrap_or("")) })),
      // To the plugin's own frames on every page of this engine, and text for the composer on screen
      "ui/post" => match (params.get("message"), self.runtimes.upgrade()) {
        (None, _) => Err(RpcError::new(-32602, "`message` is needed")),
        (_, None) => Err(RpcError::new(-32000, "the engine is closing")),
        (Some(message), Some(rt)) => Ok(json!({ "pages": rt.to_pages(acpira_shared::protocol::HostMsg::PluginPost { plugin: self.id.clone(), message: message.clone() }) })),
      },
      "ui/composer" => match (params.get("text").and_then(Value::as_str), self.runtimes.upgrade()) {
        (None, _) => Err(RpcError::new(-32602, "`text` is needed")),
        (_, None) => Err(RpcError::new(-32000, "the engine is closing")),
        (Some(text), Some(rt)) => {
          let replace = params.get("replace").and_then(Value::as_bool).unwrap_or(false);
          let session = params.get("session").and_then(Value::as_str).map(str::to_owned);
          let msg = acpira_shared::protocol::HostMsg::PluginComposer { plugin: self.id.clone(), text: text.chars().take(COMPOSER_MAX_CHARS).collect(), replace, session };
          Ok(json!({ "pages": rt.to_pages(msg) }))
        }
      },
      // Another plugin's method: answered on its own task, since the callee may take its full time limit
      "plugin/call" => {
        let (Some(rt), Some(plugin), Some(name)) = (self.runtimes.upgrade(), params.get("plugin").and_then(Value::as_str), params.get("method").and_then(Value::as_str)) else {
          return Box::pin(async { Err(RpcError::new(-32602, "`plugin` and `method` are needed")) });
        };
        let (plugin, name, args, from) = (plugin.to_owned(), name.to_owned(), params.get("params").cloned().unwrap_or(Value::Null), self.id.clone());
        // Only the plugins its manifest names (`dependencies`, `optionalDependencies`): what the user saw on its card
        if !self.want.may_call(&plugin) {
          return Box::pin(async move { Err(RpcError::new(-32000, format!("plugin {from} does not list `{plugin}` in its dependencies"))) });
        }
        return Box::pin(async move { rt.rpc(&plugin, &name, args, &from).await.map_err(|e| RpcError::new(-32000, e)) });
      }
      "providers/set" => match (params.get("providers").and_then(Value::as_array), self.runtimes.upgrade()) {
        (None, _) => Err(RpcError::new(-32602, "`providers` must be an array")),
        (_, None) => Err(RpcError::new(-32000, "the engine is closing")),
        (Some(list), Some(rt)) => rt.set_live_providers(&self.id, list).map(|n| json!({ "providers": n })).map_err(|e| RpcError::new(-32602, e)),
      },
      _ => {
        // The engine starts the processes before its session manager is up: a call that early waits for it a while
        let (rt, id) = (self.runtimes.clone(), self.id.clone());
        return Box::pin(async move {
          let t = Instant::now();
          loop {
            let (call, coming) = rt.upgrade().map(|rt| (rt.host_call.read().clone(), rt.host_coming.load(Ordering::Relaxed))).unwrap_or((None, false));
            match call {
              Some(f) => return f(id, method, params).await,
              None if coming && t.elapsed() < HOST_CALL_WAIT => tokio::time::sleep(Duration::from_millis(50)).await,
              None => return Err(RpcError::method_not_found(&method)),
            }
          }
        });
      }
    };
    Box::pin(async move { r })
  }
}
