//! Zero-tool cross-family Critique runner shared by `acpira-agent` (Plan / Draft checkpoints)
//! and `acpira-host` (Turn-End Gate stuck diagnosis).
//!
//! Sends a single stateless `tools: []` request (reusing the `generate.rs` / `llm::stream` pattern)
//! bounded by a 45-second hard timeout so the Lead's prompt cache stays inside Anthropic's 300 s TTL.

pub mod evidence;
pub mod peer;
pub mod wire;

use std::time::{Duration, Instant};

use acpira_rpc::cancel::Cancel;
use acpira_shared::providers::Thinking;
use serde_json::json;

use self::evidence::{DefaultSecretFilter, EvidencePacket, RenderBudget, render_bounded_json};
use self::wire::{Checkpoint, CritiqueOutcome, parse_verdict, system_prompt, user_prompt};
use crate::llm::{self, Endpoint, Event, Item, LlmError, Part, Request, Usage};
use crate::store::Store;

/// Hard wall-clock timeout keeping single-shot zero-tool Critique comfortably inside Anthropic's 300 s cache TTL
pub const CRITIC_TIMEOUT: Duration = Duration::from_secs(45);
/// High enough so always-reasoning models (DeepSeek / Kimi / Gemini) never exhaust output budget during hidden thinking
/// before emitting the final JSON verdict; non-thinking models stop immediately after the ~150-token JSON object
const CRITIC_MAX_TOKENS: u64 = 4096;

/// Run one zero-tool stateless cross-family Critique call over `endpoint` with a bounded JSON evidence packet.
/// Always degrades gracefully to `CritiqueOutcome::Invalid` on timeout, cancellation, stream error, or ungrounded JSON.
#[allow(clippy::too_many_arguments, reason = "pure stateless call taking explicit endpoint, packet and cancellation handle")]
pub async fn run_zero_tool(
  http: ureq::Agent,
  endpoint: Endpoint,
  model_id: String,
  model_label: String,
  checkpoint: Checkpoint,
  original_task: &str,
  draft: &str,
  packet: &EvidencePacket,
  cancel: Option<Cancel>,
) -> CritiqueOutcome {
  let started = Instant::now();
  let bounded = render_bounded_json(packet, RenderBudget::default(), &DefaultSecretFilter);
  let request = Request {
    model: model_id,
    system: system_prompt(checkpoint).to_owned(),
    items: vec![Item::User(vec![Part::Text(user_prompt(checkpoint, original_task, draft, &bounded))])],
    // Strictly zero tools at the wire level: physically impossible to mutate workspace or run commands
    tools: vec![],
    max_tokens: Some(CRITIC_MAX_TOKENS),
    output_limit: Some(CRITIC_MAX_TOKENS),
    sampling: Default::default(),
    thinking: Thinking::Off,
    effort: None,
    cache_key: None,
    fast: crate::llm::fast::SpeedTier::Standard,
  };

  let call = async {
    let mut rx = llm::stream(http, endpoint, request);
    while let Some(ev) = rx.recv().await {
      if let Event::Done { reply, usage, .. } = ev? {
        return Ok((reply.text, usage));
      }
    }
    Err(LlmError::Protocol("the critique stream ended without a result".into()))
  };

  let awaited = match cancel {
    Some(c) => {
      tokio::select! {
        _ = c.cancelled() => return CritiqueOutcome::Invalid { model: model_label, reason: "cancelled".into(), ms: elapsed_ms(started), usage: None },
        r = tokio::time::timeout(CRITIC_TIMEOUT, call) => r,
      }
    }
    None => tokio::time::timeout(CRITIC_TIMEOUT, call).await,
  };

  let ms = elapsed_ms(started);
  let (text, usage): (String, Option<Usage>) = match awaited {
    Ok(Ok(pair)) => pair,
    Ok(Err(e)) => return CritiqueOutcome::Invalid { model: model_label, reason: format!("critic_call_failed: {e}"), ms, usage: None },
    Err(_) => return CritiqueOutcome::Invalid { model: model_label, reason: "critic_timeout".into(), ms, usage: None },
  };

  match parse_verdict(&text) {
    Ok(v) => match v.assessment {
      wire::Assessment::Approve => CritiqueOutcome::Approved { model: model_label, feedback: v.feedback, ms, usage },
      wire::Assessment::Revise => match v.defect {
        Some(defect) => CritiqueOutcome::Revise { model: model_label, feedback: v.feedback, defect, ms, usage },
        None => CritiqueOutcome::Invalid { model: model_label, reason: "critic_invalid: missing defect".into(), ms, usage },
      },
    },
    Err(reason) => CritiqueOutcome::Invalid { model: model_label, reason: format!("critic_invalid: {reason}"), ms, usage },
  }
}

fn elapsed_ms(started: Instant) -> u64 {
  u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

/// Append a structured `critique` audit event to the session JSONL (`scripts/eval/run.ts` reads these counters)
pub fn log_event(store: &Store, checkpoint: Checkpoint, outcome: &CritiqueOutcome) {
  let ev = match outcome {
    CritiqueOutcome::Skipped(reason) => json!({
      "type": "critique",
      "checkpoint": checkpoint.as_str(),
      "outcome": "skipped",
      "skipReason": reason.as_str(),
    }),
    CritiqueOutcome::NoPeer => json!({
      "type": "critique",
      "checkpoint": checkpoint.as_str(),
      "outcome": "skipped",
      "skipReason": "no_cross_family_peer",
    }),
    CritiqueOutcome::Approved { model, feedback, ms, usage } => {
      let mut v = json!({
        "type": "critique",
        "checkpoint": checkpoint.as_str(),
        "outcome": "approve",
        "model": model,
        "ms": ms,
        "feedback": crate::budget::cut(feedback, 500),
      });
      if let Some(u) = usage {
        v["usage"] = json!({ "input": u.input, "output": u.output });
      }
      v
    }
    CritiqueOutcome::Revise { model, feedback, defect, ms, usage } => {
      let mut v = json!({
        "type": "critique",
        "checkpoint": checkpoint.as_str(),
        "outcome": "revise",
        "model": model,
        "ms": ms,
        "target": crate::budget::cut(&defect.target, 200),
        "evidence": crate::budget::cut(&defect.evidence, 400),
        "feedback": crate::budget::cut(feedback, 500),
      });
      if let Some(u) = usage {
        v["usage"] = json!({ "input": u.input, "output": u.output });
      }
      v
    }
    CritiqueOutcome::Invalid { model, reason, ms, usage } => {
      let mut v = json!({
        "type": "critique",
        "checkpoint": checkpoint.as_str(),
        "outcome": "invalid",
        "model": model,
        "ms": ms,
        "reason": crate::budget::cut(reason, 300),
      });
      if let Some(u) = usage {
        v["usage"] = json!({ "input": u.input, "output": u.output });
      }
      v
    }
  };
  store.append(ev);
}
