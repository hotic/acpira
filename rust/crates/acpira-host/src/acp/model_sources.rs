//! Grok model provenance from its config files: only aliases and source
//! labels leave this module, endpoint credentials stay in the CLI config

use std::path::Path;

use acpira_shared::model_sources::ModelSources;
use acpira_shared::transcript::{OptionSource, SourceKind};

use crate::store::data_dir::home_dir;

pub fn grok_model_sources(text: &str) -> ModelSources {
  let mut sources = ModelSources::new();
  let Ok(config) = text.parse::<toml::Table>() else { return sources };
  let Some(toml::Value::Table(models)) = config.get("model") else { return sources };
  for (alias, value) in models {
    let toml::Value::Table(v) = value else { continue };
    // A context-window override alone does not turn a built-in model into a custom endpoint
    if let Some(toml::Value::String(url)) = v.get("base_url")
      && !url.trim().is_empty()
    {
      sources.insert(alias.clone(), OptionSource { id: alias.clone(), name: alias.clone(), kind: SourceKind::Custom });
    }
  }
  sources
}

pub async fn read_model_sources(agent: &str, cwd: &str) -> ModelSources {
  if agent != "grok" {
    return ModelSources::new();
  }
  let mut out = ModelSources::new();
  for path in [home_dir().join(".grok/config.toml"), Path::new(cwd).join(".grok/config.toml")] {
    if let Ok(text) = tokio::fs::read_to_string(&path).await {
      out.extend(grok_model_sources(&text));
    }
  }
  out
}

#[cfg(test)]
mod tests {
  #[test]
  fn only_endpoints_count_as_custom() {
    let s = super::grok_model_sources("[model.mine]\nbase_url = \"https://x\"\n[model.grok-4]\ncontext_window = 1\n");
    assert_eq!(s.keys().collect::<Vec<_>>(), ["mine"]);
  }
}
