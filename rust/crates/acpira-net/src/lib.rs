//! Paired devices: a phone or another computer reaching this computer's engine without accounts. One Ed25519 key per
//! install is the identity; pairing (a QR code, a link, a short code on the LAN) leaves each side with the other's key;
//! iroh then finds a path in one QUIC connection (LAN, Tailscale, a punched hole, a relay). The gateway on the computer
//! and the phone's `acpira-mobile` both build on this crate, and nothing else in Acpira links iroh.

pub mod endpoint;
pub mod frame;
pub mod identity;
pub mod link;
pub mod pair;
pub mod path;
pub mod ticket;

pub use iroh;
pub use iroh::{Endpoint, EndpointAddr, EndpointId, RelayUrl, SecretKey};
