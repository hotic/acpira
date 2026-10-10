//! Per-family request differences as data: how thinking is switched, whether earlier reasoning goes back into the
//! history, and how the provider caches prompts. A model matches by its pinned `family`, else by the first rule whose
//! pattern its id (or its source's preset / host) contains. Adding a family is a table row, not a code path.
//! Wire facts here are from the providers' public docs as of 2026-10; the ones not yet seen on a real wire are marked

use serde_json::{Map, Value, json};

use acpira_shared::providers::{Provider, ProviderModel, Thinking};

/// How a request turns thinking on / off and picks a level
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinkingParam {
  /// No switch; the model id decides (Kimi's -thinking models)
  None,
  /// `thinking: { type: "enabled" | "disabled" }` (GLM, DeepSeek V3.2)
  ThinkingType,
  /// `enable_thinking: bool` (Qwen on DashScope)
  EnableThinking,
  /// `reasoning_effort: <level>` (OpenAI-style servers)
  ReasoningEffort,
  /// `reasoning: { effort } | { enabled }` (OpenRouter)
  OpenRouter,
}

/// Which earlier reasoning is sent back with assistant messages
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReasoningEcho {
  Never,
  /// Only the assistant messages after the latest user message, as `reasoning_content` (tool rounds of the running turn:
  /// DeepSeek V3.2 requires them there and drops them across user turns)
  CurrentTurn,
  /// Every assistant message keeps `reasoning_content` (Kimi's thinking models)
  Always,
  /// The reasoning goes back inside the content as `<think>…</think>` (MiniMax M2's interleaved thinking; unverified)
  ThinkTags,
}

