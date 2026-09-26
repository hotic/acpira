//! Which saved account takes over when the bound one runs out of quota (acpira.accountSwitch). Pure: the account
//! manager feeds it the list, the parked (known exhausted) accounts and the clock

use std::collections::HashMap;

use acpira_shared::transcript::{AccountInfo, AccountQuota};

use crate::util::ms_of_iso;

/// How long an account that reported exhaustion stays out of rotation when its quota does not say when it refills
pub const PARK_FALLBACK_MS: i64 = 30 * 60_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SwitchStrategy {
  /// The account whose nearest allowance window refills first: allowance that is about to reset is spent before it is lost
  EarliestReset,
  /// The account with the largest remaining share in its tightest window
  MostRemaining,
  /// The next usable account in list (added) order
  ListOrder,
  Off,
}

impl SwitchStrategy {
  /// Unknown or absent values fall back to the default, earliestReset
  pub fn parse(s: Option<&str>) -> SwitchStrategy {
    match s {
      Some("mostRemaining") => SwitchStrategy::MostRemaining,
      Some("listOrder") => SwitchStrategy::ListOrder,
      Some("off") => SwitchStrategy::Off,
      _ => SwitchStrategy::EarliestReset,
    }
  }
}

/// A window counts as empty only while its reset is still ahead: a reset time already passed means it has refilled
fn window_empty(remaining: f64, resets_at: Option<&str>, now: i64) -> bool {
  remaining <= 0.0 && resets_at.and_then(ms_of_iso).is_none_or(|at| at > now)
}

fn usable(a: &AccountInfo, current: Option<&str>, parked: &HashMap<String, i64>, now: i64) -> bool {
  Some(a.id.as_str()) != current
    && parked.get(&a.id).is_none_or(|until| *until <= now)
    && a.quota.as_ref().is_none_or(|q| !q.windows.iter().any(|w| window_empty(w.remaining.0, w.resets_at.as_deref(), now)))
}

/// The nearest future reset among the account's windows; i64::MAX when none is known
fn nearest_reset(q: &AccountQuota, now: i64) -> i64 {
  q.windows.iter().filter_map(|w| w.resets_at.as_deref().and_then(ms_of_iso)).filter(|at| *at > now).min().unwrap_or(i64::MAX)
}

fn known(a: &AccountInfo) -> Option<&AccountQuota> {
  a.quota.as_ref().filter(|q| !q.windows.is_empty())
}

/// The share left in the tightest window
fn tightest(q: &AccountQuota) -> f64 {
  q.windows.iter().map(|w| w.remaining.0).fold(1.0, f64::min)
}

/// The account to move to, None when the strategy is off or no other account has allowance left. Accounts whose quota is
/// unknown are still candidates, ranked after every account with a known quota (list order among themselves)
pub fn pick_fallback(
  accounts: &[AccountInfo],
  current: Option<&str>,
  strategy: SwitchStrategy,
  parked: &HashMap<String, i64>,
  now: i64,
) -> Option<String> {
  let candidates: Vec<(usize, &AccountInfo)> = accounts.iter().enumerate().filter(|(_, a)| usable(a, current, parked, now)).collect();
  let best = match strategy {
    SwitchStrategy::Off => None,
    SwitchStrategy::ListOrder => candidates.first().copied(),
    SwitchStrategy::EarliestReset | SwitchStrategy::MostRemaining => candidates.iter().copied().min_by(|(ia, a), (ib, b)| {
      match (known(a), known(b)) {
        (Some(qa), Some(qb)) => {
          let by_reset = nearest_reset(qa, now).cmp(&nearest_reset(qb, now));
          let by_left = tightest(qb).total_cmp(&tightest(qa));
          let primary = if strategy == SwitchStrategy::EarliestReset { by_reset.then(by_left) } else { by_left.then(by_reset) };
          primary.then(ia.cmp(ib))
        }
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => ia.cmp(ib),
      }
    }),
  };
  best.map(|(_, a)| a.id.clone())
}

