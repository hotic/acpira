//! Ranking behind the @ file search (mirror of src/host/fileRank.ts): subsequence match, best first

use acpira_shared::protocol::FileHit;

pub fn rank_files(files: &[FileHit], query: &str, limit: usize) -> Vec<FileHit> {
  let q = query.trim().to_lowercase();
  if q.is_empty() {
    return files.iter().take(limit).cloned().collect();
  }
  let mut scored: Vec<(f64, usize)> =
    files.iter().enumerate().filter_map(|(i, f)| score(&f.path.to_lowercase(), &q).map(|s| (s, i))).collect();
  // Stable, best first (Array.prototype.sort is stable)
  scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
  scored.into_iter().take(limit).map(|(_, i)| files[i].clone()).collect()
}

fn score(path: &str, q: &str) -> Option<f64> {
  let chars: Vec<char> = path.chars().collect();
  let base = chars.iter().rposition(|c| *c == '/').map(|i| i + 1).unwrap_or(0);
  let (mut from, mut prev, mut s) = (0usize, -2i64, 0f64);
  for ch in q.chars() {
    let i = (from..chars.len()).find(|j| chars[*j] == ch)?;
    let word_start = i == 0 || "/._-".contains(chars[i - 1]);
    s += 1.0 + if i as i64 == prev + 1 { 2.0 } else { 0.0 } + if i >= base { 2.0 } else { 0.0 } + if word_start { 3.0 } else { 0.0 };
    prev = i as i64;
    from = i + 1;
  }
  Some(s - (path.encode_utf16().count().min(500) as f64) / 1000.0)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn file_name_matches_rank_first() {
    let hit = |p: &str| FileHit { uri: format!("file:///{p}"), path: p.into() };
    let files = vec![hit("src/components/alpha.ts"), hit("src/alpha.ts"), hit("docs/beta.md")];
    let r = rank_files(&files, "alpha", 10);
    assert_eq!(r[0].path, "src/alpha.ts");
    assert_eq!(r.len(), 2);
  }
}
