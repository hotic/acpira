//! Session links, mirroring src/shared/sessionLinks.ts: `acpira://session/<id>` is the canonical form tool results write; the
//! VS Code page copies a clickable `vscode://hotic.acpira/session/<id>` instead. Both sides run test/fixtures/session-links.json

pub const SESSION_LINK_PREFIX: &str = "acpira://session/";

const RESERVED: [&str; 4] = ["index", "prefs", "categories", "trash"];

/// A session id names files under the store root: ASCII letters, digits, `_` and `-` only
pub fn is_session_id(id: &str) -> bool {
  !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') && !RESERVED.contains(&id)
}

/// The canonical link for a session
pub fn session_link(id: &str) -> String {
  format!("{SESSION_LINK_PREFIX}{id}")
}

// Surrounding `<…>` or backticks are what models and chat clients wrap links in
fn unwrap(input: &str) -> &str {
  let s = input.trim();
  if s.len() >= 2 && ((s.starts_with('<') && s.ends_with('>')) || (s.starts_with('`') && s.ends_with('`'))) {
    return s[1..s.len() - 1].trim();
  }
  s
}

/// The session a link points at: `acpira://session/<id>`, or any `scheme://…/session/<id>` with "acpira" before the path's
/// end; query, fragment and one trailing slash are ignored. Bare ids are not links
pub fn session_id_of_link(href: &str) -> Option<String> {
  let mut s = unwrap(href);
  if let Some(cut) = s.find(['?', '#']) {
    s = &s[..cut];
  }
  let s = s.strip_suffix('/').unwrap_or(s);
  let (scheme, rest) = s.split_once("://")?;
  let mut chars = scheme.chars();
  if !chars.next().is_some_and(|c| c.is_ascii_alphabetic()) || !chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-')) {
    return None;
  }
  let scheme = scheme.to_ascii_lowercase();
  let (authority, path) = match rest.find('/') {
    Some(i) => (&rest[..i], &rest[i..]),
    None => (rest, ""),
  };
  if scheme == "acpira" && authority.eq_ignore_ascii_case("session") {
    let id = path.get(1..).unwrap_or("");
    return is_session_id(id).then(|| id.to_string());
  }
  // `/session/` must sit in the path, and "acpira" anywhere before it (scheme, authority or an earlier path segment)
  let at = path.rfind("/session/")?;
  let id = &path[at + "/session/".len()..];
  let before = format!("{scheme}://{authority}{}", &path[..at]).to_lowercase();
  (is_session_id(id) && before.contains("acpira")).then(|| id.to_string())
}

/// A session reference as a tool takes it: a link, else a bare id
pub fn session_ref_id(input: &str) -> Option<String> {
  let s = unwrap(input);
  session_id_of_link(s).or_else(|| is_session_id(s).then(|| s.to_string()))
}

#[cfg(test)]
mod tests {
  use super::*;

  /// The cases `test/sessionLinks.test.ts` runs through the TypeScript mirror: both sides must agree on every one
  #[test]
  fn parses_exactly_like_the_typescript_mirror() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!("../../../../test/fixtures/session-links.json")).unwrap();
    for case in fixture.as_array().unwrap() {
      let input = case["input"].as_str().unwrap();
      assert_eq!(session_id_of_link(input).as_deref(), case["link"].as_str(), "link {input:?}");
      assert_eq!(session_ref_id(input).as_deref(), case["ref"].as_str(), "ref {input:?}");
    }
  }
}
