//! The engine devices reach: one persistent engine of its own (`shell: "remote"` in its socket key), started on demand
//! with the login shell's environment, so an agent started from the phone sees what one started from the desktop app
//! sees. Sessions other shells' engines run are reached from it over the push bus; it ends on its own once no device is
//! connected and no turn runs.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use acpira_desktop::engine::{EndpointOpts, EngineEndpoint, EngineStream, engine_endpoint, reach_engine};
use acpira_desktop::locator::SidecarCommand;

pub type Log = Arc<dyn Fn(&str) + Send + Sync>;

pub struct EngineSpec {
  pub binary: PathBuf,
  pub home: PathBuf,
  /// Pass `--home` (the gateway itself was given one); otherwise the engine finds ACPIRA_HOME / ~/.acpira like the shells
  pub explicit_home: bool,
  /// The login shell's environment, merged into the launched engine's
  pub env: BTreeMap<String, String>,
  /// Tests: an engine already listening here
  pub socket: Option<PathBuf>,
}

impl EngineSpec {
  pub fn endpoint(&self) -> EngineEndpoint {
    match &self.socket {
      Some(s) => EngineEndpoint { socket: s.clone(), log: s.with_extension("log") },
      None => engine_endpoint(&EndpointOpts { binary: &self.binary, home: &self.home, cwd: None, shell: Some("remote"), tmp: None, uid: None }),
    }
  }
}

pub async fn connect(spec: &EngineSpec, log: &Log) -> std::io::Result<EngineStream> {
  let mut cmd = SidecarCommand::binary(&spec.binary);
  if spec.explicit_home {
    cmd.args = vec!["--home".into(), spec.home.to_string_lossy().into_owned()];
  }
  cmd.env = spec.env.clone();
  let log = log.clone();
  reach_engine(&cmd, &spec.endpoint(), None, &|| false, &move |l: &str| log(l)).await
}
