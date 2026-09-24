//! Context-length error recognition (mirror of src/shared/turnErrors.ts)

use std::sync::LazyLock;

use regex::Regex;

use crate::transcript::TurnError;

static TOO_LONG: LazyLock<Regex> = LazyLock::new(|| {
  Regex::new(r"(?i)\bprompt (?:to the model )?(?:was|is) too long\b|\bcontext (?:length|window) (?:exceeded|limit exceeded)\b|\bmaximum context length\b").unwrap()
});

pub fn is_context_length_error(error: Option<&TurnError>) -> bool {
  let Some(e) = error else { return false };
  matches!(e.kind.as_deref(), Some("context_length_exceeded" | "context_window_exceeded" | "prompt_too_long"))
    || TOO_LONG.is_match(&e.message)
}
