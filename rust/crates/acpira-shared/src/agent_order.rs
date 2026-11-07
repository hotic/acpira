//! Agent ordering and enablement (mirror of packages/shared/agentOrder.ts)

use std::collections::{HashMap, HashSet};

use crate::transcript::{AgentId, AgentInfo};

#[derive(Debug, Clone, Default)]
pub struct AgentPrefs {
  pub order: Vec<AgentId>,
  pub disabled: Vec<AgentId>,
}

/// Listed ids first in their saved order, the rest keep registry order after them.
/// Disabled agents are flagged, never dropped
pub fn arrange_agents(list: Vec<AgentInfo>, prefs: &AgentPrefs) -> Vec<AgentInfo> {
  let rank: HashMap<&str, usize> = prefs.order.iter().enumerate().map(|(i, id)| (id.as_str(), i)).collect();
  let off: HashSet<&str> = prefs.disabled.iter().map(String::as_str).collect();
  let mut keyed: Vec<((usize, usize), AgentInfo)> = list
    .into_iter()
    .enumerate()
    .map(|(i, mut a)| {
      let k = (rank.get(a.id.as_str()).copied().unwrap_or(prefs.order.len()), i);
      if off.contains(a.id.as_str()) {
        a.disabled = Some(true);
      }
      (k, a)
    })
    .collect();
  keyed.sort_by_key(|(k, _)| *k);
  keyed.into_iter().map(|(_, a)| a).collect()
}
