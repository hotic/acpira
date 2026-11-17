//! The file a `write` / `edit` call names, read out of its arguments while they still stream. A long file takes the
//! model minutes to write, and until the call is whole its row would otherwise only say that it waits.

/// Reads a tool call's JSON arguments piece by piece and gives the first `path` string, at any depth, once it is whole.
/// Text inside another string never counts: only a string that follows a `path` key does
#[derive(Default)]
pub(super) struct PathScan {
  /// Inside a string: what it holds so far, escapes kept as written. Left empty for a string that cannot matter
  string: Option<String>,
  /// The string being read is some other key's value: its text is not kept
  skip: bool,
  escaped: bool,
  /// The last whole string, until the next token says whether it was a key
  last: Option<String>,
  /// The key whose value comes next
  key: Option<String>,
  found: bool,
}

impl PathScan {
  /// The path, the first time a piece completes it
  pub(super) fn feed(&mut self, piece: &str) -> Option<String> {
    if self.found {
      return None;
    }
    for c in piece.chars() {
      let Some(text) = &mut self.string else {
        match c {
          '"' => {
            self.skip = self.key.as_deref().is_some_and(|k| k != "path");
            self.string = Some(String::new());
          }
          ':' => self.key = self.last.take(),
          c if c.is_whitespace() => {}
          // Any other token: the string before it was a value, and a key's value that is not a string is passed over
          _ => {
            self.last = None;
            self.key = None;
          }
        }
        continue;
      };
      if self.escaped {
        self.escaped = false;
      } else if c == '\\' {
        self.escaped = true;
      } else if c == '"' {
        let text = self.string.take().unwrap_or_default();
        match self.key.take() {
          Some(key) if key == "path" => {
            // The escapes are JSON's, so JSON reads them back
            if let Ok(path) = serde_json::from_str::<String>(&format!("\"{text}\""))
              && !path.trim().is_empty()
            {
              self.found = true;
              return Some(path);
            }
          }
          Some(_) => {}
          None => self.last = Some(text),
        }
        continue;
      }
      if !self.skip {
        text.push(c);
      }
    }
    None
  }
}

#[cfg(test)]
mod tests {
  use super::PathScan;

  fn scan(pieces: &[&str]) -> Option<String> {
    let mut s = PathScan::default();
    pieces.iter().find_map(|p| s.feed(p))
  }

  #[test]
  fn the_path_is_read_before_the_content_ends() {
    assert_eq!(scan(&["{\"pa", "th\": \"src/a", ".rs\", \"content\": \"fn ma"]), Some("src/a.rs".into()));
  }

  #[test]
  fn a_path_written_inside_other_text_does_not_count() {
    assert_eq!(scan(&["{\"content\": \"{\\\"path\\\": \\\"wrong.rs\\\"}\", \"path\": \"right.rs\""]), Some("right.rs".into()));
  }

  #[test]
  fn a_nested_edit_gives_its_file() {
    assert_eq!(scan(&["{\"edits\": [{\"old_string\": \"a\", \"path\": \"dir/b.kt\", \"new_string\": \""]), Some("dir/b.kt".into()));
  }

  #[test]
  fn escapes_are_decoded_and_a_cut_string_waits() {
    assert_eq!(scan(&["{\"path\": \"C:\\\\dir\\\\\\u4e2d.txt"]), None);
    assert_eq!(scan(&["{\"path\": \"C:\\\\dir\\\\\\u4e2d.txt\""]), Some("C:\\dir\\中.txt".into()));
  }

  #[test]
  fn a_path_that_is_not_a_string_is_passed_over() {
    assert_eq!(scan(&["{\"path\": 3, \"paths\": [\"a\", \"b\"], \"content\": \"x\"}"]), None);
  }

  #[test]
  fn only_the_first_path_is_given() {
    let mut s = PathScan::default();
    assert_eq!(s.feed("{\"path\": \"a\","), Some("a".into()));
    assert_eq!(s.feed("\"edits\": [{\"path\": \"b\"}]}"), None);
  }
}
