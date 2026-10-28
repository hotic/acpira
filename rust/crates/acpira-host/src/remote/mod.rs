//! SSH remote workspaces (docs/dev/remote.md). `helper` is `acpira remote --stdio`, which a host runs for this machine's
//! engine; `client` is the engine's end of that channel; `wire` the frames both speak. `ssh`, `ssh_config` and `deploy`
//! reach a host through the system ssh, `connections` keeps the hosts and their channels; `peer` is this engine as a
//! client of a host's engine (remote run); `host_store` keeps one session history per host (the host's own store)

pub mod client;
pub mod connections;
pub mod deploy;
pub mod fetch;
pub mod helper;
pub mod host_store;
pub mod peer;
pub mod ssh;
pub mod ssh_config;
pub mod wire;
