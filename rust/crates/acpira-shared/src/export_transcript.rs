//! Transcript → Markdown for reading (mirror of src/shared/exportTranscript.ts)

use crate::transcript::*;

pub struct ExportLabels {
  pub user: String,
  pub agent: String,
  pub project: String,
  pub exported: String,
  pub attachments: String,
  pub thinking: String,
  pub compacted: String,
  pub auto_compact: String,
  pub error: String,
}

pub struct ExportInput<'a> {
  pub title: &'a str,
  pub agent_name: &'a str,
  pub cwd: &'a str,
  pub exported_at: &'a str,
  pub turns: &'a [Turn],
}

const TOOL_CONTENT_MAX: usize = 4000;

fn slice16(s: &str, n: usize) -> String {
  let mut units = 0;
  let mut out = String::new();
  for c in s.chars() {
    units += c.len_utf16();
    if units > n {
      break;
    }
    out.push(c);
  }
  out
}

pub fn export_markdown(input: &ExportInput, labels: &ExportLabels, blob_path: &dyn Fn(&str) -> Option<String>) -> String {
  let mut parts = vec![
    format!("# {}", input.title),
    format!(
      "- **{}**: {}\n- **{}**: {}\n- **{}**: {}",
      labels.agent, input.agent_name, labels.project, input.cwd, labels.exported, input.exported_at
    ),
    "---".to_owned(),
  ];
  for turn in input.turns {
    let rendered = match turn {
      Turn::User(u) => user_turn(u, labels),
      Turn::Agent(a) => agent_turn(a, input.agent_name, labels, blob_path),
    };
    if !rendered.is_empty() {
      parts.push(rendered);
    }
  }
  format!("{}\n", parts.join("\n\n"))
}

fn user_turn(turn: &UserTurn, labels: &ExportLabels) -> String {
  if turn.auto == Some(true) {
    return format!("_{}_", labels.auto_compact);
  }
  let mut out = vec![format!("### {}", labels.user), String::new(), turn.text.clone()];
  if let Some(a) = turn.attachments.as_ref().filter(|a| !a.is_empty()) {
    let names: Vec<String> = a
      .iter()
      .map(|x| match x {
        Attachment::File { name, .. } => name.clone(),
        Attachment::Image { name, blob, .. } => name.clone().or_else(|| blob.clone()).unwrap_or_else(|| "image".into()),
        Attachment::Text { name, .. } => name.clone(),
      })
      .collect();
    out.push(String::new());
    out.push(format!("> {}: {}", labels.attachments, names.join(", ")));
  }
  out.join("\n")
}

fn stop_name(s: TurnStop) -> String {
  serde_json::to_value(s).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
}

fn agent_turn(turn: &AgentTurn, agent_name: &str, labels: &ExportLabels, blob_path: &dyn Fn(&str) -> Option<String>) -> String {
  let mut out = vec![format!("### {agent_name}")];
  for b in &turn.blocks {
    if let Some(r) = block(b, labels, blob_path) {
      out.push(String::new());
      out.push(r);
    }
  }
  match (&turn.stop, &turn.error) {
    (Some(TurnStop::Error), Some(e)) => {
      out.push(String::new());
      out.push(format!("> {}: {}", labels.error, e.message));
    }
    (Some(s), _) if *s != TurnStop::EndTurn => {
      out.push(String::new());
      out.push(format!("> {}", stop_name(*s)));
    }
    _ => {}
  }
  out.join("\n")
}

