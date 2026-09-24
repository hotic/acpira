//! Line-level diff: LCS over the common lines; beyond 800 combined lines only a head is shown

use acpira_shared::transcript::{DiffKind, DiffLine};

use crate::i18n::tp;

fn line(kind: DiffKind, text: String, old_line: Option<u64>, new_line: Option<u64>) -> DiffLine {
  DiffLine { kind, text, old_line, new_line }
}

pub fn diff_lines(old_text: &str, new_text: &str) -> Vec<DiffLine> {
  let a = source_lines(old_text);
  let b = source_lines(new_text);
  if a.len() + b.len() > 800 {
    let mut out = vec![line(DiffKind::Hunk, tp("host.hunk", &[("a", &a.len().to_string()), ("b", &b.len().to_string())]), None, None)];
    out.extend(a.iter().take(40).enumerate().map(|(i, t)| line(DiffKind::Del, format!("-{t}"), Some(i as u64 + 1), None)));
    out.extend(b.iter().take(40).enumerate().map(|(i, t)| line(DiffKind::Add, format!("+{t}"), None, Some(i as u64 + 1))));
    return out;
  }
  let (m, n) = (a.len(), b.len());
  let w = n + 1;
  let mut dp = vec![0u32; (m + 1) * w];
  for i in (0..m).rev() {
    for j in (0..n).rev() {
      dp[i * w + j] = if a[i] == b[j] { dp[(i + 1) * w + j + 1] + 1 } else { dp[(i + 1) * w + j].max(dp[i * w + j + 1]) };
    }
  }
  let mut out = vec![];
  let (mut i, mut j) = (0, 0);
  while i < m && j < n {
    if a[i] == b[j] {
      out.push(line(DiffKind::Ctx, format!(" {}", a[i]), Some(i as u64 + 1), Some(j as u64 + 1)));
      i += 1;
      j += 1;
    } else if dp[(i + 1) * w + j] >= dp[i * w + j + 1] {
      out.push(line(DiffKind::Del, format!("-{}", a[i]), Some(i as u64 + 1), None));
      i += 1;
    } else {
      out.push(line(DiffKind::Add, format!("+{}", b[j]), None, Some(j as u64 + 1)));
      j += 1;
    }
  }
  while i < m {
    out.push(line(DiffKind::Del, format!("-{}", a[i]), Some(i as u64 + 1), None));
    i += 1;
  }
  while j < n {
    out.push(line(DiffKind::Add, format!("+{}", b[j]), None, Some(j as u64 + 1)));
    j += 1;
  }
  collapse_context(out, 3)
}

fn source_lines(text: &str) -> Vec<String> {
  if text.is_empty() {
    return vec![];
  }
  let mut lines: Vec<String> = text.replace("\r\n", "\n").split('\n').map(str::to_owned).collect();
  if lines.last().is_some_and(String::is_empty) {
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
}
