//! Fast & Ultra Fast speed tiers: the same model served faster at a premium price, offered in the session's speed
//! control where the model has them. Each wire format has its own switch:
//! - Anthropic Messages: `speed: "fast"` in the body plus the `fast-mode-2026-02-01` beta header. Claude Opus 5, 5.5 and
//!   4.8 on Anthropic's own API only (Opus 4.7 answers it with an error); `usage.speed` names the speed served.
//! - OpenAI Chat and Responses: `service_tier: "priority"` for Fast (OpenAI renamed priority processing to Fast mode on
//!   2026-07-30 and takes `"fast"` too, but `"priority"` is the spelling every model generation and the gateways in front
//!   of them accept) and `service_tier: "ultrafast"` for Ultra Fast. The response's `service_tier` names the tier served:
//!   `"default"` when a ramp-rate limit downgraded the call to standard speed (billed at standard rates).
//!
//! Which models have each tier is decided here from the format and the model id; `ProviderModel::fast` /
//! `ProviderModel::ultrafast` override that for a gateway model this list does not know or one whose route drops the
//! parameter, and `ProviderModel::fast_listed` / `ProviderModel::ultrafast_listed` (set from the endpoint's `/models`
//! `service_tiers` listing) stand in for the id list while the setting is absent.

use acpira_shared::providers::{ApiFormat, Provider, ProviderModel};
use serde_json::Value;

/// Speed tier selected for a session or sent on one model call
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SpeedTier {
  #[default]
  Standard,
  Fast,
  Ultrafast,
}

impl SpeedTier {
  pub fn id(self) -> &'static str {
    match self {
      SpeedTier::Standard => "standard",
      SpeedTier::Fast => "fast",
      SpeedTier::Ultrafast => "ultrafast",
    }
  }

  pub fn is_fast(self) -> bool {
    !matches!(self, SpeedTier::Standard)
  }

  /// Parse a wire value from ACP `set_config_option` or a stored preference
  pub fn parse(raw: &str) -> Option<Self> {
    match raw.trim().to_ascii_lowercase().as_str() {
      "standard" | "default" | "off" | "false" => Some(SpeedTier::Standard),
      "fast" | "priority" | "on" | "true" => Some(SpeedTier::Fast),
      "ultrafast" | "ultra_fast" | "ultra-fast" => Some(SpeedTier::Ultrafast),
      _ => None,
    }
  }

  /// Parse a JSON value from ACP `set_config_option` (boolean or string)
  pub fn from_wire(raw: &Value) -> Option<Self> {
    match raw {
      Value::Bool(true) => Some(SpeedTier::Fast),
      Value::Bool(false) => Some(SpeedTier::Standard),
      Value::String(s) => Self::parse(s),
      _ => None,
    }
  }

  /// Read from a session log's `state` event; accepts legacy boolean values (`true` / `false`) as well as `"ultrafast"`
  pub fn from_state(v: Option<&Value>) -> Self {
    v.and_then(Self::from_wire).unwrap_or(SpeedTier::Standard)
  }

  /// Write into a session log's `state` / `view` event; keeps `false` / `true` for Standard / Fast so older logs and
  /// readers stay compatible, and writes `"ultrafast"` for Ultra Fast
  pub fn to_state(self) -> Value {
    match self {
      SpeedTier::Standard => Value::Bool(false),
      SpeedTier::Fast => Value::Bool(true),
      SpeedTier::Ultrafast => Value::String("ultrafast".into()),
    }
  }
}

/// Which premium speed tiers a model offers
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Offered {
  pub fast: bool,
  pub ultrafast: bool,
}

impl Offered {
  pub fn any(self) -> bool {
    self.fast || self.ultrafast
  }

  pub fn allows(self, tier: SpeedTier) -> bool {
    match tier {
      SpeedTier::Standard => true,
      SpeedTier::Fast => self.fast,
      SpeedTier::Ultrafast => self.ultrafast,
    }
  }

