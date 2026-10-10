//! Known services a new source can start from. The ids double as family hints (`family.rs` matches a source's preset),
//! and the URLs are the services' documented OpenAI-compatible roots as of 2026-10 (not each checked on a live wire).
//! The settings page gets this table with the providers view, so there is no second copy in TypeScript

use acpira_shared::providers::Preset;

/// (id, name, format, base URL, key page, local)
type Row = (&'static str, &'static str, &'static str, &'static str, Option<&'static str>, bool);

const TABLE: &[Row] = &[
  ("deepseek", "DeepSeek", "openai-chat", "https://api.deepseek.com/v1", Some("https://platform.deepseek.com/api_keys"), false),
  ("moonshot", "Kimi (Moonshot)", "openai-chat", "https://api.moonshot.cn/v1", Some("https://platform.moonshot.cn/console/api-keys"), false),
  ("zhipu", "智谱 GLM", "openai-chat", "https://open.bigmodel.cn/api/paas/v4", Some("https://open.bigmodel.cn/usercenter/apikeys"), false),
  ("z.ai", "Z.ai", "openai-chat", "https://api.z.ai/api/paas/v4", Some("https://z.ai/manage-apikey/apikey-list"), false),
  ("dashscope", "通义百炼 (DashScope)", "openai-chat", "https://dashscope.aliyuncs.com/compatible-mode/v1", Some("https://bailian.console.aliyun.com/?apiKey=1"), false),
  ("minimax", "MiniMax", "openai-chat", "https://api.minimaxi.com/v1", Some("https://platform.minimaxi.com/user-center/basic-information/interface-key"), false),
  ("openrouter", "OpenRouter", "openai-chat", "https://openrouter.ai/api/v1", Some("https://openrouter.ai/settings/keys"), false),
  ("siliconflow", "SiliconFlow", "openai-chat", "https://api.siliconflow.cn/v1", Some("https://cloud.siliconflow.cn/account/ak"), false),
  ("ollama", "Ollama", "openai-chat", "http://127.0.0.1:11434/v1", None, true),
  ("lmstudio", "LM Studio", "openai-chat", "http://127.0.0.1:1234/v1", None, true),
  ("custom", "Custom", "openai-chat", "", None, false),
];

pub fn presets() -> Vec<Preset> {
  TABLE
    .iter()
    .map(|&(id, name, format, base_url, key_url, local)| Preset {
      id: id.into(),
      name: name.into(),
      format: format.into(),
      base_url: base_url.into(),
      key_url: key_url.map(str::to_owned),
      local,
    })
    .collect()
}

#[cfg(test)]
mod tests {
  use super::*;
  use acpira_shared::providers::{Provider, ProviderModel};

  #[test]
  fn each_vendor_preset_selects_its_family() {
    for p in presets().iter().filter(|p| ["deepseek", "moonshot", "zhipu", "z.ai", "dashscope", "minimax"].contains(&p.id.as_str())) {
      let provider: Provider = serde_json::from_value(serde_json::json!({ "id": "x", "preset": p.id, "baseUrl": p.base_url })).unwrap();
      let family = crate::llm::family::resolve(&provider, &ProviderModel::new("some-model"));
      assert_ne!(family.name, crate::llm::family::GENERIC.name, "{} matches no family", p.id);
    }
    assert!(presets().iter().all(|p| p.local == p.key_url.is_none() || p.id == "custom"));
  }
}
