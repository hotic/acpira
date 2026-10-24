//! The shell's side of the envelope protocol (packages/shared/sidecar.ts, acpira-shared sidecar.rs). Page messages go out
//! and host messages come in as raw JSON: the shell reads only the few fields it acts on and relays the rest verbatim,
//! so a large session view is never parsed into a tree here

use serde::{Deserialize, Serialize, Serializer};
use serde_json::value::RawValue;
use serde_json::{Map, Value};

pub use acpira_shared::sidecar::SIDECAR_PROTOCOL_VERSION;

/// What a view opens on: a session id, or the most recent session
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Initial {
  Session(String),
  MostRecent,
}

impl Serialize for Initial {
  fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
    match self {
      Initial::Session(id) => s.serialize_str(id),
      Initial::MostRecent => {
        use serde::ser::SerializeMap;
        let mut m = s.serialize_map(Some(1))?;
        m.serialize_entry("mostRecent", &true)?;
        m.end()
      }
    }
  }
}

/// Shell → engine
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Outgoing {
  Hello {
    protocol_version: i64,
    request_id: String,
    client: Value,
    env: Value,
    settings: Map<String, Value>,
  },
  AttachView {
    view_id: String,
    host: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    initial: Option<Initial>,
    #[serde(skip_serializing_if = "Option::is_none")]
    blob_base: Option<String>,
  },
  DetachView {
    view_id: String,
  },
  WebviewMessage {
    view_id: String,
    message: Box<RawValue>,
  },
  PlatformResponse {
    request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
  },
  PlatformEvent {
    event: Value,
  },
  PreviewOpen {
    request_id: String,
    panel_id: String,
    url: String,
    picker: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    folder: Option<String>,
  },
  PreviewClose {
    panel_id: String,
  },
  Shutdown,
}

impl Outgoing {
  /// The view an envelope belongs to: a view closed before the handshake takes its queued envelopes with it
  pub fn view_id(&self) -> Option<&str> {
    match self {
      Outgoing::AttachView { view_id, .. } | Outgoing::DetachView { view_id } | Outgoing::WebviewMessage { view_id, .. } => Some(view_id),
      _ => None,
    }
  }

  pub fn line(&self) -> String {
    let mut s = serde_json::to_string(self).unwrap_or_default();
    s.push('\n');
    s
  }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SidecarInfo {
  #[serde(default)]
  pub version: String,
  #[serde(default)]
  pub pid: u32,
}

/// Engine → shell, every envelope's fields in one shape; `message` stays raw
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Incoming {
  #[serde(rename = "type")]
  pub kind: String,
  pub view_id: Option<String>,
  pub message: Option<Box<RawValue>>,
  pub request_id: Option<String>,
  pub request: Option<Value>,
  pub sessions_dir: Option<String>,
  pub features: Option<Vec<String>>,
  pub sidecar: Option<SidecarInfo>,
  pub reason: Option<String>,
  pub url: Option<String>,
  pub token: Option<String>,
  pub error: Option<String>,
}

/// A host message as the shell relays it: the raw JSON, its type, and the session it shows when it carries one (`init`'s
/// active session, a `session` push, a `sessionPatch`'s view)
#[derive(Debug, Clone)]
pub struct HostMessage {
  pub raw: Box<RawValue>,
  pub kind: String,
  pub session: Option<String>,
}

impl HostMessage {
  pub fn from_raw(raw: Box<RawValue>) -> HostMessage {
    let (kind, session) = peek(raw.get());
    HostMessage { raw, kind, session }
  }

  /// A message the shell makes itself (a toast, a chime, a menu command), peeked the same way
  pub fn from_value(v: &Value) -> HostMessage {
    HostMessage::from_raw(RawValue::from_string(v.to_string()).expect("a JSON value serializes to valid JSON"))
  }
}

#[derive(Deserialize)]
struct Kind {
  #[serde(rename = "type", default)]
  kind: String,
}

#[derive(Deserialize)]
struct Id {
  id: Option<String>,
}

#[derive(Deserialize)]
struct SessionPush {
  session: Option<Id>,
}

#[derive(Deserialize)]
struct PatchView {
  view: Option<Id>,
}

#[derive(Deserialize)]
struct PatchPush {
  patch: Option<PatchView>,
}

#[derive(Deserialize)]
struct InitState {
  active: Option<Id>,
}

#[derive(Deserialize)]
struct InitPush {
  state: Option<InitState>,
}

/// The type of a host message and the session it shows. serde skips every other field without building it, so this
/// costs one scan of the text per field read
pub fn peek(json: &str) -> (String, Option<String>) {
  let kind = serde_json::from_str::<Kind>(json).map(|k| k.kind).unwrap_or_default();
  let session = match kind.as_str() {
    "session" => serde_json::from_str::<SessionPush>(json).ok().and_then(|p| p.session?.id),
    "sessionPatch" => serde_json::from_str::<PatchPush>(json).ok().and_then(|p| p.patch?.view?.id),
    "init" => serde_json::from_str::<InitPush>(json).ok().and_then(|p| p.state?.active?.id),
    _ => None,
  };
  (kind, session)
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn envelopes_serialize_as_the_engine_reads_them() {
    let attach = Outgoing::AttachView { view_id: "V".into(), host: "editor".into(), initial: Some(Initial::MostRecent), blob_base: None };
    assert_eq!(
      serde_json::from_str::<Value>(&attach.line()).unwrap(),
      json!({ "type": "attachView", "viewId": "V", "host": "editor", "initial": { "mostRecent": true } })
    );
    let msg = Outgoing::WebviewMessage { view_id: "V".into(), message: RawValue::from_string(r#"{"type":"ready"}"#.into()).unwrap() };
    assert_eq!(msg.line(), "{\"type\":\"webviewMessage\",\"viewId\":\"V\",\"message\":{\"type\":\"ready\"}}\n");
    let ok = Outgoing::PlatformResponse { request_id: "r".into(), result: Some(Value::Null), error: None };
    assert_eq!(serde_json::from_str::<Value>(&ok.line()).unwrap(), json!({ "type": "platformResponse", "requestId": "r", "result": null }));
    // What the engine's own parser accepts
    for m in [attach, msg, ok, Outgoing::Shutdown, Outgoing::PreviewClose { panel_id: "p".into() }] {
      serde_json::from_str::<acpira_shared::sidecar::ShellMsg>(&m.line()).unwrap();
    }
  }

  #[test]
  fn peeks_the_type_and_the_shown_session_without_the_rest() {
    assert_eq!(peek(r#"{"type":"session","session":{"turns":[{"id":"t"}],"id":"S1"}}"#), ("session".into(), Some("S1".into())));
    assert_eq!(peek(r#"{"type":"sessionPatch","patch":{"view":{"id":"S2"},"tail":[]}}"#), ("sessionPatch".into(), Some("S2".into())));
    assert_eq!(peek(r#"{"type":"init","state":{"active":null}}"#), ("init".into(), None));
    assert_eq!(peek(r#"{"type":"init","state":{"active":{"id":"S3"}}}"#), ("init".into(), Some("S3".into())));
    // A session field of another shape on another message is not read
    assert_eq!(peek(r#"{"type":"toast","session":"x"}"#), ("toast".into(), None));
    // U+2028 inside a string stays one envelope
    let raw = RawValue::from_string("{\"type\":\"session\",\"session\":{\"id\":\"S\",\"t\":\"a\u{2028}b\"}}".into()).unwrap();
    assert_eq!(HostMessage::from_raw(raw).session.as_deref(), Some("S"));
  }
}
