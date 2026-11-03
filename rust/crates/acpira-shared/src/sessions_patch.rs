//! Session list patches. The list holds every session on the machine (hundreds of KB as JSON) and changes whenever any
//! session starts, waits or ends; a page that asked for list patches (`ready.listPatches`, the paired phone) gets the new
//! list as runs over the list it holds instead of the whole list again. The TypeScript side has the types only
//! (`SessionsPatch` in packages/shared/protocol.ts); the phone applies them (`AcpiraSync/SessionsPatch.swift`)

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::transcript::SessionSummary;

/// One run of the new list: `[from, count]` copies that many entries of the held list from index `from`; an object is a
/// new or changed entry
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ListRun {
  Keep([usize; 2]),
  Entry(Box<SessionSummary>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionsPatch {
  /// The revision of the list the patch applies to and the one it makes: a whole list is revision 0, each patch one
  /// more. A page holding another revision (a patch it could not decode) asks for the whole list (`resyncSessions`)
  pub base: u64,
  pub rev: u64,
  /// Length of the list the patch applies to
  pub size: usize,
  pub runs: Vec<ListRun>,
}

/// The patch from `old` to `new`; `None` when they are equal. Entries are matched by id, so a session that moved keeps
/// being copied rather than resent
pub fn diff(old: &[SessionSummary], new: &[SessionSummary], base: u64) -> Option<SessionsPatch> {
  if old == new {
    return None;
  }
  let at: HashMap<&str, usize> = old.iter().enumerate().map(|(i, s)| (s.id.as_str(), i)).collect();
  let mut runs: Vec<ListRun> = Vec::new();
  for s in new {
    match at.get(s.id.as_str()).copied().filter(|&j| old.get(j) == Some(s)) {
      Some(j) => match runs.last_mut() {
        Some(ListRun::Keep([from, count])) if *from + *count == j => *count += 1,
        _ => runs.push(ListRun::Keep([j, 1])),
      },
      None => runs.push(ListRun::Entry(Box::new(s.clone()))),
    }
  }
  Some(SessionsPatch { base, rev: base + 1, size: old.len(), runs })
}

/// `held` (at revision `rev`) with `patch` applied; `None` when the patch does not fit it
pub fn apply(held: &[SessionSummary], rev: u64, patch: &SessionsPatch) -> Option<Vec<SessionSummary>> {
  if rev != patch.base || held.len() != patch.size {
    return None;
  }
  let mut out = Vec::new();
  for run in &patch.runs {
    match run {
      ListRun::Keep([from, count]) => out.extend_from_slice(held.get(*from..from.checked_add(*count)?)?),
      ListRun::Entry(s) => out.push((**s).clone()),
    }
  }
  Some(out)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn s(id: &str, title: &str) -> SessionSummary {
    serde_json::from_value(serde_json::json!({ "id": id, "title": title, "agent": "claude", "cwd": "/w", "updatedAt": "t" })).unwrap()
  }

  #[test]
  fn a_change_that_moves_one_entry_resends_only_that_entry() {
    let old: Vec<_> = (0..100).map(|i| s(&format!("s{i}"), "t")).collect();
    let mut new = old.clone();
    let mut moved = new.remove(40);
    moved.title = "renamed".into();
    new.insert(0, moved);
    new.remove(70);
    new.push(s("fresh", "t"));
    let p = diff(&old, &new, 4).unwrap();
    assert_eq!(p.runs.iter().filter(|r| matches!(r, ListRun::Entry(_))).count(), 2);
    assert_eq!(apply(&old, 4, &p).unwrap(), new);
    assert_eq!(apply(&old, 3, &p), None, "a patch for another revision");
    let wire = serde_json::to_string(&p).unwrap();
    assert!(wire.starts_with(r#"{"base":4,"rev":5,"size":100,"runs":[{"id":"s40""#), "{wire}");
    assert_eq!(serde_json::from_str::<SessionsPatch>(&wire).unwrap(), p);
  }

  #[test]
  fn equal_lists_need_no_patch_and_a_wrong_base_is_refused() {
    let old = vec![s("a", "t"), s("b", "t")];
    assert_eq!(diff(&old, &old, 0), None);
    let p = diff(&old, &[s("b", "t")], 0).unwrap();
    assert_eq!(apply(&old[..1], 0, &p), None);
    assert_eq!(apply(&old, 0, &SessionsPatch { base: 0, rev: 1, size: 2, runs: vec![ListRun::Keep([1, 5])] }), None);
  }
}
