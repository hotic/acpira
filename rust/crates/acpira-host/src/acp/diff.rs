//! Line-level diff: git's histogram algorithm (via `similar`), so a small edit in a large file stays small.
//! A time budget bounds pathological inputs; past it the algorithm settles for a coarser but still valid script.

use std::time::{Duration, Instant};

use acpira_shared::transcript::{DiffKind, DiffLine};
use similar::{Algorithm, DiffTag, capture_diff_slices_deadline};

use crate::i18n::tp;

/// Wall-clock budget for one diff; beyond it `similar` returns an approximate (never wrong) script
const DIFF_BUDGET: Duration = Duration::from_millis(200);

fn line(kind: DiffKind, text: String, old_line: Option<u64>, new_line: Option<u64>) -> DiffLine {
  DiffLine { kind, text, old_line, new_line }
}

pub fn diff_lines(old_text: &str, new_text: &str) -> Vec<DiffLine> {
  let a = source_lines(old_text);
  let b = source_lines(new_text);
  let ops = capture_diff_slices_deadline(Algorithm::Histogram, &a, &b, Some(Instant::now() + DIFF_BUDGET));
  let mut out = vec![];
  for op in ops {
    let (tag, old, new) = op.as_tag_tuple();
    match tag {
      DiffTag::Equal => {
        out.extend(old.zip(new).map(|(i, j)| line(DiffKind::Ctx, format!(" {}", a[i]), Some(i as u64 + 1), Some(j as u64 + 1))));
      }
      DiffTag::Delete | DiffTag::Insert | DiffTag::Replace => {
        push_changed(&mut out, &a, old, DiffKind::Del);
        push_changed(&mut out, &b, new, DiffKind::Add);
      }
    }
  }
  collapse_context(out, 3)
}

fn push_changed(out: &mut Vec<DiffLine>, src: &[&str], range: std::ops::Range<usize>, kind: DiffKind) {
  for i in range {
    let n = Some(i as u64 + 1);
    out.push(match kind {
      DiffKind::Del => line(kind, format!("-{}", src[i]), n, None),
      _ => line(kind, format!("+{}", src[i]), None, n),
    });
  }
}

fn source_lines(text: &str) -> Vec<&str> {
  if text.is_empty() {
    return vec![];
  }
  let mut lines: Vec<&str> = text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).collect();
  if lines.last().is_some_and(|l| l.is_empty()) {
    lines.pop();
  }
  lines
}

/// Keep 3 lines of context around changes; a leading hunk stays, a trailing one is dropped
fn collapse_context(lines: Vec<DiffLine>, keep: usize) -> Vec<DiffLine> {
  let mut out: Vec<DiffLine> = vec![];
  let mut run: Vec<DiffLine> = vec![];
  let flush = |out: &mut Vec<DiffLine>, run: &mut Vec<DiffLine>, at_end: bool| {
    let r = std::mem::take(run);
    if r.len() <= keep * 2 || (out.is_empty() && r.len() <= keep) || (at_end && r.len() <= keep) {
      out.extend(r);
    } else if at_end {
      out.extend(r.into_iter().take(keep));
    } else {
      let head = if out.is_empty() { 0 } else { keep };
      let n = r.len() - head - keep;
      out.extend_from_slice(&r[..head]);
      out.push(line(DiffKind::Hunk, tp("host.unchanged", &[("n", &n.to_string())]), None, None));
      out.extend_from_slice(&r[r.len() - keep..]);
    }
  };
  for l in lines {
    if l.kind == DiffKind::Ctx {
      run.push(l);
    } else {
      flush(&mut out, &mut run, false);
      out.push(l);
    }
  }
  flush(&mut out, &mut run, true);
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn collapses_long_context() {
    let old: String = (1..=20).map(|i| format!("l{i}\n")).collect();
    let new = old.replace("l10\n", "L10\n");
    let d = diff_lines(&old, &new);
    let kinds: Vec<_> = d.iter().map(|l| l.kind).collect();
    // leading hunk, 3 ctx, del, add, 3 ctx (trailing run keeps its head)
    assert_eq!(kinds[0], DiffKind::Hunk);
    assert_eq!(kinds.iter().filter(|k| **k == DiffKind::Ctx).count(), 6);
    assert_eq!(d[1].old_line, Some(7));
    assert_eq!(d.last().unwrap().text, " l13");
    assert_eq!(diff_lines("", "a\nb\n").len(), 2);
  }

  #[test]
  fn a_small_edit_in_a_large_file_stays_small() {
    let old: String = (1..=1600).map(|i| format!("fn f{i}() {{}}\n")).collect();
    let new = old.replace("fn f900() {}\n", "fn f900() { todo!() }\nfn extra() {}\n");
    let d = diff_lines(&old, &new);
    let changed: Vec<_> = d.iter().filter(|l| matches!(l.kind, DiffKind::Add | DiffKind::Del)).collect();
    assert_eq!(changed.len(), 3);
    assert_eq!((changed[0].kind, changed[0].old_line), (DiffKind::Del, Some(900)));
    assert_eq!(changed[1].new_line, Some(900));
    assert_eq!(changed[2].new_line, Some(901));
  }

  #[test]
  fn a_full_rewrite_keeps_every_line_so_the_stat_is_exact() {
    let d = diff_lines(&"old\n".repeat(1000), &"new\n".repeat(1000));
    assert_eq!(d.iter().filter(|l| l.kind == DiffKind::Del).count(), 1000);
    assert_eq!(d.iter().filter(|l| l.kind == DiffKind::Add).count(), 1000);
  }
}