  /// Clamp a remembered session tier to what this model actually offers
  pub fn effective(self, tier: SpeedTier) -> SpeedTier {
    match tier {
      SpeedTier::Ultrafast if self.ultrafast => SpeedTier::Ultrafast,
      SpeedTier::Ultrafast | SpeedTier::Fast if self.fast => SpeedTier::Fast,
      _ => SpeedTier::Standard,
    }
  }
}

/// The beta an Anthropic Messages request names when it asks for fast mode
pub const ANTHROPIC_BETA: &str = "fast-mode-2026-02-01";

/// The value of OpenAI's `service_tier` that asks for Fast mode
pub const OPENAI_TIER: &str = "priority";

/// The value of OpenAI's `service_tier` that asks for Ultra Fast mode
pub const OPENAI_ULTRAFAST_TIER: &str = "ultrafast";

/// Fast mode lists at twice the standard rate where the vendors publish it (Claude Opus 5 and 5.5, gpt-5.6-sol); other
/// models may cost up to 2.5 times, so a fast call's cost record stays an estimate
pub const PRICE_FACTOR: f64 = 2.0;

/// Ultra Fast lists at roughly six times the standard rate (estimate)
pub const ULTRAFAST_PRICE_FACTOR: f64 = 6.0;

/// Estimated cost multiplier for the speed tier that was actually served
pub fn price_factor(tier: SpeedTier) -> f64 {
  match tier {
    SpeedTier::Standard => 1.0,
    SpeedTier::Fast => PRICE_FACTOR,
    SpeedTier::Ultrafast => ULTRAFAST_PRICE_FACTOR,
  }
}

/// Claude models with fast mode, matched as id prefixes (dots read as dashes: gateways spell `claude-opus-5.5`)
const CLAUDE: &[&str] = &["claude-opus-5", "claude-opus-4-8"];

/// OpenAI families with priority processing / Fast mode, matched as id prefixes
const GPT: &[&str] = &["gpt-6", "gpt-5", "gpt-4.1", "gpt-4o"];

/// OpenAI models with the Ultra Fast tier (`service_tier: "ultrafast"`), matched as id prefixes
const GPT_ULTRAFAST: &[&str] = &["gpt-6-astra", "gpt-6.1-sol", "gpt-6.2"];

/// OpenAI reasoning models with it, matched whole or with a date suffix (`o3-2025-04-16`), so `o3-mini` stays out
const O_SERIES: &[&str] = &["o3", "o4-mini"];

/// Variants of those families that are not chat models
const MODALITIES: &[&str] = &["audio", "realtime", "search", "transcribe", "tts", "image"];

/// Which speed tiers the model's session offers: explicit model overrides first, else the endpoint's `service_tiers`
/// listing, else the known id list for its format. Explicit `fast: Some(false)` turns speed off unless `ultrafast` is
/// explicitly forced on.
pub fn offered_tiers(provider: &Provider, model: &ProviderModel) -> Offered {
  let format = provider.api_format();
  let openai = matches!(format, Some(ApiFormat::OpenaiChat | ApiFormat::OpenaiResponses));
  let fast = model.fast.unwrap_or_else(|| {
    if openai && let Some(listed) = model.fast_listed {
      return listed;
    }
    known(format, &model.id)
  });
  let ultrafast = if model.fast == Some(false) && model.ultrafast != Some(true) {
    false
  } else {
    model.ultrafast.unwrap_or_else(|| {
      if openai && let Some(listed) = model.ultrafast_listed {
        return listed;
      }
      known_ultrafast(format, &model.id)
    })
  };
  Offered { fast, ultrafast }
}

/// Whether the model's session offers any speed tier
pub fn offered(provider: &Provider, model: &ProviderModel) -> bool {
  offered_tiers(provider, model).any()
}

/// The known Fast list: the id without a gateway prefix (`openai/gpt-5.5`), lowercase
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

