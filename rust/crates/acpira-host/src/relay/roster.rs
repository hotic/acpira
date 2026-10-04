//! The persona list (`~/.acpira/subagents.json`), shared by every window and IDE sidecar like accounts.json: read on
//! demand (it is small, and another sidecar may have changed it), written tmp + rename under the file lock

use std::path::PathBuf;

use anyhow::Result;
use serde_json::Value;

use acpira_shared::subagents::{SubagentPersona, sanitize_personas};

use crate::store::file_lock::{with_file_lock, write_atomic};

pub struct Roster {
  path: PathBuf,
}

fn parse(text: &str) -> Vec<SubagentPersona> {
  let v: Value = serde_json::from_str(text).unwrap_or(Value::Null);
  sanitize_personas(v.get("personas").unwrap_or(&v))
}

/// A page's edit applied onto the file as it is now: personas the page removed go, the ones it changed are written
/// over; the ones it left alone keep the file's version, which another window may have changed meanwhile. A persona
/// new to the page is added even when another window took its id meanwhile (`sanitize_personas` renumbers it)
fn merge(current: Vec<SubagentPersona>, base: &[SubagentPersona], next: Vec<SubagentPersona>) -> Vec<SubagentPersona> {
  let mut out = current;
  out.retain(|o| !base.iter().any(|b| b.id == o.id) || next.iter().any(|n| n.id == o.id));
  for n in next {
    if base.contains(&n) {
      continue;
    }
    let known = base.iter().any(|b| b.id == n.id);
    match out.iter_mut().find(|o| o.id == n.id) {
      Some(o) if known => *o = n,
      _ => out.push(n),
    }
  }
  out
}

impl Roster {
  pub fn new(root: &std::path::Path) -> Self {
    Roster { path: root.join("subagents.json") }
  }

  /// Every persona, as sanitized as a hand edit would be; a missing or unreadable file is an empty list
  pub fn list(&self) -> Vec<SubagentPersona> {
    std::fs::read_to_string(&self.path).map(|t| parse(&t)).unwrap_or_default()
  }

  /// Enabled personas only: what the tool offers
  pub fn enabled(&self) -> Vec<SubagentPersona> {
    self.list().into_iter().filter(|p| p.enabled).collect()
  }

  pub fn find(&self, id: &str) -> Option<SubagentPersona> {
    let id = id.trim();
    self.enabled().into_iter().find(|p| p.id == id || p.name.eq_ignore_ascii_case(id))
  }

  /// The settings page sends `{ base, personas }`: the list it showed and the list it wants. Under the file lock its
  /// changes against that base are applied onto the file's current list (`merge`); a bare list replaces the file.
  /// Answers what was stored
  pub async fn save(&self, v: &Value) -> Result<Vec<SubagentPersona>> {
    let (base, next) = match v.get("personas") {
      Some(list) => (Some(sanitize_personas(v.get("base").unwrap_or(&Value::Null))), sanitize_personas(list)),
      None => (None, sanitize_personas(v)),
    };
    let path = self.path.clone();
    if let Some(dir) = path.parent() {
      tokio::fs::create_dir_all(dir).await?;
    }
    with_file_lock(&path, || async {
      let merged = match &base {
        Some(base) => merge(tokio::fs::read_to_string(&path).await.map(|t| parse(&t)).unwrap_or_default(), base, next),
        None => next,
      };
      // Sanitized again: merged ids stay unique and the list stays within its cap
      let personas = sanitize_personas(&serde_json::to_value(&merged)?);
      let body = serde_json::to_vec_pretty(&serde_json::json!({ "version": 1, "personas": personas }))?;
      write_atomic(&path, &body, None).await?;
      Ok(personas)
    })
    .await
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[tokio::test]
  async fn saves_sanitized_unique_slugs_and_reads_them_back() {
    let dir = std::env::temp_dir().join(format!("acpira-roster-{}", std::process::id()));
    let roster = Roster::new(&dir);
    assert!(roster.list().is_empty());
    let saved = roster
      .save(&json!([
        { "name": "Codex Review", "agent": "codex", "model": " gpt-6 ", "mode": "consult", "when": "review diffs" },
        { "name": "Codex Review", "agent": "codex", "mode": "work", "enabled": false },
        { "name": "", "agent": "codex" },
        { "name": "快手", "agent": "opencode", "mode": "work" },
      ]))
      .await
      .unwrap();
    let ids: Vec<&str> = saved.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["codex-review", "codex-review-2", "agent"]);
    assert_eq!(saved[0].model.as_deref(), Some("gpt-6"));
    assert_eq!(roster.list(), saved);
    assert_eq!(roster.enabled().len(), 2);
    assert_eq!(roster.find("Codex Review").map(|p| p.id), Some("codex-review".into()));
    assert!(roster.find("codex-review-2").is_none(), "disabled personas are not summonable");
    std::fs::remove_dir_all(&dir).ok();
  }

  #[tokio::test]
  async fn two_windows_editing_from_the_same_list_keep_each_others_changes() {
    let dir = std::env::temp_dir().join(format!("acpira-roster-merge-{}", std::process::id()));
    let roster = Roster::new(&dir);
    let a = roster.save(&json!([{ "name": "A", "agent": "codex", "when": "first" }])).await.unwrap();
    let base = serde_json::to_value(&a).unwrap();
    let edit = |personas: Value| json!({ "base": base, "personas": personas });
    // Window one adds B; window two, still showing [A], edits A: both stay
    let mut with_b = base.clone();
    with_b.as_array_mut().unwrap().push(json!({ "name": "B", "agent": "opencode" }));
    roster.save(&edit(with_b)).await.unwrap();
    let mut edited = base.clone();
    edited[0]["when"] = json!("second");
    let stored = roster.save(&edit(edited)).await.unwrap();
    assert_eq!(stored.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["a", "b"]);
    assert_eq!(stored[0].when, "second");
    // A third window, also from [A], adds its own B: a new one, not B overwritten
    let mut other_b = base.clone();
    other_b.as_array_mut().unwrap().push(json!({ "name": "B", "agent": "claude" }));
    let stored = roster.save(&edit(other_b)).await.unwrap();
    assert_eq!(stored.iter().map(|p| (p.id.as_str(), p.agent.as_str())).collect::<Vec<_>>(), [("a", "codex"), ("b", "opencode"), ("b-2", "claude")]);
    // Deleting A from that same base removes A only
    let stored = roster.save(&edit(json!([]))).await.unwrap();
    assert_eq!(stored.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["b", "b-2"]);
    std::fs::remove_dir_all(&dir).ok();
  }
}
