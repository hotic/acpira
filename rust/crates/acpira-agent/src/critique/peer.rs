//! Silent cross-family strong peer resolution for Critique (zero Composer knobs).
//! Guarantees:
//! 1. Critic model family strictly differs from the Lead model's broad family (no silent same-family fallback).
//! 2. Small/sidekick-tier models (luna, mini, flash, haiku, etc.) are never picked to critique a frontier Lead.
//! 3. Missing eligible peer degrades silently to `None` (Single).

use acpira_shared::model_catalog::Catalog;
use acpira_shared::providers::{Provider, ProviderModel, model_pick};

use crate::config::Config;

/// Broad model family used to guarantee genuine cross-family diversity between Lead and Critic
pub fn broad_family(model_id: &str, provider_preset: &str, base_url: &str) -> &'static str {
  let id = model_id.to_ascii_lowercase();
  let src = format!("{} {}", provider_preset.to_ascii_lowercase(), base_url.to_ascii_lowercase());
  let matches_any = |s: &str, pats: &[&str]| pats.iter().any(|p| s.contains(p));

  if matches_any(&id, &["claude", "opus", "sonnet", "haiku", "fable", "anthropic"]) {
    return "anthropic";
  }
  if matches_any(&id, &["gpt", "codex", "o1-", "o3-", "o4-", "astra", "sol", "luna", "terra"]) {
    return "openai";
  }
  if matches_any(&id, &["gemini", "carbon", "neon", "argon", "hydrogen"]) {
    return "gemini";
  }
  if matches_any(&id, &["deepseek"]) {
    return "deepseek";
  }
  if matches_any(&id, &["qwen", "qwq"]) {
    return "qwen";
  }
  if matches_any(&id, &["kimi", "moonshot"]) {
    return "kimi";
  }
  if matches_any(&id, &["glm", "zhipu"]) {
    return "glm";
  }
  if matches_any(&id, &["grok", "xai"]) {
    return "xai";
  }
  if matches_any(&src, &["anthropic"]) {
    return "anthropic";
  }
  if matches_any(&src, &["openai"]) {
    return "openai";
  }
  if matches_any(&src, &["google", "gemini", "antigravity"]) {
    return "gemini";
  }
  "other"
}

/// Small/sidekick-tier models that are great executors/sidekicks but must not critique a frontier Lead
pub fn is_small_model(model_id: &str) -> bool {
  let id = model_id.to_ascii_lowercase();
  let small_tokens = ["luna", "mini", "nano", "flash", "haiku", "swe-2", "lite", "small", "tiny", "micro"];
  small_tokens.iter().any(|t| id.contains(t))
}

/// Heuristic capability rank for choosing the strongest available cross-family peer
fn capability_score(model_id: &str, catalog: &Catalog) -> u64 {
  let id = model_id.to_ascii_lowercase();
  let mut score: u64 = 100;
  if id.contains("opus") || id.contains("fable") || id.contains("astra") || id.contains("carbon") || id.contains("pro") {
    score += 500;
  } else if id.contains("sonnet") || id.contains("neon") || id.contains("sol") || id.contains("max") {
    score += 300;
  }
  // Tie-break with catalogue input pricing (millicents per M tokens) when available
  if let Some(cost) = catalog.lookup(model_id, None).and_then(|e| e.cost.as_ref()) {
    score += (cost.input * 10.0) as u64;
  }
  score
}

/// Whether implicit zero-tool Critique is enabled (kill switch: `ACPIRA_CRITIC_ENABLED=0` disables it)
pub fn enabled() -> bool {
  !matches!(std::env::var("ACPIRA_CRITIC_ENABLED").ok().as_deref().map(str::trim), Some("0" | "false" | "off"))
}

/// Resolve a verified cross-family strong Critic model from `config` for an arbitrary `lead_family`
/// (usable both by the built-in agent and by `acpira-host` when an external CLI like Claude Code or Codex leads).
/// Never falls back to a same-family model or a small/sidekick-tier model; returns `None` if none qualifies.
pub fn resolve_peer_for_family<'a>(
  config: &'a Config,
  catalog: &Catalog,
  lead_family: &str,
  exclude_picks: &[&str],
  preferred: Option<&str>,
) -> Option<(&'a Provider, &'a ProviderModel, String)> {
  if !enabled() {
    return None;
  }
  let allow_same = matches!(std::env::var("ACPIRA_CRITIC_ALLOW_SAME_FAMILY").ok().as_deref(), Some("1" | "true"));
  let env_pick = std::env::var("ACPIRA_CRITIC_MODEL").ok().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty());
  if let Some(explicit) = preferred.map(str::to_owned).or(env_pick) {
    let (p, m) = config.find(&explicit)?;
    config.api_key(&p.id)?;
    let fam = broad_family(&m.id, &p.preset, &p.base_url);
    if !allow_same && (fam == lead_family || fam == "other") {
      return None;
    }
    if !allow_same && is_small_model(&m.id) {
      return None;
    }
    return Some((p, m, model_pick(&p.id, &m.id)));
  }

  let mut best: Option<(u64, &'a Provider, &'a ProviderModel, String)> = None;
  for (p, m) in config.providers.usable() {
    if config.api_key(&p.id).is_none() {
      continue;
    }
    let pick = model_pick(&p.id, &m.id);
    if exclude_picks.iter().any(|x| *x == pick) {
      continue;
    }
    if is_small_model(&m.id) {
      continue;
    }
    let fam = broad_family(&m.id, &p.preset, &p.base_url);
    if fam == lead_family || fam == "other" {
      continue;
    }
    let rank = capability_score(&m.id, catalog);
    if best.as_ref().is_none_or(|(b_rank, _, _, _)| rank > *b_rank) {
      best = Some((rank, p, m, pick));
    }
  }
  best.map(|(_, p, m, pick)| (p, m, pick))
}