/// The known Ultra Fast list: OpenAI Chat / Responses only, id without a gateway prefix
fn known_ultrafast(format: Option<ApiFormat>, id: &str) -> bool {
  if !matches!(format, Some(ApiFormat::OpenaiChat | ApiFormat::OpenaiResponses)) {
    return false;
  }
  let id = id.to_lowercase();
  let bare = id.rsplit('/').next().unwrap_or(&id);
  GPT_ULTRAFAST.iter().any(|p| bare.starts_with(p)) && !MODALITIES.iter().any(|x| bare.contains(x))
}

/// The speed tier the provider reports it actually served (None when it reported nothing: trust the request)
pub fn served_tier(served: Option<&str>) -> Option<SpeedTier> {
  served.map(|s| match s {
    "ultrafast" => SpeedTier::Ultrafast,
    "fast" | "priority" => SpeedTier::Fast,
    _ => SpeedTier::Standard,
  })
}

/// Whether the provider's report of the speed it served says Fast or Ultra Fast
pub fn served_fast(served: Option<&str>) -> Option<bool> {
  served_tier(served).map(SpeedTier::is_fast)
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  fn offered_in(format: &str, id: &str) -> Offered {
    let provider: Provider = serde_json::from_value(json!({ "id": "p", "format": format, "baseUrl": "https://gw.example.com/v1" })).unwrap();
    offered_tiers(&provider, &ProviderModel::new(id))
  }

  fn listed_offered(format: &str, id: &str, fast: Option<bool>, fast_listed: Option<bool>, ultrafast: Option<bool>, ultrafast_listed: Option<bool>) -> Offered {
    let provider: Provider = serde_json::from_value(json!({ "id": "p", "format": format, "baseUrl": "https://gw.example.com/v1" })).unwrap();
    let mut m = ProviderModel::new(id);
    m.fast = fast;
    m.fast_listed = fast_listed;
    m.ultrafast = ultrafast;
    m.ultrafast_listed = ultrafast_listed;
    offered_tiers(&provider, &m)
  }

  #[test]
  fn claude_opus_5_and_4_8_on_anthropic_messages() {
    for id in ["claude-opus-5-5", "claude-opus-5", "claude-opus-4-8", "claude-opus-5.5", "anthropic/claude-opus-5-5", "Claude-Opus-5-5"] {
      assert_eq!(offered_in("anthropic", id), Offered { fast: true, ultrafast: false }, "{id}");
    }
    for id in ["claude-opus-4-7", "claude-sonnet-5-5", "claude-fable-5-1", "claude-haiku-4-5"] {
      assert!(!offered_in("anthropic", id).any(), "{id}");
    }
    // The body field is Anthropic's: the same model behind an OpenAI-format gateway is not assumed to take it
    assert!(!offered_in("openai-chat", "claude-opus-5-5").any());
  }

  #[test]
  fn gpt_families_and_o_series_on_openai_formats() {
    for id in ["gpt-6.1-sol", "gpt-6-astra", "gpt-6.2-astra", "gpt-6.2-sol", "openai/gpt-6.2-astra"] {
      assert_eq!(offered_in("openai-chat", id), Offered { fast: true, ultrafast: true }, "{id}");
      assert_eq!(offered_in("openai-responses", id), Offered { fast: true, ultrafast: true }, "{id}");
    }
    for id in ["gpt-6-luna", "gpt-5.6-sol", "gpt-5.4-mini", "gpt-5", "gpt-4.1-nano", "gpt-4o-mini", "o3", "o3-2025-04-16", "o4-mini", "openai/gpt-5.5"] {
      assert_eq!(offered_in("openai-chat", id), Offered { fast: true, ultrafast: false }, "{id}");
      assert_eq!(offered_in("openai-responses", id), Offered { fast: true, ultrafast: false }, "{id}");
    }
    for id in ["gpt-5-nano", "gpt-4o-audio-preview", "gpt-4o-realtime-preview", "o3-mini", "o1", "gpt-oss-120b", "deepseek-v4-pro", "swe-2"] {
      assert!(!offered_in("openai-chat", id).any(), "{id}");
    }
    assert!(!offered_in("anthropic", "gpt-6.1-sol").any());
  }

  #[test]
  fn the_model_setting_overrides_the_list() {
    let provider: Provider = serde_json::from_value(json!({ "id": "p", "format": "openai-chat" })).unwrap();
    let mut m = ProviderModel::new("my-gateway-alias");
    assert!(!offered(&provider, &m));
    m.fast = Some(true);
    assert_eq!(offered_tiers(&provider, &m), Offered { fast: true, ultrafast: false });
    m.ultrafast = Some(true);
    assert_eq!(offered_tiers(&provider, &m), Offered { fast: true, ultrafast: true });
    let mut gpt = ProviderModel::new("gpt-6.1-sol");
    gpt.fast = Some(false);
    assert!(!offered(&provider, &gpt));
    gpt.fast = Some(true);
    gpt.ultrafast = Some(false);
    assert_eq!(offered_tiers(&provider, &gpt), Offered { fast: true, ultrafast: false });
  }

  #[test]
  fn a_listing_flag_opens_and_closes_fast_and_ultrafast_on_openai_formats() {
    for format in ["openai-chat", "openai-responses"] {
      assert_eq!(listed_offered(format, "grok-4.7", None, Some(true), None, Some(false)), Offered { fast: true, ultrafast: false }, "{format}");
      assert_eq!(listed_offered(format, "grok-4.7", None, Some(true), None, Some(true)), Offered { fast: true, ultrafast: true }, "{format}");
      assert_eq!(listed_offered(format, "gpt-6-astra", None, Some(false), None, Some(false)), Offered::default(), "{format}");
      assert_eq!(listed_offered(format, "gpt-6.1-sol", None, Some(true), None, Some(false)), Offered { fast: true, ultrafast: false }, "{format}");
      // Without the flags the known list still decides
      assert_eq!(listed_offered(format, "gpt-6.1-sol", None, None, None, None), Offered { fast: true, ultrafast: true }, "{format}");
      assert_eq!(listed_offered(format, "grok-4.7", None, None, None, None), Offered::default(), "{format}");
    }
  }

  #[test]
  fn the_listing_flag_does_not_reach_another_format() {
    assert_eq!(listed_offered("anthropic", "claude-opus-5.5", None, Some(false), None, Some(true)), Offered { fast: true, ultrafast: false });
    assert_eq!(listed_offered("anthropic", "gpt-6.1-sol", None, Some(true), None, Some(true)), Offered::default());
  }

  #[test]
  fn an_explicit_setting_wins_over_the_listing_flag() {
    for format in ["openai-chat", "openai-responses", "anthropic"] {
      assert_eq!(listed_offered(format, "gpt-6-astra", Some(true), Some(false), Some(false), Some(true)), Offered { fast: true, ultrafast: false }, "{format}");
      assert_eq!(listed_offered(format, "gpt-6.1-sol", Some(false), Some(true), None, Some(true)), Offered::default(), "{format}");
      assert_eq!(listed_offered(format, "claude-opus-5.5", Some(true), Some(false), None, None), Offered { fast: true, ultrafast: false }, "{format}");
    }
  }

  #[test]
  fn served_tier_classifies_ultrafast_fast_and_downgraded() {
    assert_eq!(served_tier(Some("ultrafast")), Some(SpeedTier::Ultrafast));
    assert_eq!(served_tier(Some("priority")), Some(SpeedTier::Fast));
    assert_eq!(served_tier(Some("fast")), Some(SpeedTier::Fast));
    assert_eq!(served_tier(Some("default")), Some(SpeedTier::Standard));
    assert_eq!(served_tier(Some("standard")), Some(SpeedTier::Standard));
    assert_eq!(served_tier(None), None);
    assert_eq!(served_fast(Some("ultrafast")), Some(true));
    assert_eq!(served_fast(Some("default")), Some(false));
  }
}