/// How the provider caches prompts
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheKind {
  /// Automatic on a repeated prefix
  ImplicitPrefix,
  /// Only where the request marks a breakpoint (Anthropic `cache_control`)
  Breakpoints,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Family {
  pub name: &'static str,
  pub thinking: ThinkingParam,
  pub echo: ReasoningEcho,
  pub cache: CacheKind,
  /// How long a cached prefix lives, in seconds, when the provider says
  pub cache_ttl: Option<u32>,
}

pub const GENERIC: Family =
  Family { name: "generic", thinking: ThinkingParam::ReasoningEffort, echo: ReasoningEcho::Never, cache: CacheKind::ImplicitPrefix, cache_ttl: None };

/// (patterns matched against the lowercase model id, then the preset / base URL; family)
const RULES: &[(&[&str], Family)] = &[
  (
    &["deepseek"],
    Family { name: "deepseek", thinking: ThinkingParam::ThinkingType, echo: ReasoningEcho::CurrentTurn, cache: CacheKind::ImplicitPrefix, cache_ttl: None },
  ),
  (
    &["kimi", "moonshot"],
    Family { name: "kimi", thinking: ThinkingParam::None, echo: ReasoningEcho::Always, cache: CacheKind::ImplicitPrefix, cache_ttl: None },
  ),
  (
    &["glm", "zhipu", "bigmodel", "z.ai"],
    Family { name: "glm", thinking: ThinkingParam::ThinkingType, echo: ReasoningEcho::CurrentTurn, cache: CacheKind::ImplicitPrefix, cache_ttl: None },
  ),
  (
    &["qwen", "qwq", "dashscope"],
    Family { name: "qwen", thinking: ThinkingParam::EnableThinking, echo: ReasoningEcho::Never, cache: CacheKind::ImplicitPrefix, cache_ttl: None },
  ),
  (
    &["minimax"],
    Family { name: "minimax", thinking: ThinkingParam::None, echo: ReasoningEcho::ThinkTags, cache: CacheKind::ImplicitPrefix, cache_ttl: None },
  ),
];

/// OpenRouter's request shape wins over the model's own family for the thinking switch (it translates per upstream)
const OPENROUTER: &str = "openrouter";

pub fn by_name(name: &str) -> Option<Family> {
  if name == GENERIC.name {
    return Some(GENERIC);
  }
  RULES.iter().map(|(_, f)| *f).find(|f| f.name == name)
}

pub fn resolve(provider: &Provider, model: &ProviderModel) -> Family {
  let mut family = model.family.as_deref().and_then(by_name).unwrap_or_else(|| {
    let id = model.id.to_lowercase();
    let source = format!("{} {}", provider.preset.to_lowercase(), provider.base_url.to_lowercase());
    RULES
      .iter()
      .find(|(pats, _)| pats.iter().any(|p| id.contains(p)))
      .or_else(|| RULES.iter().find(|(pats, _)| pats.iter().any(|p| source.contains(p))))
      .map(|(_, f)| *f)
      .unwrap_or(GENERIC)
  });
  if provider.preset == OPENROUTER || provider.base_url.contains("openrouter.ai") {
    family.thinking = ThinkingParam::OpenRouter;
    // OpenRouter normalizes reasoning into its own field; sending reasoning_content back is not part of its API
    family.echo = ReasoningEcho::Never;
  }
  family
}

/// Add the thinking switch and level to a request body. `Auto` without a level leaves the provider's default
pub fn apply_thinking(body: &mut Map<String, Value>, family: &Family, thinking: Thinking, effort: Option<&str>) {
  let on = match thinking {
    Thinking::On => Some(true),
    Thinking::Off => Some(false),
    Thinking::Auto => None,
  };
  match family.thinking {
    ThinkingParam::None => {}
    ThinkingParam::ThinkingType => {
      if let Some(on) = on {
        body.insert("thinking".into(), json!({ "type": if on { "enabled" } else { "disabled" } }));
      }
    }
    ThinkingParam::EnableThinking => {
      if let Some(on) = on {
        body.insert("enable_thinking".into(), Value::Bool(on));
      }
    }
    ThinkingParam::ReasoningEffort => {
      if on != Some(false)
        && let Some(e) = effort
      {
        body.insert("reasoning_effort".into(), Value::from(e));
      }
    }
    ThinkingParam::OpenRouter => match (on, effort) {
      (Some(false), _) => {
        body.insert("reasoning".into(), json!({ "enabled": false }));
      }
      (_, Some(e)) => {
        body.insert("reasoning".into(), json!({ "effort": e }));
      }
      (Some(true), None) => {
        body.insert("reasoning".into(), json!({ "enabled": true }));
      }
      (None, None) => {}
    },
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn provider(preset: &str, base: &str) -> Provider {
    serde_json::from_value(json!({ "id": "p", "preset": preset, "baseUrl": base })).unwrap()
  }

  #[test]
  fn families_match_by_model_id_then_source_and_a_pin_wins() {
    let any = provider("custom", "https://gw.example.com/v1");
    assert_eq!(resolve(&any, &ProviderModel::new("DeepSeek-V3.2")).name, "deepseek");
    assert_eq!(resolve(&any, &ProviderModel::new("glm-5.2")).name, "glm");
    assert_eq!(resolve(&any, &ProviderModel::new("my-model")).name, "generic");
    let dash = provider("custom", "https://dashscope.aliyuncs.com/compatible-mode/v1");
    assert_eq!(resolve(&dash, &ProviderModel::new("my-model")).name, "qwen");
    let mut pinned = ProviderModel::new("deepseek-chat");
    pinned.family = Some("kimi".into());
    assert_eq!(resolve(&any, &pinned).name, "kimi");
    let or = resolve(&provider("openrouter", "https://openrouter.ai/api/v1"), &ProviderModel::new("deepseek/deepseek-chat"));
    assert_eq!((or.name, or.thinking, or.echo), ("deepseek", ThinkingParam::OpenRouter, ReasoningEcho::Never));
  }

  #[test]
  fn thinking_switches_per_family() {
    let mut b = Map::new();
    apply_thinking(&mut b, &by_name("glm").unwrap(), Thinking::Off, None);
    assert_eq!(b["thinking"], json!({ "type": "disabled" }));
    let mut b = Map::new();
    apply_thinking(&mut b, &by_name("qwen").unwrap(), Thinking::Auto, Some("high"));
    assert!(b.is_empty(), "auto without a family level leaves the default");
    let mut b = Map::new();
    apply_thinking(&mut b, &GENERIC, Thinking::Auto, Some("low"));
    assert_eq!(b["reasoning_effort"], "low");
  }
}
