//! Shared logger callback type used across storage, accounts, plugins and session modules.

use std::sync::Arc;

pub type LogFn = Arc<dyn Fn(&str) + Send + Sync>;
