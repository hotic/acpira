//! The unstable `models` field of a session start answer (`availableModels` + `currentModelId`, switched with
//! `session/set_model`) for an agent that offers no model config option: Hermes Agent 0.22.5 lists its models only
//! there. It is presented to the rest of the engine as an ordinary model select, so the composer, the persona rounds
//! and the settings page's model probe all work unchanged; a set_config_option for it goes out as session/set_model

use std::collections::HashMap;

use serde_json::{Value, json};

/// The synthetic select's id: free whenever the agent has no model config option of its own
pub const CONFIG_ID: &str = "model";

const STARTS: [&str; 4] = ["session/new", "session/load", "session/resume", "session/fork"];

/// Per native session, the config options as presented (the agent's own plus the synthetic model select)
#[derive(Default)]
pub struct LegacyModels {
  sessions: parking_lot::Mutex<HashMap<String, Vec<Value>>>,
}

impl LegacyModels {
  /// Whether `method` starts a session: then the caller keeps the request's sessionId for `adopt` (a load answer
  /// carries none of its own)
  pub fn starts(method: &str) -> bool {
    STARTS.contains(&method)
  }

  /// A start answer with `models` and no model config option gains the synthetic select
  pub fn adopt(&self, method: &str, requested: Option<&str>, result: &mut Value) {
    if !Self::starts(method) {
      return;
    }
    let Some(select) = synthetic_select(result) else { return };
    let Some(sid) = result.get("sessionId").and_then(Value::as_str).or(requested).map(str::to_owned) else { return };
    let mut options = result.get("configOptions").and_then(Value::as_array).cloned().unwrap_or_default();
    options.push(select);
    result["configOptions"] = Value::Array(options.clone());
    self.sessions.lock().insert(sid, options);
  }

  /// `(sessionId, modelId)` when the request picks the synthetic select of a session this process presented one for
  pub fn intercept(&self, method: &str, params: &Value) -> Option<(String, String)> {
    if method != "session/set_config_option" || params.get("configId").and_then(Value::as_str) != Some(CONFIG_ID) {
      return None;
    }
    let sid = params.get("sessionId")?.as_str()?;
    let value = params.get("value")?.as_str()?;
    self.sessions.lock().contains_key(sid).then(|| (sid.to_owned(), value.to_owned()))
  }

  /// After session/set_model succeeded: the full config option set with the new current model, as a
  /// set_config_option answer carries it
  pub fn picked(&self, sid: &str, model: &str) -> Value {
    let mut sessions = self.sessions.lock();
    let options = sessions.entry(sid.to_owned()).or_default();
    for o in options.iter_mut().filter(|o| o.get("id").and_then(Value::as_str) == Some(CONFIG_ID)) {
      o["currentValue"] = Value::from(model);
    }
    json!({ "configOptions": options.clone() })
  }
}

fn synthetic_select(result: &Value) -> Option<Value> {
  let has_model = result
    .get("configOptions")
    .and_then(Value::as_array)
    .is_some_and(|opts| opts.iter().any(|o| o.get("category").and_then(Value::as_str) == Some("model") || o.get("id").and_then(Value::as_str) == Some(CONFIG_ID)));
  if has_model {
    return None;
  }
  let models = result.get("models")?;
  let options: Vec<Value> = models
    .get("availableModels")?
    .as_array()?
    .iter()
    .filter_map(|m| {
      let id = m.get("modelId")?.as_str()?;
      let name = m.get("name").and_then(Value::as_str).unwrap_or(id);
      let mut o = json!({ "value": id, "name": name });
      if let Some(d) = m.get("description").and_then(Value::as_str) {
        o["description"] = Value::from(d);
      }
      Some(o)
    })
    .collect();
  if options.is_empty() {
    return None;
  }
  Some(json!({
    "id": CONFIG_ID,
    "name": "Model",
    "category": "model",
    "type": "select",
    "currentValue": models.get("currentModelId").cloned().unwrap_or(Value::Null),
    "options": options,
  }))
}

#[cfg(test)]
mod tests {
  use super::*;

  fn hermes_new() -> Value {
    json!({
      "sessionId": "s1",
      "modes": { "availableModes": [], "currentModeId": "default" },
      "models": {
        "availableModels": [
          { "modelId": "custom:asgard:kimi-k3", "name": "kimi-k3", "description": "Provider: Asgard" },
          { "modelId": "custom:asgard:glm-5.3", "name": "glm-5.3" }
        ],
        "currentModelId": "custom:asgard:kimi-k3"
      }
    })
  }

  #[test]
  fn models_without_a_model_option_become_a_select_that_set_model_switches() {
    let l = LegacyModels::default();
    let mut r = hermes_new();
    l.adopt("session/new", None, &mut r);
    let select = &r["configOptions"][0];
    assert_eq!((select["category"].as_str(), select["currentValue"].as_str()), (Some("model"), Some("custom:asgard:kimi-k3")));
    assert_eq!(select["options"][0], json!({ "value": "custom:asgard:kimi-k3", "name": "kimi-k3", "description": "Provider: Asgard" }));
    let pick = json!({ "sessionId": "s1", "configId": "model", "value": "custom:asgard:glm-5.3" });
    assert_eq!(l.intercept("session/set_config_option", &pick), Some(("s1".into(), "custom:asgard:glm-5.3".into())));
    assert_eq!(l.picked("s1", "custom:asgard:glm-5.3")["configOptions"][0]["currentValue"], "custom:asgard:glm-5.3");
    // Another session of the same process, never presented a select, goes to the agent untouched
    assert_eq!(l.intercept("session/set_config_option", &json!({ "sessionId": "s2", "configId": "model", "value": "x" })), None);
  }

  #[test]
  fn an_agent_with_its_own_model_option_or_no_models_is_left_alone() {
    let l = LegacyModels::default();
    let mut own = hermes_new();
    own["configOptions"] = json!([{ "id": "llm", "category": "model", "type": "select", "options": [] }]);
    let before = own.clone();
    l.adopt("session/new", None, &mut own);
    assert_eq!(own, before);
    let mut bare = json!({ "sessionId": "s3" });
    l.adopt("session/new", None, &mut bare);
    assert!(bare.get("configOptions").is_none());
    // A load answer carries no sessionId of its own: the request's is used
    let mut loaded = hermes_new();
    loaded.as_object_mut().unwrap().remove("sessionId");
    l.adopt("session/load", Some("s4"), &mut loaded);
    assert!(l.intercept("session/set_config_option", &json!({ "sessionId": "s4", "configId": "model", "value": "m" })).is_some());
  }
}
