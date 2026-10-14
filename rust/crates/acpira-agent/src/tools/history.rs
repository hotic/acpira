//! `history_grep` and `history_list`: the folded history of chunk compaction (`chunk.rs`) read back from the session log
//! by a chunk's id, OpenCode's tools of the same names (Bkm016/opencode `tool/history.ts`). Offered while the settings
//! page picks chunk compaction; read-only and never asking, since they only read this session's own log

use serde_json::{Value, json};

use super::{Action, Ctx, Output, num_arg, str_arg, text_content};
use crate::budget::Keep;
use crate::llm::ToolSpec;

/// Hits a search returns at most, and lines a listing returns at most
const MAX_HITS: usize = 20;
const MAX_LINES: usize = 80;

pub fn grep_spec() -> ToolSpec {
  ToolSpec {
    name: super::HISTORY_GREP.into(),
    description: "Search the folded conversation history (the chunks of earlier turns) for a fixed string, not a regex. Returned line \
                  offsets are local to the chunk and can be passed to history_list."
      .into(),
    parameters: json!({
      "type": "object",
      "properties": {
        "pattern": { "type": "string", "description": "Fixed string to search for" },
        "chunk_id": { "type": "string", "description": "8-character chunk ID to search in; all chunks when absent" },
        "case_sensitive": { "type": "boolean", "description": "Default false" },
        "head_limit": { "type": "number", "description": "Hits to return (default and maximum 20)" },
      },
      "required": ["pattern"],
    }),
  }
}

pub fn list_spec() -> ToolSpec {
  ToolSpec {
    name: super::HISTORY_LIST.into(),
    description: "Read a folded chunk of the conversation history by line range: the user's messages, the assistant's replies, its tool \
                  calls and their output. Use history_grep first to find a line."
      .into(),
    parameters: json!({
      "type": "object",
      "properties": {
        "chunk_id": { "type": "string", "description": "8-character chunk ID" },
        "offset": { "type": "number", "description": "First line, local to the chunk (default 0)" },
        "limit": { "type": "number", "description": "Lines to return (default and maximum 80)" },
      },
      "required": ["chunk_id"],
    }),
  }
}

fn chunk_arg(args: &Value) -> Option<String> {
  args.get("chunk_id").and_then(Value::as_str).map(str::trim).filter(|c| !c.is_empty()).map(str::to_owned)
}

pub fn prepare_grep(args: &Value) -> Result<Action, String> {
  let pattern = str_arg(args, "pattern")?.to_owned();
  if pattern.is_empty() {
    return Err("The pattern is empty".into());
  }
  let case_sensitive = args.get("case_sensitive").and_then(Value::as_bool).unwrap_or(false);
  let limit = num_arg(args, "head_limit").map_or(MAX_HITS, |n| (n as usize).clamp(1, MAX_HITS));
  Ok(Action::HistoryGrep { pattern, chunk: chunk_arg(args), case_sensitive, limit })
}

pub fn prepare_list(args: &Value) -> Result<Action, String> {
  let chunk = chunk_arg(args).ok_or("Missing required string argument \"chunk_id\"")?;
  let offset = num_arg(args, "offset").unwrap_or(0) as usize;
  let limit = num_arg(args, "limit").map_or(MAX_LINES, |n| (n as usize).clamp(1, MAX_LINES));
  Ok(Action::HistoryList { chunk, offset, limit })
}

fn output(result: Result<String, String>, ctx: &Ctx) -> Output {
  match result {
    Ok(text) => {
      let model = super::budgeted(&text, Keep::Head, ctx);
      Output { content: vec![text_content(&model)], model, is_error: false, raw_output: None }
    }
    Err(e) => Output::error(e),
  }
}

pub fn grep(pattern: &str, chunk: Option<&str>, case_sensitive: bool, limit: usize, ctx: &Ctx) -> Output {
  output(crate::chunk::grep(&ctx.history, pattern, chunk, case_sensitive, limit), ctx)
}

pub fn list(chunk: &str, offset: usize, limit: usize, ctx: &Ctx) -> Output {
  output(crate::chunk::list(&ctx.history, chunk, offset, limit), ctx)
}
