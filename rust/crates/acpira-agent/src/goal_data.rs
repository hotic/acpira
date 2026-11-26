//! Pure session goal data types, tool specs, argument parsing, and state transitions.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::defs::{GOAL_CREATE, GOAL_GET, GOAL_UPDATE, now_ms};
use crate::llm::ToolSpec;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Status {
  Active,
  Paused,
  Blocked,
  /// The token budget is spent: the turn running then wraps up, and no other turn continues the goal or counts toward it
  BudgetLimited,
  Complete,
}

impl Status {
  /// The wire name (codex-acp's `status`)
  pub fn wire(self) -> &'static str {
    match self {
      Status::Active => "active",
      Status::Paused => "paused",
      Status::Blocked => "blocked",
      Status::BudgetLimited => "budgetLimited",
      Status::Complete => "complete",
    }
  }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Goal {
  pub objective: String,
  pub status: Status,
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub token_budget: Option<u64>,
  #[serde(default)]
  pub tokens_used: u64,
  /// Wall clock of the turns that ran while the goal was active
  #[serde(default)]
  pub time_used_ms: u64,
  #[serde(default)]
  pub created_at: u64,
  #[serde(default)]
  pub updated_at: u64,
  /// Continuation turns in a row that called no tool other than the goal's own
  #[serde(default)]
  pub idle_turns: u32,
}

impl Goal {
  pub fn new(objective: &str, budget: Option<u64>) -> Goal {
    let now = now_ms();
    Goal {
      objective: objective.trim().to_owned(),
      status: Status::Active,
      token_budget: budget,
      tokens_used: 0,
      time_used_ms: 0,
      created_at: now,
      updated_at: now,
      idle_turns: 0,
    }
  }

  pub fn remaining(&self) -> Option<u64> {
    self.token_budget.map(|b| b.saturating_sub(self.tokens_used))
  }

  /// The snapshot the client gets at `session_info_update._meta.goal`: codex-acp's fields
  pub fn snapshot(&self) -> Value {
    let mut v = json!({
      "objective": self.objective,
      "status": self.status.wire(),
      "tokensUsed": self.tokens_used,
      "timeUsedSeconds": self.time_used_ms / 1000,
      "updatedAt": self.updated_at,
    });
    if let Some(b) = self.token_budget {
      v["tokenBudget"] = json!(b);
    }
    v
  }
}

/// A goal tool's call, run by the turn itself (`goal::run_tool`): it changes the session, not the disk
#[derive(Debug, Clone, PartialEq)]
pub enum Call {
  Get,
  Create { objective: String, budget: Option<u64> },
  Update { status: Status },
}

pub fn spec(name: &str) -> Option<ToolSpec> {
  Some(match name {
    GOAL_GET => ToolSpec {
      name: GOAL_GET.into(),
      description: "Get this session's goal: its objective and status, the token budget and usage, and the time spent.".into(),
      parameters: json!({ "type": "object", "properties": {} }),
    },
    GOAL_CREATE => ToolSpec {
      name: GOAL_CREATE.into(),
      description: "Start a goal: an objective you keep working toward across turns until it is achieved. Only when the user \
explicitly asks for a goal, or to keep going until something is done; never infer one from an ordinary task. Fails while \
the session has an unfinished goal."
        .into(),
      parameters: json!({
        "type": "object",
        "properties": {
          "objective": { "type": "string", "description": "The concrete objective, in the user's terms" },
          "token_budget": { "type": "integer", "description": "A positive token budget, only when the user asks for one" },
        },
        "required": ["objective"],
      }),
    },
    GOAL_UPDATE => ToolSpec {
      name: GOAL_UPDATE.into(),
      description: "Mark the session's goal complete, blocked or paused. complete: only once an audit of the current state \
proves every requirement is met, never because the budget is nearly spent or you are stopping. blocked: only when the \
same blocker has stopped progress for three goal turns in a row and you cannot go on without the user or an outside \
change. paused: only when the user asks to pause it. Resuming and budget limits belong to the user and the system."
        .into(),
      parameters: json!({
        "type": "object",
        "properties": { "status": { "type": "string", "enum": ["complete", "blocked", "paused"] } },
        "required": ["status"],
      }),
    },
    _ => return None,
  })
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
  args.get(key).and_then(Value::as_str).ok_or_else(|| format!("Missing required string argument \"{key}\""))
}

fn num_arg(args: &Value, key: &str) -> Option<u64> {
  match args.get(key)? {
    Value::Number(n) => n.as_u64().or_else(|| n.as_f64().filter(|f| *f >= 0.0).map(|f| f as u64)),
    Value::String(s) => s.trim().parse().ok(),
    _ => None,
  }
}

pub fn prepare_call(name: &str, args: &Value) -> Result<Call, String> {
  Ok(match name {
    GOAL_GET => Call::Get,
    GOAL_CREATE => {
      let objective = str_arg(args, "objective")?.trim().to_owned();
      if objective.is_empty() {
        return Err("The objective must not be empty".into());
      }
      let budget = match args.get("token_budget") {
        None | Some(Value::Null) => None,
        Some(_) => Some(num_arg(args, "token_budget").filter(|b| *b > 0).ok_or("token_budget must be a positive whole number when given")?),
      };
      Call::Create { objective, budget }
    }
    _ => Call::Update {
      status: match str_arg(args, "status")? {
        "complete" => Status::Complete,
        "blocked" => Status::Blocked,
        "paused" => Status::Paused,
        other => {
          return Err(format!(
            "update_goal only marks the goal complete, blocked, or paused at the user's explicit request; \"{other}\" belongs to the user or the system"
          ));
        }
      },
    },
  })
}

/// What a reopened session tells the client: its goal while unfinished, else none
pub fn published(goal: Option<&Goal>) -> Option<&Goal> {
  goal.filter(|g| g.status != Status::Complete)
}

/// A goal read back by a new process: an active one comes back paused. True when it changed
pub fn revive(goal: &mut Option<Goal>) -> bool {
  match goal.as_mut() {
    Some(g) if g.status == Status::Active => {
      g.status = Status::Paused;
      g.updated_at = now_ms();
      true
    }
    _ => false,
  }
}

/// The goal after a rewind (`acp.rs` `Server::rewind`) to a turn that started with `then`, the session holding `now`
pub fn rewound(then: Option<Goal>, now: Option<&Goal>) -> Option<Goal> {
  let mut g = then?;
  if let Some(n) = now.filter(|n| n.created_at == g.created_at) {
    g.tokens_used = g.tokens_used.max(n.tokens_used);
    g.time_used_ms = g.time_used_ms.max(n.time_used_ms);
  }
  if g.status == Status::Active && now.is_none_or(|n| n.status != Status::Active) {
    g.status = Status::Paused;
  }
  if matches!(g.status, Status::Active | Status::Paused) && g.remaining() == Some(0) {
    g.status = Status::BudgetLimited;
  }
  g.updated_at = now_ms();
  Some(g)
}
