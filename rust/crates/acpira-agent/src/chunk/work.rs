//! The work-state index of a running turn's folded steps, built from its tool calls without a model

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use super::calls::{arg, calls, exit_code, failed, file_changes, own_result, rel, then_run_output};
use super::size::{one_line, preview, preview_lines, tokens};
use crate::llm::Item;

/// A changed file in the work-state index
struct FileEntry {
  count: usize,
  first: usize,
  last: usize,
  /// The last three changes: (step, changed lines)
  diffs: Vec<(usize, Vec<String>)>,
}

/// A failed call, and the later step that fixed it (the same file written, the same command run without an error)
struct ErrorEntry {
  step: usize,
  key: String,
  what: String,
  detail: String,
  resolved: Option<usize>,
}

struct CommandEntry {
  step: usize,
  command: String,
  exit: Option<i64>,
  lines: usize,
  tail: Vec<String>,
}

/// The index of a running turn's folded steps, built from its tool calls without a model
#[derive(Default)]
struct WorkIndex {
  steps: usize,
  files: BTreeMap<String, FileEntry>,
  file_order: Vec<String>,
  errors: Vec<ErrorEntry>,
  narration: Vec<String>,
  reads: Vec<(String, Vec<usize>)>,
  /// (key, steps, first line of the latest output)
  searches: Vec<(String, Vec<usize>, String)>,
  commands: Vec<CommandEntry>,
  other: Vec<(String, Vec<usize>)>,
}

fn push_step(list: &mut Vec<(String, Vec<usize>)>, key: String, step: usize) {
  match list.iter_mut().find(|(k, _)| *k == key) {
    Some((_, steps)) => steps.push(step),
    None => list.push((key, vec![step])),
  }
}

fn work_index(region: &[Item], cwd: &Path) -> WorkIndex {
  let mut ix = WorkIndex::default();
  let all = calls(region);
  let mut step_of: HashMap<usize, usize> = HashMap::new();
  for (i, item) in region.iter().enumerate() {
    if let Item::Assistant { text, .. } = item {
      ix.steps += 1;
      step_of.insert(i, ix.steps);
      if !text.trim().is_empty() {
        ix.narration.push(format!("- {}. {}", ix.steps, one_line(text, 60)));
      }
    }
  }
  for c in &all {
    let step = step_of[&c.at];
    let key = match (arg(&c.args, &["path", "file_path"]), arg(&c.args, &["command", "cmd"])) {
      (Some(p), _) if c.name == crate::tools::WRITE || c.name == crate::tools::EDIT => format!("file:{}", rel(p, cwd)),
      (_, Some(cmd)) => format!("command:{cmd}"),
      _ => format!("{}:{}", c.name, c.raw_args),
    };
    if failed(c) {
      let what = match arg(&c.args, &["command", "cmd"]) {
        Some(cmd) => format!("{} `{}`", c.name, one_line(cmd, 40)),
        None => format!("{} {}", c.name, key.strip_prefix("file:").map(str::to_owned).unwrap_or_else(|| one_line(c.raw_args, 40))),
      };
      let raw = c.result.map_or("", |(o, _)| o).trim();
      let detail = preview(&raw.lines().take(8).collect::<Vec<_>>().join("\n"), 200);
      ix.errors.push(ErrorEntry { step, key, what, detail, resolved: None });
      continue;
    }
    if c.result.is_none() {
      continue;
    }
    for e in ix.errors.iter_mut().filter(|e| e.resolved.is_none() && e.key == key) {
      e.resolved = Some(step);
    }
    match c.name {
      crate::tools::WRITE | crate::tools::EDIT => {
        for (file, lines) in file_changes(c, cwd) {
          if !ix.files.contains_key(&file) {
            ix.file_order.push(file.clone());
          }
          let entry = ix.files.entry(file).or_insert(FileEntry { count: 0, first: step, last: step, diffs: vec![] });
          entry.count += 1;
          entry.last = step;
          entry.diffs.push((step, lines));
          if entry.diffs.len() > 3 {
            entry.diffs.remove(0);
          }
        }
        // The command the change ran (`then_run`) counts like a bash call of its own
        if let Some(command) = arg(&c.args, &["then_run"]).map(str::trim)
          && let Some(out) = then_run_output(c.result.map_or("", |(o, _)| o), command)
        {
          let key = format!("command:{command}");
          match exit_code(out) {
            Some(code) if code != 0 => {
              let detail = preview(&out.trim().lines().take(8).collect::<Vec<_>>().join("\n"), 200);
              ix.errors.push(ErrorEntry { step, key, what: format!("then_run `{}`", one_line(command, 40)), detail, resolved: None });
            }
            _ => {
              for e in ix.errors.iter_mut().filter(|e| e.resolved.is_none() && e.key == key) {
                e.resolved = Some(step);
              }
            }
          }
          ix.commands.push(command_entry(step, command, out));
        }
      }
      crate::tools::READ => {
        for p in crate::tools::read::targets(&c.args, cwd) {
          push_step(&mut ix.reads, crate::tools::shown(&p, cwd), step);
        }
      }
      crate::tools::GREP | crate::tools::GLOB | crate::tools::LIST => {
        let target = arg(&c.args, &["path"]).map(|p| format!(" in {}", rel(p, cwd))).unwrap_or_default();
        let include = arg(&c.args, &["include"]).map(|i| format!(" ({i})")).unwrap_or_default();
        let key = one_line(&format!("{} {}{target}{include}", c.name, arg(&c.args, &["pattern"]).unwrap_or("")), 60);
        let head = one_line(own_result(c.result.map_or("", |(o, _)| o)).trim().lines().next().unwrap_or(""), 25);
        match ix.searches.iter_mut().find(|s| s.0 == key) {
          Some(s) => {
            s.1.push(step);
            s.2 = head;
          }
          None => ix.searches.push((key, vec![step], head)),
        }
      }
      crate::tools::BASH if arg(&c.args, &["command", "cmd"]).is_some() => {
        ix.commands.push(command_entry(step, arg(&c.args, &["command", "cmd"]).unwrap_or(""), own_result(c.result.map_or("", |(o, _)| o))));
      }
      _ => push_step(&mut ix.other, format!("{} {}", c.name, one_line(c.raw_args, 40)), step),
    }
  }
  ix
}

