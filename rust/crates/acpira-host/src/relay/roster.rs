//! The persona list and the groups of personas (`~/.acpira/subagents.json`), shared by every window and IDE sidecar like
//! accounts.json: read on demand (it is small, and another sidecar may have changed it), written tmp + rename under the
//! file lock

use std::path::PathBuf;

use anyhow::Result;
use serde_json::Value;

use acpira_shared::subagents::{SubagentGroup, SubagentPersona, sanitize_groups, sanitize_personas};

use crate::store::file_lock::{with_file_lock, write_atomic};

pub struct Roster {
  path: PathBuf,
}

/// The file's personas, and its groups checked against them (a member whose persona is gone drops out)
fn parse(text: &str) -> (Vec<SubagentPersona>, Vec<SubagentGroup>) {
  let v: Value = serde_json::from_str(text).unwrap_or(Value::Null);
  let personas = sanitize_personas(v.get("personas").unwrap_or(&v));
  let groups = sanitize_groups(v.get("groups").unwrap_or(&Value::Null), &personas);
  (personas, groups)
}

/// What a list entry is keyed by when two windows' edits are merged
trait Keyed: PartialEq {
  fn key(&self) -> &str;
}

impl Keyed for SubagentPersona {
  fn key(&self) -> &str {
    &self.id
  }
}

impl Keyed for SubagentGroup {
  fn key(&self) -> &str {
    &self.id
  }
}