fn block(b: &AgentBlock, labels: &ExportLabels, blob_path: &dyn Fn(&str) -> Option<String>) -> Option<String> {
  Some(match b {
    AgentBlock::Thought(t) => format!("<details><summary>{}</summary>\n\n{}\n\n</details>", labels.thinking, t.text),
    AgentBlock::Text(t) => t.markdown.clone(),
    AgentBlock::Image(i) => image_line(i.blob.as_deref(), &i.mime_type, i.uri.as_deref(), blob_path),
    AgentBlock::Plan(p) => p
      .entries
      .iter()
      .map(|e| match e.status {
        PlanStatus::Completed => format!("- [x] {}", e.title),
        PlanStatus::InProgress => format!("- [ ] {} _(in progress)_", e.title),
        PlanStatus::Pending => format!("- [ ] {}", e.title),
      })
      .collect::<Vec<_>>()
      .join("\n"),
    AgentBlock::ToolCall(t) => {
      let head = format!(
        "- **{}**{}{}",
        t.verb,
        t.target.as_ref().filter(|x| !x.is_empty()).map(|x| format!(" `{x}`")).unwrap_or_default(),
        t.meta.as_ref().filter(|x| !x.is_empty()).map(|x| format!(" ({x})")).unwrap_or_default()
      );
      let items: Vec<&ToolContent> = match (&t.contents, &t.content) {
        (Some(l), _) => l.iter().collect(),
        (None, Some(c)) => vec![c],
        _ => vec![],
      };
      let content: Vec<String> = items.into_iter().filter_map(|c| tool_content(c, blob_path)).filter(|s| !s.is_empty()).collect();
      if content.is_empty() { head } else { format!("{head}\n{}", content.join("\n")) }
    }
    AgentBlock::Permission(_) => return None,
    AgentBlock::Question(q) => {
      q.outcome?;
      q.questions
        .iter()
        .map(|x| {
          let answer = match q.answers.as_ref().and_then(|a| a.get(&x.id)) {
            Some(serde_json::Value::Array(items)) => items.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>().join(", "),
            Some(serde_json::Value::String(s)) => s.clone(),
            _ => "—".into(),
          };
          format!("**Q:** {}\n\n**A:** {answer}", x.text)
        })
        .collect::<Vec<_>>()
        .join("\n\n")
    }
    AgentBlock::Compaction(c) => {
      if c.status == CompactionStatus::Completed {
        format!("_{}_", labels.compacted)
      } else {
        let st = serde_json::to_value(c.status).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default();
        format!("_{}: {st}_", labels.compacted)
      }
    }
    AgentBlock::PlanDocument(p) => format!("#### {}\n\n{}", p.title, p.markdown),
    AgentBlock::Notice(n) => format!(
      "> {}: {}{}",
      if n.severity == Severity::Error { labels.error.as_str() } else { "Notice" },
      n.title,
      n.details.as_ref().filter(|d| !d.is_empty()).map(|d| format!("\n> {d}")).unwrap_or_default()
    ),
  })
}

fn image_line(blob: Option<&str>, mime: &str, uri: Option<&str>, blob_path: &dyn Fn(&str) -> Option<String>) -> String {
  if let Some(path) = blob.filter(|b| !b.is_empty()).and_then(blob_path) {
    return format!("![image]({path})");
  }
  match uri.filter(|u| !u.is_empty()) {
    Some(u) => format!("[image]({u})"),
    None => format!("[image: {mime}]"),
  }
}

fn tool_content(c: &ToolContent, blob_path: &dyn Fn(&str) -> Option<String>) -> Option<String> {
  Some(match c {
    ToolContent::Text { text } => {
      let body = if text.encode_utf16().count() > TOOL_CONTENT_MAX {
        format!("{}\n… (truncated)", slice16(text, TOOL_CONTENT_MAX))
      } else {
        text.clone()
      };
      format!("```\n{body}\n```")
    }
    ToolContent::Diff { lines, .. } => {
      let body: Vec<String> =
        lines.iter().map(|l| if l.kind == DiffKind::Hunk { format!("@@ {} @@", l.text) } else { l.text.clone() }).collect();
      format!("```diff\n{}\n```", body.join("\n"))
    }
    ToolContent::List { items } => items.iter().map(|i| format!("  - {i}")).collect::<Vec<_>>().join("\n"),
    ToolContent::Image(i) => image_line(i.blob.as_deref(), &i.mime_type, i.uri.as_deref(), blob_path),
  })
}

/// `<slug>-<yyyyMMdd-HHmmss>.<md|json>`; `stamp` is the local-time part, formatted by the host
pub fn export_file_name(title: &str, markdown: bool, stamp: &str) -> String {
  let cleaned: String = title.chars().filter(|c| !"/\\:*?\"<>|".contains(*c) && !c.is_control()).collect();
  let dashed = cleaned.split_whitespace().collect::<Vec<_>>().join("-");
  let mut dashed = dashed;
  // A leading / trailing whitespace run collapsed to a dash in the TS; split_whitespace drops it, then trim separators
  let trimmed = dashed.trim_matches(|c| c == '-' || c == '.').to_owned();
  dashed = trimmed;
  let capped: String = dashed.chars().take(60).collect();
  let slug = capped.trim_end_matches(['-', '.']);
  let slug = if slug.is_empty() { "session" } else { slug };
  format!("{slug}-{stamp}.{}", if markdown { "md" } else { "json" })
}