/// A command's line in the index: its exit code, how many lines it printed and the last few
fn command_entry(step: usize, command: &str, output: &str) -> CommandEntry {
  let out = output.trim();
  let lines: Vec<&str> = out.lines().filter(|l| !l.starts_with("[exit code ")).collect();
  let tail = lines[lines.len().saturating_sub(3)..].iter().map(|l| one_line(l, 40)).collect();
  CommandEntry { step, command: one_line(command, 40), exit: exit_code(out), lines: lines.len(), tail }
}

fn step_list(steps: &[usize]) -> String {
  let list = steps.iter().map(usize::to_string).collect::<Vec<_>>().join(",");
  if steps.len() > 1 { format!("steps {list}; {} times", steps.len()) } else { format!("step {list}") }
}

/// How much of the index is spelled out: narration and list budgets in tokens (None: all), files with their diffs
#[derive(Clone, Copy)]
struct Fit {
  narration: Option<u64>,
  lists: Option<u64>,
  diff_files: usize,
}

fn render_work(ix: &WorkIndex, id: &str, fit: Fit) -> String {
  let cap = |lines: Vec<String>, budget: Option<u64>| -> Vec<String> {
    match budget {
      Some(b) if !lines.is_empty() => preview_lines(&lines, b).split('\n').map(str::to_owned).collect(),
      _ => lines,
    }
  };
  let mut out = vec![
    format!("<work-state chunk=\"{id}\" status=\"partial\" folded-steps=\"1-{}\">", ix.steps),
    // OpenCode found that models answer from memory about folded output unless told it is gone
    format!(
      "This turn is still in progress. Steps 1-{} were folded to free context: their tool outputs (file contents, search hits, command output) are no longer in your context, only this index is. The messages after it are the later steps, verbatim. Continue the current task from the latest step and do not redo folded steps. Do not answer from memory about folded output: when you need its exact text, call history_grep or history_list with chunk_id=\"{id}\", or run the tool again.",
      ix.steps
    ),
    String::new(),
    format!("## Modified files ({})", ix.file_order.len()),
  ];
  for (k, file) in ix.file_order.iter().enumerate() {
    let f = &ix.files[file];
    let span = if f.last != f.first { format!("{}-{}", f.first, f.last) } else { f.first.to_string() };
    out.push(format!("- {file}: {} change{} (step {span})", f.count, if f.count > 1 { "s" } else { "" }));
    // The list itself is never shortened; only the most recently changed files keep their diffs when it is tight.
    // Counted from the end: `k + diff_files` overflowed at `usize::MAX` (all diffs) from the second file on
    if ix.file_order.len() - k <= fit.diff_files {
      for (step, lines) in &f.diffs {
        out.push(format!("  step {step}:"));
        out.extend(lines.iter().map(|l| format!("    {l}")));
      }
    }
  }
  out.push(String::new());
  out.push(format!("## Errors ({})", ix.errors.len()));
  for e in &ix.errors {
    let detail = e.detail.replace('\n', "\n    ");
    let (step, what) = (e.step, &e.what);
    out.push(match e.resolved {
      Some(r) if fit.lists.is_some() => format!("- step {step} {what} (resolved at step {r})"),
      Some(r) => format!("- step {step} {what} (resolved at step {r}): {detail}"),
      None => format!("- step {step} {what} (UNRESOLVED): {detail}"),
    });
  }
  out.push(String::new());
  out.push("## Steps".to_owned());
  out.extend(cap(ix.narration.clone(), fit.narration));
  out.push(String::new());
  out.push(format!("## Files read ({}; contents folded, read again or use history_grep before relying on them)", ix.reads.len()));
  out.extend(cap(ix.reads.iter().map(|(f, s)| format!("- {f} @ {}", s.iter().map(usize::to_string).collect::<Vec<_>>().join(","))).collect(), fit.lists));
  out.push(String::new());
  out.push(format!("## Searches ({})", ix.searches.len()));
  out.extend(cap(ix.searches.iter().map(|(k, s, h)| format!("- {k} → {} ({})", if h.is_empty() { "(no output)" } else { h }, step_list(s))).collect(), fit.lists));
  out.push(String::new());
  out.push(format!("## Commands ({})", ix.commands.len()));
  out.extend(cap(
    ix.commands
      .iter()
      .map(|c| {
        let mut s = format!("- {}. `{}` exit={}, {} output lines, last {}:", c.step, c.command, c.exit.map_or("?".to_owned(), |e| e.to_string()), c.lines, c.tail.len());
        for l in &c.tail {
          s.push_str(&format!("\n    {l}"));
        }
        s
      })
      .collect(),
    fit.lists,
  ));
  if !ix.other.is_empty() {
    out.push(String::new());
    out.push(format!("## Other tool calls ({})", ix.other.len()));
    out.extend(cap(ix.other.iter().map(|(k, s)| format!("- {k} ({})", step_list(s))).collect(), fit.lists));
  }
  out.push("</work-state>".to_owned());
  out.join("\n")
}

/// The work-state index within `max` tokens: narration shortened first, then diffs kept for the 20 / 10 / 5 files
/// changed last, then the lists of reads, searches and commands; the list of changed files is never shortened
pub(super) fn work_state(region: &[Item], id: &str, cwd: &Path, max: u64) -> String {
  let ix = work_index(region, cwd);
  let all = Fit { narration: None, lists: None, diff_files: usize::MAX };
  let full = render_work(&ix, id, all);
  if tokens(&full) <= max {
    return full;
  }
  let bare = WorkIndex { narration: vec![], ..work_index(region, cwd) };
  for diff_files in [usize::MAX, 20, 10, 5] {
    let fixed = tokens(&render_work(&bare, id, Fit { diff_files, ..all }));
    if fixed > max * 7 / 10 {
      continue;
    }
    return render_work(&ix, id, Fit { narration: Some((max - fixed).max(500)), lists: None, diff_files });
  }
  let lists = (max / 10).max(300);
  let fixed = tokens(&render_work(&bare, id, Fit { narration: None, lists: Some(lists), diff_files: 0 }));
  render_work(&ix, id, Fit { narration: Some(max.saturating_sub(fixed).max(500)), lists: Some(lists), diff_files: 0 })
}
