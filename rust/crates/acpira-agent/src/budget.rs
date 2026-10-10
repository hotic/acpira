//! The tool output budget: what a tool returns goes into the model's context only up to a limit. Above it the full text
//! is saved under the session's `outputs/` directory and the model gets a preview, what was cut, and the path to read
//! the rest from. Only the model's copy is cut; the reader sees the whole output on the tool card. Limits follow
//! OpenCode (`tool/truncate.ts`: 2000 lines, 50 KB)

use std::path::Path;

pub const MAX_LINES: usize = 2000;
pub const MAX_BYTES: usize = 50 * 1024;

/// What a preview keeps
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keep {
  /// The beginning (file contents, listings)
  Head,
  /// The beginning and the end (command output: the error is usually last)
  HeadTail,
}

/// The text fitted to the budget; `name` names the spill file (a tool call id)
pub fn fit(text: &str, keep: Keep, dir: &Path, name: &str) -> String {
  let lines = text.lines().count();
  if lines <= MAX_LINES && text.len() <= MAX_BYTES {
    return text.to_owned();
  }
  let file = dir.join(format!("{}.txt", safe_name(name)));
  let saved = std::fs::create_dir_all(dir).and_then(|_| std::fs::write(&file, text)).is_ok();
  let preview = match keep {
    Keep::Head => head(text, MAX_LINES, MAX_BYTES),
    Keep::HeadTail => {
      let h = head(text, MAX_LINES / 2, MAX_BYTES / 2);
      let t = tail(text, MAX_LINES / 2, MAX_BYTES / 2);
      let omitted = lines.saturating_sub(h.lines().count() + t.lines().count());
      format!("{h}\n… {omitted} lines omitted …\n{t}")
    }
  };
  let note = if saved {
    format!(
      "[Output truncated: {lines} lines, {} bytes in total. The full output is in {}; read it with the read tool (offset / limit) instead of re-running the command.]",
      text.len(),
      file.display()
    )
  } else {
    format!("[Output truncated: {lines} lines, {} bytes in total; the full output could not be saved.]", text.len())
  };
  format!("{preview}\n\n{note}")
}

fn safe_name(name: &str) -> String {
  name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).collect()
}

/// The first lines of `text` within both limits
pub fn head(text: &str, max_lines: usize, max_bytes: usize) -> String {
  let mut out = String::new();
  for (n, line) in text.lines().enumerate() {
    if n == 0 && line.len() + 1 > max_bytes {
      // One huge line (minified JSON, a data URL): keep its start
      return cut(line, max_bytes).to_owned();
    }
    if n >= max_lines || out.len() + line.len() + 1 > max_bytes {
      break;
    }
    if n > 0 {
      out.push('\n');
    }
    out.push_str(line);
  }
  out
}

/// The last lines of `text` within both limits
pub fn tail(text: &str, max_lines: usize, max_bytes: usize) -> String {
  let mut kept: Vec<&str> = vec![];
  let mut bytes = 0;
  for line in text.lines().rev() {
    if kept.is_empty() && line.len() + 1 > max_bytes {
      let mut start = line.len() - max_bytes.min(line.len());
      while !line.is_char_boundary(start) {
        start += 1;
      }
      return line[start..].to_owned();
    }
    if kept.len() >= max_lines || bytes + line.len() + 1 > max_bytes {
      break;
    }
    bytes += line.len() + 1;
    kept.push(line);
  }
  kept.reverse();
  kept.join("\n")
}

/// The longest prefix of `s` within `max` bytes, on a char boundary
pub fn cut(s: &str, max: usize) -> &str {
  if s.len() <= max {
    return s;
  }
  let mut end = max;
  while !s.is_char_boundary(end) {
    end -= 1;
  }
  &s[..end]
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn small_output_passes_through() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(fit("a\nb", Keep::HeadTail, dir.path(), "x"), "a\nb");
    assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none());
  }

  #[test]
  fn big_command_output_keeps_head_and_tail_and_spills_the_whole() {
    let dir = tempfile::tempdir().unwrap();
    let text: String = (1..=5000).map(|n| format!("line {n}\n")).collect();
    let out = fit(&text, Keep::HeadTail, dir.path(), "call/1");
    assert!(out.starts_with("line 1\n"));
    assert!(out.contains("line 5000"));
    assert!(!out.contains("line 2500\n"));
    assert!(out.contains("lines omitted"));
    let saved = dir.path().join("call_1.txt");
    assert!(out.contains(&saved.display().to_string()));
    assert_eq!(std::fs::read_to_string(saved).unwrap(), text);
  }

  #[test]
  fn long_lines_hit_the_byte_limit() {
    let dir = tempfile::tempdir().unwrap();
    let text = "x".repeat(1000) + "\n";
    let out = fit(&text.repeat(100), Keep::Head, dir.path(), "y");
    assert!(out.len() < MAX_BYTES + 500);
    let one = "é".repeat(60_000);
    let out = fit(&one, Keep::HeadTail, dir.path(), "z");
    assert!(out.len() < MAX_BYTES + 500 && out.starts_with('é'));
  }
}
