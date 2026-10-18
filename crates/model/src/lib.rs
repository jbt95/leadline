//! The floor of the workspace: the analysis data model and the security
//! vocabulary every other crate names.
//!
//! Nothing here depends on another workspace crate, so this is the crate
//! `config`, `git`, and `update` can reach without pulling the parser or the
//! project model in behind it.

pub mod core;
pub mod security;
pub mod symbol;

/// Default index directory, relative to an analysis root.
///
/// `config` defaults the configured index path from this and the CLI falls back
/// to it, so the constant sits below both.
pub const DEFAULT_INDEX_DIR: &str = ".leadline";

pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub type Result<T, E = Error> = std::result::Result<T, E>;
