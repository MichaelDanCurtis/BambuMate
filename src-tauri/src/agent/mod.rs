//! In-app agent: Codex (app-server) and Claude (CLI) backends that drive
//! BambuMate through the `bm_*` tool registry.

pub mod types;
pub mod locate;
pub mod snapshot;
pub mod validate;
pub mod asks;
pub mod tools;