/// A page's edit applied onto the file as it is now: entries the page removed go, the ones it changed are written
/// over; the ones it left alone keep the file's version, which another window may have changed meanwhile. An entry
/// new to the page is added even when another window took its id meanwhile (sanitizing renumbers it)
fn merge<T: Keyed>(current: Vec<T>, base: &[T], next: Vec<T>) -> Vec<T> {
  let mut out = current;
  out.retain(|o| !base.iter().any(|b| b.key() == o.key()) || next.iter().any(|n| n.key() == o.key()));
  for n in next {
    if base.contains(&n) {
      continue;
    }
    let known = base.iter().any(|b| b.key() == n.key());
    match out.iter_mut().find(|o| o.key() == n.key()) {
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

  /// The directory subagents.json lives in, where the strategy files beside it (`handoff.json`) are read from
  pub fn dir(&self) -> PathBuf {
    self.path.parent().map(PathBuf::from).unwrap_or_default()
  }

  fn read(&self) -> (Vec<SubagentPersona>, Vec<SubagentGroup>) {
    std::fs::read_to_string(&self.path).map(|t| parse(&t)).unwrap_or_default()
  }

  /// Every persona, as sanitized as a hand edit would be; a missing or unreadable file is an empty list
  pub fn list(&self) -> Vec<SubagentPersona> {
    self.read().0
  }

  /// Every group, its members the personas the file still has
  pub fn groups(&self) -> Vec<SubagentGroup> {
    self.read().1
  }

  /// Enabled personas only: what the tool offers
  pub fn enabled(&self) -> Vec<SubagentPersona> {
    self.list().into_iter().filter(|p| p.enabled).collect()
  }

  /// Enabled groups with their enabled members only, and none left without one: what the tool offers
  pub fn enabled_groups(&self) -> Vec<SubagentGroup> {
    let (personas, groups) = self.read();
    let on = |id: &String| personas.iter().any(|p| p.enabled && &p.id == id);
    groups
      .into_iter()
      .filter(|g| g.enabled)
      .map(|g| SubagentGroup { members: g.members.iter().filter(|m| on(m)).cloned().collect(), ..g })
      .filter(|g| !g.members.is_empty())
      .collect()
  }

  pub fn find(&self, id: &str) -> Option<SubagentPersona> {
    let id = id.trim();
    self.enabled().into_iter().find(|p| p.id == id || p.name.eq_ignore_ascii_case(id))
  }

  /// An enabled group by id or name, with its enabled members in order
  pub fn find_group(&self, id: &str) -> Option<(SubagentGroup, Vec<SubagentPersona>)> {
    let id = id.trim();
    let group = self.enabled_groups().into_iter().find(|g| g.id == id || g.name.eq_ignore_ascii_case(id))?;
    let personas = self.enabled();
    let members = group.members.iter().filter_map(|m| personas.iter().find(|p| &p.id == m).cloned()).collect();
    Some((group, members))
  }

  /// The settings page sends `{ base, personas }` and / or `{ baseGroups, groups }`: each list it showed and the one it
  /// wants. Under the file lock its changes against that base are applied onto the file's current list (`merge`); a
  /// list the message leaves out stays as the file has it, and a bare array replaces the personas. The groups are
  /// checked against the personas stored, so a member whose persona went drops out. Answers the personas stored
  pub async fn save(&self, v: &Value) -> Result<Vec<SubagentPersona>> {
    let personas_edit = match v.get("personas") {
      Some(list) => Some((Some(sanitize_personas(v.get("base").unwrap_or(&Value::Null))), sanitize_personas(list))),
      None if v.is_array() => Some((None, sanitize_personas(v))),
      None => None,
    };
    let path = self.path.clone();
    if let Some(dir) = path.parent() {
      tokio::fs::create_dir_all(dir).await?;
    }
    with_file_lock(&path, || async {
      let (current, current_groups) = tokio::fs::read_to_string(&path).await.map(|t| parse(&t)).unwrap_or_default();
      let merged = match personas_edit {
        Some((Some(base), next)) => merge(current, &base, next),
        Some((None, next)) => next,
        None => current,
      };
      // Sanitized again: merged ids stay unique and the list stays within its cap
      let personas = sanitize_personas(&serde_json::to_value(&merged)?);
      let groups = match v.get("groups") {
        // The page's lists are checked against the personas it showed, which a member may have left since
        Some(list) => {
          let base = sanitize_groups(v.get("baseGroups").unwrap_or(&Value::Null), &personas);
          merge(current_groups, &base, sanitize_groups(list, &personas))
        }
        None => current_groups,
      };
      let groups = sanitize_groups(&serde_json::to_value(&groups)?, &personas);
      let body = serde_json::to_vec_pretty(&serde_json::json!({ "version": 1, "personas": personas, "groups": groups }))?;
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
  async fn groups_live_beside_the_personas_and_lose_members_whose_persona_went() {
    let dir = std::env::temp_dir().join(format!("acpira-roster-groups-{}", std::process::id()));
    let roster = Roster::new(&dir);
    roster
      .save(&json!([
        { "name": "Astra", "agent": "codex" },
        { "name": "Argon", "agent": "acpira" },
        { "name": "Opus", "agent": "claude", "enabled": false },
      ]))
      .await
      .unwrap();
    let groups = json!([{ "name": "Reviewers", "members": ["astra", "argon", "opus"], "when": "two reviews" }]);
    roster.save(&json!({ "baseGroups": [], "groups": groups })).await.unwrap();
    assert_eq!(roster.groups()[0].members, ["astra", "argon", "opus"]);
    // Offered with its enabled members only, found by id or name
    assert_eq!(roster.enabled_groups()[0].members, ["astra", "argon"]);
    let (g, members) = roster.find_group("Reviewers").unwrap();
    assert_eq!((g.id.as_str(), members.iter().map(|p| p.id.as_str()).collect::<Vec<_>>()), ("reviewers", vec!["astra", "argon"]));
    // A persona edit leaves the groups alone, except for a member whose persona it removed
    let base = serde_json::to_value(roster.list()).unwrap();
    let mut next = base.clone();
    next.as_array_mut().unwrap().remove(1);
    roster.save(&json!({ "base": base, "personas": next })).await.unwrap();
    assert_eq!(roster.groups()[0].members, ["astra", "opus"]);
    // A group with no enabled member left is not offered
    let base = serde_json::to_value(roster.groups()).unwrap();
    let mut only_opus = base.clone();
    only_opus[0]["members"] = json!(["opus"]);
    roster.save(&json!({ "baseGroups": base, "groups": only_opus })).await.unwrap();
    assert!(roster.enabled_groups().is_empty() && roster.find_group("reviewers").is_none());
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
