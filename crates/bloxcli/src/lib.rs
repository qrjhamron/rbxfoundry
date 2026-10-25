//! Library root. Canonical modules are grouped by responsibility;
//! the flat re-exports at the bottom keep the pre-refactor public
//! paths (`bloxcli::engine`, `bloxcli::tree`, ...) working for
//! integration tests and external callers.

pub mod app;
pub mod cli;
pub mod commands;
pub mod diagnostics;
pub mod history;
pub mod mcp;
pub mod protocol;
pub mod rbxio;
pub mod session;
pub mod templates;
pub mod validation;

// Legacy flat paths (re-exports, not copies).
pub use diagnostics::{completion, diff};
pub use protocol::{engine, tree};
pub use rbxio::convert;
pub use session::{debug, net, play};
pub use validation::integrity;
