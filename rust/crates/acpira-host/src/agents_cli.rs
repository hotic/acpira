//! `acpira agents [--json]`: the built-in agents as this host launches them. The plain form lists where each CLI was found; `--json`
//! is what the repository probes read, so they spawn and initialize an agent exactly as the sidecar would

use serde_json::{Value, json};

use crate::acp::agents::registry::AgentRegistry;
use crate::acp::transport::process::initialize_request;
use crate::platform::command::{Os, spawn_spec};
use crate::store::{agent_config, data_dir};

pub async fn run(args: &[String]) -> i32 {
  crate::acp::agents::login_path::ready().await;
  let root = args
    .iter()
    .position(|a| a == "--home")
    .and_then(|i| args.get(i + 1))
    .map(|s| data_dir::absolute(std::path::Path::new(s)))
    .unwrap_or_else(data_dir::acpira_home);
  let path = root.join("agents.json");
  let custom = match agent_config::read(&path).await {
    Ok(v) => v,
    Err(e) if e.downcast_ref::<std::io::Error>().is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) => Value::Null,
    Err(e) => {
      eprintln!("{}: {e}", path.display());
      return 1;
    }
  };
  let registry = AgentRegistry::new(&custom);
  let exe = std::env::current_exe().ok().map(|p| p.to_string_lossy().into_owned());
  let registry = match exe {
    Some(exe) => registry.with_self_agent(&exe, &root),
    None => registry,
  };
  let mut agents = vec![];
  for id in registry.ids().to_vec() {
    let Ok(def) = registry.get(&id) else { continue };
    let binary = registry.resolve_binary(&id).await;
    let spawn = binary.as_deref().map(|b| {
      let s = spawn_spec(b, &def.args, Os::current(), std::env::var("ComSpec").ok().as_deref());
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
  let added = crate::acp::agents::login_path::added();
  if !added.is_empty() {
    println!("PATH from the login shell adds: {}", added.join(":"));
  }
  for a in &agents {
    let found = a["binary"].as_str().unwrap_or("not found");
    println!("{:<10} {:<16} {found}", a["id"].as_str().unwrap_or(""), a["name"].as_str().unwrap_or(""));
  }
  0
}
