//! Agent-specific model provenance (mirror of src/shared/modelSources.ts)

use std::collections::BTreeMap;

use crate::transcript::{ConfigControl, OptionSource, SourceKind};

pub type ModelSources = BTreeMap<String, OptionSource>;

fn grok_builtin(id: &str) -> bool {
  id == "grok-build" || id.strip_prefix("grok-").is_some_and(|r| r.starts_with(|c: char| c.is_ascii_digit()))
}

pub fn apply_model_sources(agent: &str, controls: &mut [ConfigControl], configured: &ModelSources) {
  for control in controls.iter_mut().filter(|c| c.category.as_deref() == Some("model")) {
    for option in &mut control.options {
      if agent == "kimi" {
        let Some(slash) = option.id.find('/') else { continue };
        if slash < 1 {
          continue;
        }
        let provider = option.id[..slash].to_owned();
        let kind = if provider == "kimi-code" { SourceKind::Official } else { SourceKind::Custom };
        option.source = Some(OptionSource { id: provider.clone(), name: provider, kind });
      } else if agent == "grok" {
        if let Some(source) = configured.get(&option.id) {
          option.source = Some(source.clone());
        } else if grok_builtin(&option.id) {
          option.source = Some(OptionSource { id: "grok".into(), name: "Grok".into(), kind: SourceKind::Official });
        }
      }
    }
  }
}
