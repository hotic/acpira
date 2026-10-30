//! `acpira-gateway`: the computer side of paired devices (docs/dev/paired-remote.md). A separate binary rather than an
//! `acpira` subcommand: iroh brings some 220 crates the engine never needs, and only the computers that turn remote
//! access on run this. The Windows desktop exe runs it as its `gateway` role instead (apps/desktop/src/main.rs).

pub mod awake;
pub mod bonjour;
pub mod bridge;
pub mod cli;
pub mod control;
pub mod engine;
pub mod gateway;
pub mod presence;
pub mod push;
pub mod store;
