//! Agent-specific model provenance (mirror of src/shared/modelSources.ts)

use std::collections::BTreeMap;

use crate::transcript::{ConfigControl, OptionSource, SessionOption, SourceKind};

/// What the host read off the agent's config: Grok aliases, Pi / OpenCode provider ids, or `ALL` for the single endpoint
/// Codex / Claude route every model through
pub type ModelSources = BTreeMap<String, OptionSource>;

pub const ALL: &str = "*";

fn grok_builtin(id: &str) -> bool {
  id == "grok-build" || id.strip_prefix("grok-").is_some_and(|r| r.starts_with(|c: char| c.is_ascii_digit()))
}

/// `provider/model` ids (Pi, OpenCode): the provider becomes the source, and the `Provider/` label the adapter put in
/// front of the name moves into it. Already-sourced options are left alone, so a republish never strips twice
fn provider_prefixed(option: &mut SessionOption, configured: &ModelSources) {
  if option.source.is_some() {
    return;
  }
  let Some((provider, _)) = option.id.split_once('/').filter(|(p, _)| !p.is_empty()) else { return };
  let provider = provider.to_owned();
  let label = match option.name.split_once('/') {
    Some((label, rest)) if !label.trim().is_empty() && !rest.trim().is_empty() => {
      let label = label.trim().to_owned();
      option.name = rest.trim().to_owned();
      Some(label)
    }
    _ => None,
  };
  option.source = Some(match configured.get(&provider) {
    Some(s) => OptionSource { name: label.unwrap_or_else(|| s.name.clone()), ..s.clone() },
    None => OptionSource { id: provider.clone(), name: label.unwrap_or(provider), kind: SourceKind::Official },
  });
}

pub fn apply_model_sources(agent: &str, controls: &mut [ConfigControl], configured: &ModelSources) {
  for control in controls.iter_mut().filter(|c| c.category.as_deref() == Some("model")) {
    for option in &mut control.options {
      match agent {
        "kimi" => {
          let Some(slash) = option.id.find('/') else { continue };
          if slash < 1 {
            continue;
          }
          let provider = option.id[..slash].to_owned();
          let kind = if provider == "kimi-code" { SourceKind::Official } else { SourceKind::Custom };
          option.source = Some(OptionSource { id: provider.clone(), name: provider, kind });
        }
        "grok" => {
          if let Some(source) = configured.get(&option.id) {
            option.source = Some(source.clone());
          } else if grok_builtin(&option.id) {
            option.source = Some(OptionSource { id: "grok".into(), name: "Grok".into(), kind: SourceKind::Official });
          }
        }
        "pi" | "opencode" => provider_prefixed(option, configured),
        "codex" | "claude" => {
          if let Some(source) = configured.get(ALL) {
            option.source = Some(source.clone());
          }
        }
        _ => {}
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn model(options: &[(&str, &str)]) -> Vec<ConfigControl> {
    vec![ConfigControl {
      id: "model".into(),
      name: "Model".into(),
      category: Some("model".into()),
      options: options.iter().map(|(id, name)| SessionOption { id: (*id).into(), name: (*name).into(), ..Default::default() }).collect(),
      ..Default::default()
    }]
  }

  fn custom(id: &str, name: &str) -> OptionSource {
    OptionSource { id: id.into(), name: name.into(), kind: SourceKind::Custom }
  }

  fn seen(controls: &[ConfigControl]) -> Vec<(String, Option<(String, SourceKind)>)> {
    controls[0].options.iter().map(|o| (o.name.clone(), o.source.as_ref().map(|s| (s.name.clone(), s.kind)))).collect()
  }

  #[test]
  fn provider_labels_move_from_names_into_sources_once() {
    let configured = ModelSources::from([("asgard".into(), custom("asgard", "asgard"))]);
    let mut c =
      model(&[("asgard/kimi-k3", "asgard/Kimi K3"), ("anthropic/claude-opus-5", "anthropic/Claude Opus 5"), ("default", "Default")]);
    apply_model_sources("pi", &mut c, &configured);
    let first = seen(&c);
    assert_eq!(
      first,
      [
        ("Kimi K3".into(), Some(("asgard".into(), SourceKind::Custom))),
        ("Claude Opus 5".into(), Some(("anthropic".into(), SourceKind::Official))),
        ("Default".into(), None),
      ]
    );
    apply_model_sources("pi", &mut c, &configured);
    assert_eq!(seen(&c), first);
    // OpenCode names carry the provider's display name
    let mut c = model(&[("asgard/claude-opus-5.5", "Asgard/Claude Opus 5.5"), ("opencode/big-pickle", "OpenCode Zen/Big Pickle")]);
    apply_model_sources("opencode", &mut c, &configured);
    assert_eq!(
      seen(&c),
      [
        ("Claude Opus 5.5".into(), Some(("Asgard".into(), SourceKind::Custom))),
        ("Big Pickle".into(), Some(("OpenCode Zen".into(), SourceKind::Official))),
      ]
    );
  }

  #[test]
  fn a_single_endpoint_sources_every_model() {
    let mut c = model(&[("opus", "claude-opus-5.5"), ("default", "Default (recommended)")]);
    apply_model_sources("claude", &mut c, &ModelSources::new());
    assert!(c[0].options.iter().all(|o| o.source.is_none()));
    apply_model_sources("claude", &mut c, &ModelSources::from([(ALL.into(), custom("gw", "gw.example"))]));
    assert!(c[0].options.iter().all(|o| o.source.as_ref().is_some_and(|s| s.kind == SourceKind::Custom)));
  }
}
