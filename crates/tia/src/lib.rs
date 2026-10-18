//! Test impact analysis: select the tests a change can affect.
//!
//! Stage 0 is the static-graph rung. [`conventions`] decides what a test file
//! is, and [`select`] turns a change set and the dependency graph into the test
//! files that change can affect, behind an escalation ladder that never skips a
//! changed file. Nothing here executes a test, and nothing is persisted.
//!
//! The skeleton this crate shipped with — `read_inputs`, which gathered one
//! item from each declared dependency to prove the set was sufficient and legal
//! — is gone: the real entry point is [`select::select`], and it reads the
//! parser, the graph, discovery, and the `[tia]` settings directly. The
//! coverage, mutation, and churn readers the skeleton exercised are inputs to
//! the recorders that arrive with the T1 tier, not to a static selection.

pub mod conventions;
pub mod select;
