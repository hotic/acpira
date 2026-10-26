//! `acpira plugin list|propose|disable|rollback` and `acpira safe-mode on|off`, for scripts, tests and the extension
//! bench. Nothing here enables a plugin: `propose` files the frozen version for the user to confirm on the plugin page
//! (`docs/dev/plugins.md`, Activation), so a model with a shell cannot skip the card

use std::path::Path;
use std::sync::Arc;

use crate::plugins::host::{PluginHost, describe};
use crate::plugins::store::Provenance;

const USAGE: &str = "usage: acpira plugin list [--json] | propose <id> | disable <id> | rollback <id>";

fn host(home: &Path) -> Arc<PluginHost> {
  PluginHost::new(home.join(crate::plugins::DIR), false, Arc::new(|line| eprintln!("[acpira] {line}")))
}

pub async fn plugin(args: &[String], home: &Path) -> i32 {
  let host = host(home);
  host.start().await;
  let id = args.get(1).filter(|a| !a.starts_with("--")).map(String::as_str);
  match (args.first().map(String::as_str), id) {
    (Some("list"), _) => {
      let view = host.view();
      if args.iter().any(|a| a == "--json") {
        println!("{}", serde_json::to_string_pretty(&view).unwrap_or_default());
        return 0;
      }
      if view.safe_mode {
        println!("safe mode is on: plugins are unloaded");
      }
      for p in &view.plugins {
        let state = match (p.enabled, p.loaded, p.proposal.is_some()) {
          (true, true, _) => "enabled",
          (true, false, _) => "enabled, not loaded",
          (false, _, true) => "proposed",
          _ => "off",
        };
        println!("{}\t{}\t{state}{}", p.id, p.version, if p.changed { "\tchanges not enabled" } else { "" });
        for e in p.error.iter().chain(p.findings.iter().flat_map(|f| f.errors.iter())) {
          println!("\terror: {e}");
        }
      }
      0
    }
    (Some("propose"), Some(id)) => match host.propose(id, Provenance::cli()).await {
      Ok((frozen, change)) => {
        println!("{}", describe(&frozen));
        println!();
        println!("{}", if change.is_some() { "Proposed: confirm it on Acpira's plugin page." } else { "Already proposed or enabled." });
        0
      }
      Err(e) => {
        eprintln!("{e:#}");
        1
      }
    },
    (Some(op @ ("disable" | "rollback")), Some(id)) => {
      let r = if op == "disable" { host.store().disable(id, Provenance::cli()).await } else { host.store().rollback(id, Provenance::cli()).await };
      match r {
        Ok(Some(_)) => 0,
        Ok(None) => {
          println!("{id}: nothing to change");
          0
        }
        Err(e) => {
          eprintln!("{e:#}");
          1
        }
      }
    }
    _ => {
      eprintln!("{USAGE}");
      2
    }
  }
}

pub async fn safe_mode(args: &[String], home: &Path) -> i32 {
  let on = match args.first().map(String::as_str) {
    Some("on") => true,
    Some("off") => false,
    _ => {
      eprintln!("usage: acpira safe-mode on|off");
      return 2;
    }
  };
  match host(home).set_safe_mode(on, Provenance::cli()).await {
    Ok(_) => {
      println!("safe mode {}: every Acpira engine on this machine follows within a second", if on { "on" } else { "off" });
      0
    }
    Err(e) => {
      eprintln!("{e:#}");
      1
    }
  }
}
