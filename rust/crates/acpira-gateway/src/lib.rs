//! `acpira-gateway`: the computer side of paired devices (docs/dev/paired-remote.md). A separate binary rather than an
//! `acpira` subcommand: iroh brings some 220 crates the engine never needs, and only the computers that turn remote
//! access on run this. Unix only for now: its control socket and the engine's are Unix sockets.

#![cfg(unix)]

pub mod awake;
pub mod bonjour;
pub mod bridge;
pub mod cli;
pub mod control;
pub mod engine;
pub mod gateway;
pub mod store;
