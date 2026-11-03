//! `acpira-gateway`: the computer side of paired devices (docs/dev/host-architecture.md, Remote access gateway). It runs
//! inside the engine (acpira-host `devices/host.rs`), which is the only binary linking it and iroh's network stack.

pub mod awake;
pub mod bonjour;
pub mod bridge;
pub mod cli;
pub mod control;
pub mod engine;
pub mod firewall;
pub mod gateway;
pub mod hosted;
pub mod presence;
pub mod push;
pub mod store;
pub mod wire_stats;
