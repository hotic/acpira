//! Fast mode: the same model served faster at a premium price, offered as the session's Fast switch where the model
//! has it. Each wire format has its own switch (vendor docs read 2026-10-13; not yet seen on a real wire):
//! - Anthropic Messages: `speed: "fast"` in the body plus the `fast-mode-2026-02-01` beta header. Claude Opus 5, 5.5 and
//!   4.8 on Anthropic's own API only (Opus 4.7 answers it with an error); `usage.speed` names the speed served.
//! - OpenAI Chat and Responses: `service_tier: "priority"`. OpenAI renamed priority processing to Fast mode on
//!   2026-07-30 and takes `"fast"` too, but `"priority"` is the spelling every model generation and the gateways in front
//!   of them accept. The response's `service_tier` names the tier served: `"default"` when a ramp-rate limit downgraded
//!   the call to standard speed (billed at standard rates).
//!
//! Which models have it is decided here from the format and the model id; `ProviderModel::fast` overrides that for a
//! gateway model this list does not know or one whose route drops the parameter.

use acpira_shared::providers::{ApiFormat, Provider, ProviderModel};

/// The beta an Anthropic Messages request names when it asks for fast mode
pub const ANTHROPIC_BETA: &str = "fast-mode-2026-02-01";

/// The value of OpenAI's `service_tier` that asks for Fast mode
pub const OPENAI_TIER: &str = "priority";

/// Fast mode lists at twice the standard rate where the vendors publish it (Claude Opus 5 and 5.5, gpt-5.6-sol); other
/// models may cost up to 2.5 times, so a fast call's cost record stays an estimate
pub const PRICE_FACTOR: f64 = 2.0;

/// Claude models with fast mode, matched as id prefixes (dots read as dashes: gateways spell `claude-opus-5.5`)
const CLAUDE: &[&str] = &["claude-opus-5", "claude-opus-4-8"];

/// OpenAI families with priority processing / Fast mode, matched as id prefixes
const GPT: &[&str] = &["gpt-6", "gpt-5", "gpt-4.1", "gpt-4o"];

/// OpenAI reasoning models with it, matched whole or with a date suffix (`o3-2025-04-16`), so `o3-mini` stays out
const O_SERIES: &[&str] = &["o3", "o4-mini"];

/// Variants of those families that are not chat models
const MODALITIES: &[&str] = &["audio", "realtime", "search", "transcribe", "tts", "image"];

/// Whether the model's session offers the Fast switch: the model's own setting, else the known list for its format
pub fn offered(provider: &Provider, model: &ProviderModel) -> bool {
  model.fast.unwrap_or_else(|| known(provider.api_format(), &model.id))
}

/// The known list: the id without a gateway prefix (`openai/gpt-5.5`), lowercase
fn known(format: Option<ApiFormat>, id: &str) -> bool {
  let id = id.to_lowercase();
  let bare = id.rsplit('/').next().unwrap_or(&id);
  match format {
    Some(ApiFormat::Anthropic) => {
      let dashed = bare.replace('.', "-");
      CLAUDE.iter().any(|p| dashed.starts_with(p))
    }
    Some(ApiFormat::OpenaiChat | ApiFormat::OpenaiResponses) => {
      let gpt = GPT.iter().any(|p| bare.starts_with(p))
        && !MODALITIES.iter().any(|x| bare.contains(x))
        // gpt-4.1-nano has the tier; the gpt-5 nanos are not listed for it
        && !(bare.starts_with("gpt-5") && bare.contains("nano"));
      let o = O_SERIES.iter().any(|p| bare == *p || bare.strip_prefix(p).is_some_and(|rest| rest.starts_with("-20")));
      gpt || o
    }
    None => false,
  }
}

/// Whether the provider's report of the speed it served says Fast (None when it reported nothing: trust the request)
pub fn served_fast(served: Option<&str>) -> Option<bool> {
  served.map(|s| matches!(s, "fast" | "priority"))
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn offered_in(format: &str, id: &str) -> bool {
    let provider: Provider = serde_json::from_value(json!({ "id": "p", "format": format, "baseUrl": "https://gw.example.com/v1" })).unwrap();
    offered(&provider, &ProviderModel::new(id))
  }

  #[test]
  fn claude_opus_5_and_4_8_on_anthropic_messages() {
    for id in ["claude-opus-5-5", "claude-opus-5", "claude-opus-4-8", "claude-opus-5.5", "anthropic/claude-opus-5-5", "Claude-Opus-5-5"] {
      assert!(offered_in("anthropic", id), "{id}");
    }
    for id in ["claude-opus-4-7", "claude-sonnet-5-5", "claude-fable-5-1", "claude-haiku-4-5"] {
      assert!(!offered_in("anthropic", id), "{id}");
    }
    // The body field is Anthropic's: the same model behind an OpenAI-format gateway is not assumed to take it
    assert!(!offered_in("openai-chat", "claude-opus-5-5"));
  }

  #[test]
  fn gpt_families_and_o_series_on_openai_formats() {
    for id in ["gpt-6.1-sol", "gpt-6-luna", "gpt-5.6-sol", "gpt-5.4-mini", "gpt-5", "gpt-4.1-nano", "gpt-4o-mini", "o3", "o3-2025-04-16", "o4-mini", "openai/gpt-5.5"] {
      assert!(offered_in("openai-chat", id), "{id}");
      assert!(offered_in("openai-responses", id), "{id}");
    }
    for id in ["gpt-5-nano", "gpt-4o-audio-preview", "gpt-4o-realtime-preview", "o3-mini", "o1", "gpt-oss-120b", "deepseek-v4-pro", "swe-2"] {
      assert!(!offered_in("openai-chat", id), "{id}");
    }
    assert!(!offered_in("anthropic", "gpt-6.1-sol"));
  }

  #[test]
  fn the_model_setting_overrides_the_list() {
    let provider: Provider = serde_json::from_value(json!({ "id": "p", "format": "openai-chat" })).unwrap();
    let mut m = ProviderModel::new("my-gateway-alias");
    assert!(!offered(&provider, &m));
    m.fast = Some(true);
    assert!(offered(&provider, &m));
    let mut gpt = ProviderModel::new("gpt-6.1-sol");
    gpt.fast = Some(false);
    assert!(!offered(&provider, &gpt));
  }

  #[test]
  fn a_downgraded_call_is_not_fast() {
    assert_eq!(served_fast(Some("priority")), Some(true));
    assert_eq!(served_fast(Some("fast")), Some(true));
    assert_eq!(served_fast(Some("default")), Some(false));
    assert_eq!(served_fast(Some("standard")), Some(false));
    assert_eq!(served_fast(None), None);
  }
}
