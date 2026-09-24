//! `acpira agents [--json]`: the built-in agents as this host launches them. The plain form lists where each CLI was found; `--json`
//! is what the repository probes read, so they spawn and initialize an agent exactly as the sidecar would

use serde_json::{Value, json};

use crate::acp::agent_process::initialize_request;
use crate::acp::agent_registry::AgentRegistry;
use crate::acp::launch::{Os, ProcessEnv, spawn_spec};

pub async fn run(args: &[String]) -> i32 {
  let registry = AgentRegistry::new(&Value::Null);
  let mut agents = vec![];
  for id in registry.ids().to_vec() {
    let Ok(def) = registry.get(&id) else { continue };
    let binary = registry.resolve_binary(&id).await;
    let spawn = binary.as_deref().map(|b| {
      let s = spawn_spec(b, &def.args, Os::current(), &ProcessEnv);
      json!({ "command": s.command, "args": s.args, "verbatim": s.verbatim })
    });
    agents.push(json!({
      "id": id,
      "name": def.name,
      "command": def.command,
      "args": def.args,
      "env": def.env,
      "binary": binary,
      "spawn": spawn,
      "initialize": initialize_request(def),
    }));
  }
  if args.iter().any(|a| a == "--json") {
    println!("{}", serde_json::to_string_pretty(&json!({ "agents": agents })).unwrap_or_default());
    return 0;
  }
  for a in &agents {
    let found = a["binary"].as_str().unwrap_or("not found");
    println!("{:<10} {:<16} {found}", a["id"].as_str().unwrap_or(""), a["name"].as_str().unwrap_or(""));
  }
  0
}