/// Until when an account that just reported exhaustion stays out of rotation: the latest reset among its empty windows
/// (it is usable again only once all of them refilled), otherwise a fixed pause — the quota service may lag behind the
/// agent, or the exhausted allowance may be one the service does not report (a model-specific window)
pub fn parked_until(quota: Option<&AccountQuota>, now: i64) -> i64 {
  quota
    .into_iter()
    .flat_map(|q| q.windows.iter())
    .filter(|w| window_empty(w.remaining.0, w.resets_at.as_deref(), now))
    .filter_map(|w| w.resets_at.as_deref().and_then(ms_of_iso))
    .max()
    .unwrap_or(now + PARK_FALLBACK_MS)
}

#[cfg(test)]
mod tests {
  use super::*;
  use acpira_shared::num::Num;
  use acpira_shared::transcript::QuotaWindow;

  const NOW: i64 = 1_790_000_000_000;

  fn iso(offset_min: i64) -> String {
    crate::util::iso_of_ms(NOW + offset_min * 60_000)
  }

  fn acc(id: &str, windows: Option<&[(f64, i64)]>) -> AccountInfo {
    AccountInfo {
      id: id.into(),
      agent: "codex".into(),
      label: format!("{id}@x.io"),
      detail: None,
      added_at: String::new(),
      last_used_at: None,
      quota: windows.map(|w| AccountQuota {
        windows: w.iter().map(|(left, reset)| QuotaWindow { id: "weekly".into(), remaining: Num(*left), resets_at: Some(iso(*reset)) }).collect(),
        on_demand_balance_usd: None,
        fetched_at: String::new(),
      }),
    }
  }

  fn pick(list: &[AccountInfo], current: Option<&str>, s: SwitchStrategy) -> Option<String> {
    pick_fallback(list, current, s, &HashMap::new(), NOW)
  }

  #[test]
  fn strategies_rank_the_usable_accounts() {
    let list = [
      acc("cur", Some(&[(0.0, 60)])),
      acc("unknown", None),
      acc("late", Some(&[(0.9, 5000)])),
      acc("soon", Some(&[(0.2, 30), (0.8, 3000)])),
      acc("empty", Some(&[(0.0, 10), (1.0, 20)])),
    ];
    // The nearest window of "soon" refills in 30 min; "empty" is out until its empty window resets
    assert_eq!(pick(&list, Some("cur"), SwitchStrategy::EarliestReset).as_deref(), Some("soon"));
    assert_eq!(pick(&list, Some("cur"), SwitchStrategy::MostRemaining).as_deref(), Some("late"));
    assert_eq!(pick(&list, Some("cur"), SwitchStrategy::ListOrder).as_deref(), Some("unknown"));
    assert_eq!(pick(&list, Some("cur"), SwitchStrategy::Off), None);
    // Unknown quota ranks after known ones but is still a candidate
    assert_eq!(pick(&list[..2], Some("cur"), SwitchStrategy::EarliestReset).as_deref(), Some("unknown"));
    assert_eq!(pick(&list[..1], Some("cur"), SwitchStrategy::EarliestReset), None);
  }

  #[test]
  fn parked_accounts_wait_for_their_reset_and_passed_resets_count_as_refilled() {
    let list = [acc("a", None), acc("b", Some(&[(0.0, -5)]))];
    let parked: HashMap<String, i64> = [("a".to_owned(), NOW + 1)].into();
    // "b" shows 0% but its window reset five minutes ago
    assert_eq!(pick_fallback(&list, None, SwitchStrategy::EarliestReset, &parked, NOW).as_deref(), Some("b"));
    assert_eq!(pick_fallback(&list[..1], None, SwitchStrategy::EarliestReset, &parked, NOW + 2).as_deref(), Some("a"));
  }

  #[test]
  fn parking_lasts_until_every_empty_window_refills() {
    let q = acc("a", Some(&[(0.0, 30), (0.0, 600), (0.5, 10)])).quota;
    assert_eq!(parked_until(q.as_ref(), NOW), NOW + 600 * 60_000);
    assert_eq!(parked_until(acc("b", Some(&[(0.4, 30)])).quota.as_ref(), NOW), NOW + PARK_FALLBACK_MS);
    assert_eq!(parked_until(None, NOW), NOW + PARK_FALLBACK_MS);
    assert_eq!(SwitchStrategy::parse(Some("listOrder")), SwitchStrategy::ListOrder);
    assert_eq!(SwitchStrategy::parse(Some("bogus")), SwitchStrategy::EarliestReset);
  }
}
