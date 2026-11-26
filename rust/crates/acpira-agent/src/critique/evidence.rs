//! Bounded implementation evidence packet, secret filtering (fail-closed), verifier whitelist,
//! and User Interaction Carryover for zero-tool Critique and Turn-End Gate diagnosis.

use std::path::Path;
use std::process::Command;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::wire::Checkpoint;

/// Three-dimensional rendering budget (Bytes / Nodes / Depth) plus Git diff limits
#[derive(Debug, Clone, Copy)]
pub struct RenderBudget {
  pub max_bytes: usize,
  pub max_nodes: usize,
  pub max_depth: usize,
  pub max_summary_files: usize,
  pub max_diff_files: usize,
  pub max_diff_lines: usize,
  pub max_diff_bytes: usize,
  pub max_path_bytes: usize,
}

impl Default for RenderBudget {
  fn default() -> Self {
    Self {
      max_bytes: 24 * 1024,
      max_nodes: 64,
      max_depth: 4,
      max_summary_files: 30,
      max_diff_files: 12,
      max_diff_lines: 300,
      max_diff_bytes: 12 * 1024,
      max_path_bytes: 256,
    }
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InteractionKind {
  Question,
  Permission,
  PlanApproval,
}

/// Settled user interaction recorded during the first pass (User Interaction Carryover)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserInteraction {
  pub kind: InteractionKind,
  pub prompt: String,
  pub response: String,
}

/// One bounded write or verifier tool action from the turn
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrimaryAction {
  pub tool: String,
  pub summary: String,
  pub output: String,
  pub is_error: bool,
  pub is_verifier: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceStatus {
  Clean,
  Dirty,
  NotAGitWorkspace,
  Unavailable,
}

/// Complete structured evidence packet fed to the zero-tool Critic
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidencePacket {
  pub checkpoint: Checkpoint,
  pub workspace_status: WorkspaceStatus,
  pub total_changed_files: usize,
  pub changed_files: Vec<String>,
  pub omitted_files: usize,
  pub diff: Option<String>,
  pub diff_truncated: bool,
  pub verifier_evidence: Vec<PrimaryAction>,
  pub primary_actions: Vec<PrimaryAction>,
  pub user_interactions: Vec<UserInteraction>,
  pub plan_text: Option<String>,
  pub gate_reasons: Vec<String>,
}

impl EvidencePacket {
  pub fn empty(checkpoint: Checkpoint) -> Self {
    Self {
      checkpoint,
      workspace_status: WorkspaceStatus::Clean,
      total_changed_files: 0,
      changed_files: vec![],
      omitted_files: 0,
      diff: None,
      diff_truncated: false,
      verifier_evidence: vec![],
      primary_actions: vec![],
      user_interactions: vec![],
      plan_text: None,
      gate_reasons: vec![],
    }
  }
}

/// Recognized verification commands whose recorded outputs are retained in the evidence packet
pub fn is_verifier_command(command: &str) -> bool {
  let low = command.to_ascii_lowercase();
  let markers = [
    "cargo test",
    "cargo check",
    "cargo clippy",
    "cargo build",
    "pnpm run build",
    "pnpm run lint",
    "pnpm test",
    "pnpm exec vitest",
    "pnpm typecheck",
    "npm run build",
    "npm run lint",
    "npm test",
    "yarn build",
    "yarn lint",
    "yarn test",
    "go test",
    "dotnet test",
    "pytest",
    "unittest",
    "swift test",
    "gradlew",
  ];
  markers.iter().any(|m| low.contains(m))
}

/// Files whose raw contents/diffs are never packed into external review context
pub fn is_sensitive_path(path: &str) -> bool {
  let base = Path::new(path).file_name().and_then(|s| s.to_str()).unwrap_or(path).to_ascii_lowercase();
  base == ".env"
    || base.starts_with(".env.")
    || base == "secrets.json"
    || base.ends_with(".pem")
    || base.ends_with(".key")
    || base == "id_rsa"
    || base == "id_ed25519"
}

/// Returned when secret filtering fails or is unavailable, triggering fail-closed omission
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecretFilterError;

/// Fail-closed secret filtering interface: returning `Err(SecretFilterError)` causes callers to omit the field rather than leak it
pub trait SecretFilter: Send + Sync {
  fn filter(&self, text: &str) -> Result<String, SecretFilterError>;
}

/// Default built-in regex secret scrubber covering common keys, tokens, PEM blocks, and secret assignments
pub struct DefaultSecretFilter;

impl SecretFilter for DefaultSecretFilter {
  fn filter(&self, text: &str) -> Result<String, SecretFilterError> {
    static PATTERNS: std::sync::OnceLock<Result<Vec<Regex>, SecretFilterError>> = std::sync::OnceLock::new();
    let regexes = PATTERNS.get_or_init(|| {
      let raw = [
        r"(?s)-----BEGIN [A-Z0-9 ]*PRIVATE KEY-----.*?-----END [A-Z0-9 ]*PRIVATE KEY-----",
        r"\b(sk-ant-[A-Za-z0-9_\-]{16,}|sk-proj-[A-Za-z0-9_\-]{16,}|sk-[A-Za-z0-9_\-]{20,})\b",
        r"\b(ghp_[A-Za-z0-9]{20,}|gho_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{20,})\b",
        r"\b(AKIA[0-9A-Z]{16}|AIza[0-9A-Za-z_\-]{35}|xox[baprs]-[A-Za-z0-9\-]{10,})\b",
        r"(?i)\bBearer\s+[A-Za-z0-9._\-]{16,}",
        r#"(?i)\b(api[_-]?key|access[_-]?token|secret[_-]?key|password|private[_-]?key)\s*[:=]\s*["']?[^\s"']{8,}["']?"#,
      ];
      raw.into_iter().map(|p| Regex::new(p).map_err(|_| SecretFilterError)).collect()
    });
    let regexes = regexes.as_ref().map_err(|_| SecretFilterError)?;
    let mut out = text.to_owned();
    for re in regexes {
      out = re.replace_all(&out, "[REDACTED]").into_owned();
    }
    Ok(out)
  }
}

/// Fail-closed scrubber: if the secret filter errors or is unavailable, omit the text completely
pub fn filter_or_omit(filter: &dyn SecretFilter, text: &str) -> String {
  match filter.filter(text) {
    Ok(clean) => clean,
    Err(SecretFilterError) => "[omitted: secret filtering unavailable]".to_owned(),
  }
}

/// Collect bounded Git workspace status and diff from `cwd`, filtering sensitive paths and secrets.
/// Explicitly disables `core.fsmonitor`, `diff.external`, and `textconv` so inspecting an untrusted repo never runs hooks
pub fn populate_git_evidence(packet: &mut EvidencePacket, cwd: &Path, budget: RenderBudget, filter: &dyn SecretFilter) {
  let status = Command::new("git")
    .args(["-c", "core.fsmonitor=false", "-c", "core.quotepath=false", "status", "--porcelain=v1"])
    .current_dir(cwd)
    .output();
  let Ok(status) = status else {
    packet.workspace_status = WorkspaceStatus::Unavailable;
    return;
  };
  if !status.status.success() {
    packet.workspace_status = WorkspaceStatus::NotAGitWorkspace;
    return;
  }
  let stdout = String::from_utf8_lossy(&status.stdout);
  let mut files: Vec<String> = stdout
    .lines()
    .filter_map(|l| l.get(3..))
    .map(|l| l.trim().trim_matches('"').to_owned())
    .filter(|p| !p.is_empty())
    .collect();
  packet.total_changed_files = files.len();
  if files.is_empty() {
    packet.workspace_status = WorkspaceStatus::Clean;
    return;
  }
  packet.workspace_status = WorkspaceStatus::Dirty;
  let safe_files: Vec<String> = files
    .drain(..)
    .filter(|p| !is_sensitive_path(p))
    .map(|p| crate::budget::cut(&p, budget.max_path_bytes).to_owned())
    .collect();
  packet.omitted_files = packet.total_changed_files.saturating_sub(safe_files.len().min(budget.max_summary_files));
  packet.changed_files = safe_files.into_iter().take(budget.max_summary_files).collect();

  let diff_targets: Vec<&str> = packet.changed_files.iter().take(budget.max_diff_files).map(String::as_str).collect();
  if diff_targets.is_empty() {
    return;
  }
  let mut cmd = Command::new("git");
  cmd
    .args(["-c", "core.fsmonitor=false", "-c", "core.quotepath=false", "diff", "--no-ext-diff", "--no-textconv", "HEAD", "--"])
    .args(&diff_targets)
    .current_dir(cwd);
  let Ok(diff_out) = cmd.output() else { return };
  if !diff_out.status.success() {
    return;
  }
  let raw_diff = String::from_utf8_lossy(&diff_out.stdout);
  if raw_diff.trim().is_empty() {
    return;
  }
  let scrubbed = filter_or_omit(filter, &raw_diff);
  let mut kept = String::new();
  let mut truncated = false;
  for (idx, line) in scrubbed.lines().enumerate() {
    if idx >= budget.max_diff_lines || kept.len() + line.len() + 1 > budget.max_diff_bytes {
      truncated = true;
      break;
    }
    if idx > 0 {
      kept.push('\n');
    }
    kept.push_str(line);
  }
  if truncated {
    kept.push_str("\n[bounded diff truncated]");
  }
  packet.diff = Some(kept);
  packet.diff_truncated = truncated;
}

/// Scrub all textual fields of `packet` fail-closed and serialize into a JSON string strictly bounded by
/// `RenderBudget` (`max_bytes`, `max_nodes`, `max_depth`)
pub fn render_bounded_json(packet: &EvidencePacket, budget: RenderBudget, filter: &dyn SecretFilter) -> String {
  let scrub = |s: &str| filter_or_omit(filter, s);
  let scrub_action = |a: &PrimaryAction| PrimaryAction {
    tool: a.tool.clone(),
    summary: scrub(&a.summary),
    output: scrub(&a.output),
    is_error: a.is_error,
    is_verifier: a.is_verifier,
  };
  let clean = EvidencePacket {
    checkpoint: packet.checkpoint,
    workspace_status: packet.workspace_status,
    total_changed_files: packet.total_changed_files,
    changed_files: packet.changed_files.iter().map(|p| crate::budget::cut(p, budget.max_path_bytes).to_owned()).collect(),
    omitted_files: packet.omitted_files,
    diff: packet.diff.as_deref().map(scrub),
    diff_truncated: packet.diff_truncated,
    verifier_evidence: packet.verifier_evidence.iter().map(scrub_action).collect(),
    primary_actions: packet.primary_actions.iter().map(scrub_action).collect(),
    user_interactions: packet
      .user_interactions
      .iter()
      .map(|u| UserInteraction { kind: u.kind, prompt: scrub(&u.prompt), response: scrub(&u.response) })
      .collect(),
    plan_text: packet.plan_text.as_deref().map(scrub),
    gate_reasons: packet.gate_reasons.iter().map(|r| scrub(r)).collect(),
  };

  let raw_val = serde_json::to_value(&clean).unwrap_or_else(|_| json!({ "error": "evidence unavailable" }));
  let mut nodes_left = budget.max_nodes;
  let pruned = prune_value(raw_val, 0, budget.max_depth, &mut nodes_left, (budget.max_bytes / 2).max(512));
  let mut text = serde_json::to_string_pretty(&pruned).unwrap_or_else(|_| "{}".to_owned());
  if text.len() > budget.max_bytes {
    let cap = budget.max_bytes.saturating_sub(64);
    let prefix = crate::budget::cut(&text, cap);
    text = format!("{prefix}\n\"[evidence packet exceeded rendering budget]\"");
  }
  text
}

fn prune_value(v: Value, depth: usize, max_depth: usize, nodes_left: &mut usize, max_str_bytes: usize) -> Value {
  if *nodes_left == 0 {
    return Value::String("[withheld: node budget exceeded]".into());
  }
  *nodes_left -= 1;
  match v {
    Value::String(s) => {
      if s.len() > max_str_bytes {
        Value::String(format!("{}… [truncated]", crate::budget::cut(&s, max_str_bytes)))
      } else {
        Value::String(s)
      }
    }
    Value::Array(arr) => {
      if depth >= max_depth {
        return Value::String(format!("[withheld: array depth {} exceeded]", arr.len()));
      }
      let mut out = Vec::with_capacity(arr.len());
      for item in arr {
        if *nodes_left == 0 {
          out.push(Value::String("[withheld: remaining items exceed node budget]".into()));
          break;
        }
        out.push(prune_value(item, depth + 1, max_depth, nodes_left, max_str_bytes));
      }
      Value::Array(out)
    }
    Value::Object(map) => {
      if depth >= max_depth {
        return Value::String("[withheld: object depth exceeded]".into());
      }
      let mut out = Map::new();
      for (k, val) in map {
        if *nodes_left == 0 {
          out.insert("_truncated".into(), Value::String("node budget exceeded".into()));
          break;
        }
        out.insert(k, prune_value(val, depth + 1, max_depth, nodes_left, max_str_bytes));
      }
      Value::Object(out)
    }
    other => other,
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  struct BrokenFilter;
  impl SecretFilter for BrokenFilter {
    fn filter(&self, _text: &str) -> Result<String, SecretFilterError> {
      Err(SecretFilterError)
    }
  }

  #[test]
  fn secret_filter_scrubs_keys_and_fails_closed() {
    let f = DefaultSecretFilter;
    let raw = "Authorization: Bearer super_secret_token_1234567890 and key sk-ant-api03-abcdef1234567890xyz";
    let clean = filter_or_omit(&f, raw);
    assert!(!clean.contains("super_secret_token") && !clean.contains("sk-ant-api03"), "{clean}");
    assert!(clean.contains("[REDACTED]"), "{clean}");

    // Fail-closed: broken filter omits the text entirely rather than leaking raw secrets
    let omitted = filter_or_omit(&BrokenFilter, raw);
    assert_eq!(omitted, "[omitted: secret filtering unavailable]");
    assert!(!omitted.contains("sk-ant"));
  }

  #[test]
  fn rendering_budget_enforces_bytes_nodes_and_depth() {
    let mut pkt = EvidencePacket::empty(Checkpoint::Draft);
    pkt.diff = Some("x".repeat(50_000));
    for i in 0..40 {
      pkt.changed_files.push(format!("src/file_{i}.rs"));
    }
    let tight = RenderBudget { max_bytes: 1200, max_nodes: 20, max_depth: 2, ..Default::default() };
    let rendered = render_bounded_json(&pkt, tight, &DefaultSecretFilter);
    assert!(rendered.len() <= tight.max_bytes, "rendered len {} > {}", rendered.len(), tight.max_bytes);
    assert!(rendered.contains("withheld") || rendered.contains("truncated"), "{rendered}");
  }

  #[test]
  fn verifier_and_sensitive_path_helpers_match_expected_entries() {
    assert!(is_verifier_command("cd rust && cargo test -p acpira-agent --lib"));
    assert!(is_verifier_command("pnpm exec vitest run test/relay.test.ts"));
    assert!(!is_verifier_command("git status"));
    assert!(is_sensitive_path(".env.local"));
    assert!(is_sensitive_path("config/secrets.json"));
    assert!(!is_sensitive_path("src/main.rs"));
  }
}
