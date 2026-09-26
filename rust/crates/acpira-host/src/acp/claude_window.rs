//! claude-agent-acp 0.81.0 seeds `usage_update.size` with its `DEFAULT_CONTEXT_WINDOW` (200000) unless the model id or
//! description spells `1m`, and learns the real window only from the turn's `result.modelUsage`. Its correction cache lives
//! in the adapter process, one per session here, so every fresh session's first turn on an alias such as `opus` (Opus 5.5,
//! natively 1M) streamed `used / 200000` until the turn ended. While the adapter still reports that placeholder, the size
//! is replaced by the window a finished turn confirmed for the same account and model, else by the built-in catalogue for
//! an official-endpoint model. A custom `ANTHROPIC_BASE_URL` is left alone: Claude Code keeps gateway aliases at 200k

use std::collections::HashMap;
use std::sync::LazyLock;

use acpira_shared::model_catalog::{Catalog, model_key};
use acpira_shared::model_shapes::is_model_control;
use acpira_shared::transcript::{SessionControls, SessionOption, SourceKind};
use parking_lot::Mutex;

/// The adapter's `DEFAULT_CONTEXT_WINDOW`
pub const PLACEHOLDER: f64 = 200_000.0;

type Key = (Option<String>, String);

/// Windows the adapter reported at the end of a completed turn, for this sidecar's lifetime
static CONFIRMED: LazyLock<Mutex<HashMap<Key, f64>>> = LazyLock::new(Default::default);

fn current_model(controls: &SessionControls) -> Option<&SessionOption> {
  let m = controls.options.iter().find(|c| is_model_control(c))?;
  let v = m.value.as_ref()?;
  m.options.iter().find(|o| &o.id == v)
}

/// Remember the window the adapter settled on once a turn has ended (its `result.modelUsage` has been applied by then)
pub fn confirm(account: Option<&str>, controls: &SessionControls, size: f64) {
  if size > 0.0
    && let Some(o) = current_model(controls)
  {
    CONFIRMED.lock().insert((account.map(str::to_owned), o.id.clone()), size);
  }
}

/// The window to show instead of a reported `size`, None to keep it
pub fn correct(account: Option<&str>, controls: &SessionControls, size: f64, catalog: &Catalog) -> Option<f64> {
  if size != PLACEHOLDER {
    return None;
  }
  let o = current_model(controls)?;
  if o.source.as_ref().is_some_and(|s| s.kind == SourceKind::Custom) {
    return None;
  }
  let known = CONFIRMED.lock().get(&(account.map(str::to_owned), o.id.clone())).copied();
  // Claude's display names drop the vendor ("Opus 5.5"), the catalogue keys keep it ("claude-opus-5-5")
  let window = known.or_else(|| {
    catalog
      .resolve(o)
      .or_else(|| catalog.get(&format!("claude-{}", model_key(&o.name))))
      .filter(|m| m.provider == "anthropic")
      .and_then(|m| m.context)
      .map(|c| c as f64)
  })?;
  (window != size).then_some(window)
}

#[cfg(test)]
mod tests {
  use super::*;
  use acpira_shared::model_catalog::CatalogFile;
  use acpira_shared::transcript::OptionSource;
  use serde_json::json;

  fn catalog() -> Catalog {
    Catalog::new(
      serde_json::from_value::<CatalogFile>(json!({ "source": "", "fetchedAt": "", "models": [
        { "provider": "anthropic", "id": "claude-opus-5-5", "name": "Claude Opus 5.5", "context": 1_000_000 },
        { "provider": "anthropic", "id": "claude-haiku-4-5", "name": "Claude Haiku 4.5", "context": 200_000 }
      ] }))
      .unwrap(),
    )
  }

  fn controls(id: &str, name: &str, custom: bool) -> SessionControls {
    let source = custom.then(|| OptionSource { id: "gw".into(), name: "gw".into(), kind: SourceKind::Custom });
    serde_json::from_value(json!({ "options": [{ "id": "model", "name": "Model", "category": "model", "value": id,
      "options": [{ "id": id, "name": name, "source": source }] }] }))
    .unwrap()
  }

  #[test]
  fn placeholder_is_replaced_from_the_catalogue_for_official_models_only() {
    let c = catalog();
    assert_eq!(correct(Some("t1"), &controls("opus", "Opus 5.5", false), PLACEHOLDER, &c), Some(1_000_000.0));
    assert_eq!(correct(Some("t1"), &controls("opus", "Opus 5.5", true), PLACEHOLDER, &c), None);
    assert_eq!(correct(Some("t1"), &controls("opus", "Opus 5.5", false), 1_000_000.0, &c), None);
    assert_eq!(correct(Some("t1"), &controls("haiku", "Haiku 4.5", false), PLACEHOLDER, &c), None);
    assert_eq!(correct(Some("t1"), &controls("default", "Default (recommended)", false), PLACEHOLDER, &c), None);
  }

  #[test]
  fn a_confirmed_window_wins_over_the_catalogue_per_account() {
    let c = catalog();
    let ctl = controls("claude-opus-5-5", "Opus 5.5", false);
    confirm(Some("t2"), &ctl, PLACEHOLDER);
    assert_eq!(correct(Some("t2"), &ctl, PLACEHOLDER, &c), None);
    assert_eq!(correct(Some("t3"), &ctl, PLACEHOLDER, &c), Some(1_000_000.0));
  }
}
