//! One engine per machine (per OS user and data root): every shell (VS Code windows, the desktop app, IntelliJ, the
//! remote access gateway, SSH clients running sessions here) reaches the same `acpira serve` on `<home>/run/engine.sock`
//! through `acpira engine` (`ensure`). The newest build serves (`policy`), always from its landed copy (`stage`); on
//! macOS under one fixed program (`launcher`), so privacy grants survive new builds.

pub mod ensure;
pub mod launcher;
pub mod policy;
pub mod stage;
