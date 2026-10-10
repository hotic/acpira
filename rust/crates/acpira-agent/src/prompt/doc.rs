//! A prompt file: optional frontmatter (`key: value` lines, values plain or `[a, "b"]` lists), a preamble, then
//! sections under second-level headings. Overlaying one document on another replaces sections by heading, appends new
//! ones and drops a section given with an empty body; headings inside fenced code do not split

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Doc {
  /// Frontmatter entries in file order
  pub meta: Vec<(String, Meta)>,
  pub preamble: String,
  /// (heading without `## `, body)
  pub sections: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Meta {
  Text(String),
  List(Vec<String>),
}

impl Doc {
  pub fn parse(text: &str) -> Doc {
    let text = text.replace("\r\n", "\n");
    let (meta, body) = match text.strip_prefix("---\n").and_then(|rest| rest.split_once("\n---\n").or_else(|| rest.strip_suffix("\n---").map(|m| (m, "")))) {
      Some((front, body)) => (parse_meta(front), body.to_owned()),
      None => (vec![], text),
    };
    let mut doc = Doc { meta, ..Default::default() };
    let mut current: Option<(String, String)> = None;
    let mut fence = false;
    for line in body.split_inclusive('\n') {
      if line.trim_start().starts_with("```") {
        fence = !fence;
      }
      if !fence && let Some(h) = line.strip_prefix("## ") {
        if let Some(s) = current.take() {
          doc.sections.push(s);
        }
        current = Some((h.trim().to_owned(), String::new()));
        continue;
      }
      match &mut current {
        Some((_, b)) => b.push_str(line),
        None => doc.preamble.push_str(line),
      }
    }
    doc.sections.extend(current);
    doc
  }

  pub fn text(&self, key: &str) -> Option<&str> {
    self.meta.iter().rev().find(|(k, _)| k == key).and_then(|(_, v)| match v {
      Meta::Text(t) => Some(t.as_str()),
      Meta::List(_) => None,
    })
  }

  /// A list entry; a plain value counts as a list of one
  pub fn list(&self, key: &str) -> Vec<String> {
    match self.meta.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v) {
      Some(Meta::List(l)) => l.clone(),
      Some(Meta::Text(t)) if !t.is_empty() => vec![t.clone()],
      _ => vec![],
    }
  }

  /// Lay `top` over this document: its preamble (when it has one), sections and frontmatter win
  pub fn overlay(&mut self, top: &Doc) {
    if !top.preamble.trim().is_empty() {
      self.preamble = top.preamble.clone();
    }
    for (heading, body) in &top.sections {
      let at = self.sections.iter().position(|(h, _)| h.eq_ignore_ascii_case(heading));
      match at {
        Some(i) if body.trim().is_empty() => {
          self.sections.remove(i);
        }
        Some(i) => self.sections[i].1 = body.clone(),
        None if !body.trim().is_empty() => self.sections.push((heading.clone(), body.clone())),
        None => {}
      }
    }
    self.meta.extend(top.meta.iter().cloned());
  }

  /// The document as prompt text (no frontmatter)
  pub fn render(&self) -> String {
    let mut parts = vec![];
    if !self.preamble.trim().is_empty() {
      parts.push(self.preamble.trim().to_owned());
    }
    for (h, body) in &self.sections {
      parts.push(format!("## {h}\n{}", body.trim()));
    }
    parts.join("\n\n")
  }
}

fn parse_meta(front: &str) -> Vec<(String, Meta)> {
  front
    .lines()
    .filter_map(|line| {
      let (k, v) = line.split_once(':')?;
      let (k, v) = (k.trim(), v.trim());
      if k.is_empty() || k.starts_with('#') {
        return None;
      }
      let value = match v.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
        Some(inner) => Meta::List(inner.split(',').map(unquote).filter(|s| !s.is_empty()).collect()),
        None => Meta::Text(unquote(v)),
      };
      Some((k.to_owned(), value))
    })
    .collect()
}

fn unquote(s: &str) -> String {
  let s = s.trim();
  s.strip_prefix('"').and_then(|r| r.strip_suffix('"')).or_else(|| s.strip_prefix('\'').and_then(|r| r.strip_suffix('\''))).unwrap_or(s).to_owned()
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn frontmatter_sections_and_overlay() {
    let base = Doc::parse("---\nname: base\nversion: 2\n---\nIntro.\n\n## Working\n- a\n\n## Tools\n```\n## not a heading\n```\n\n## Answering\n- c\n");
    assert_eq!((base.text("name"), base.text("version")), (Some("base"), Some("2")));
    assert_eq!(base.sections.iter().map(|(h, _)| h.as_str()).collect::<Vec<_>>(), ["Working", "Tools", "Answering"]);
    let top = Doc::parse("---\nmatch: [\"deepseek/*\", '*deepseek*']\n---\n## tools\n- replaced\n## Answering\n\n## Extra\n- new\n");
    assert_eq!(top.list("match"), ["deepseek/*", "*deepseek*"]);
    let mut merged = base.clone();
    merged.overlay(&top);
    assert_eq!(merged.render(), "Intro.\n\n## Working\n- a\n\n## Tools\n- replaced\n\n## Extra\n- new");
    assert_eq!(Doc::parse("plain text").preamble, "plain text");
  }
}