/// Resolve a verified cross-family strong Critic model from `config` for a built-in agent `lead_pick`.
pub fn resolve_peer<'a>(
  config: &'a Config,
  catalog: &Catalog,
  lead_pick: &str,
  sidekick_pick: Option<&str>,
  preferred: Option<&str>,
) -> Option<(&'a Provider, &'a ProviderModel, String)> {
  let (lead_provider, lead_model) = config.find(lead_pick)?;
  let lead_fam = broad_family(&lead_model.id, &lead_provider.preset, &lead_provider.base_url);
  let mut exclude = vec![lead_pick];
  if let Some(sk) = sidekick_pick {
    exclude.push(sk);
  }
  resolve_peer_for_family(config, catalog, lead_fam, &exclude, preferred)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test code")]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn resolves_cross_family_strong_peer_and_rejects_small_or_same_family() {
    let dir = tempfile::tempdir().unwrap();
    let providers = json!({
      "version": 1,
      "providers": [
        {
          "id": "gw",
          "baseUrl": "https://example.com/v1",
          "models": [
            { "id": "claude-opus-5-5" },
            { "id": "claude-sonnet-5-5" },
            { "id": "gpt-6-luna" },
            { "id": "gpt-6.2-astra" },
            { "id": "gemini-3.8-flash" }
          ]
        }
      ]
    });
    std::fs::write(dir.path().join(acpira_shared::providers::PROVIDERS_FILE), providers.to_string()).unwrap();
    std::fs::write(dir.path().join(crate::config::SECRETS_FILE), json!({ "acpira.provider.gw": "sk-test" }).to_string()).unwrap();
    let cfg = crate::config::ConfigCache::new(dir.path().to_owned()).get();
    let cat = Catalog::default();

    // Claude Opus lead -> picks GPT-6.2 Astra (skips Sonnet same-family and Luna/Flash small models)
    let (_, m, pick) = resolve_peer(&cfg, &cat, "gw/claude-opus-5-5", Some("gw/gpt-6-luna"), None).unwrap();
    assert_eq!(m.id, "gpt-6.2-astra");
    assert_eq!(pick, "gw/gpt-6.2-astra");

    // GPT-6.2 Astra lead -> picks Claude Opus (stronger than Sonnet)
    let (_, m2, _) = resolve_peer(&cfg, &cat, "gw/gpt-6.2-astra", Some("gw/gpt-6-luna"), None).unwrap();
    assert_eq!(m2.id, "claude-opus-5-5");

    // Explicit same-family override is rejected (no silent fallback)
    assert!(resolve_peer(&cfg, &cat, "gw/claude-opus-5-5", None, Some("gw/claude-sonnet-5-5")).is_none());
    // Explicit small-model override is rejected
    assert!(resolve_peer(&cfg, &cat, "gw/claude-opus-5-5", None, Some("gw/gpt-6-luna")).is_none());
  }

  #[test]
  fn returns_none_when_only_same_family_or_small_models_exist() {
    let dir = tempfile::tempdir().unwrap();
    let providers = json!({
      "version": 1,
      "providers": [
        {
          "id": "gw",
          "baseUrl": "https://example.com/v1",
          "models": [
            { "id": "claude-opus-5-5" },
            { "id": "claude-sonnet-5-5" },
            { "id": "gpt-6-luna" }
          ]
        }
      ]
    });
    std::fs::write(dir.path().join(acpira_shared::providers::PROVIDERS_FILE), providers.to_string()).unwrap();
    std::fs::write(dir.path().join(crate::config::SECRETS_FILE), json!({ "acpira.provider.gw": "sk-test" }).to_string()).unwrap();
    let cfg = crate::config::ConfigCache::new(dir.path().to_owned()).get();
    let cat = Catalog::default();

    assert!(resolve_peer(&cfg, &cat, "gw/claude-opus-5-5", Some("gw/gpt-6-luna"), None).is_none());
  }
}
