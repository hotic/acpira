//! Regular expressions written in the source. A pattern here is a literal, so a typo is a bug of this build, not of any
//! input: the one place that may panic on it is `compile`, which every `LazyLock<Regex>` in the engine goes through

use regex::Regex;

/// A literal pattern, compiled once (inside a `LazyLock`). Panics on an invalid pattern, which the tests that touch the
/// pattern catch on the first run
#[expect(clippy::expect_used, reason = "the pattern is a source literal; an invalid one is a bug the tests catch")]
pub fn compile(pattern: &'static str) -> Regex {
  Regex::new(pattern).expect("a valid literal pattern")
}

#[cfg(test)]
mod tests {
  #[test]
  fn compiles_a_literal() {
    assert!(super::compile(r"^a+$").is_match("aaa"));
  }
}
