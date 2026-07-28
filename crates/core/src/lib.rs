//! work-core: isolation + orchestration engine for the `work` CLI.
//!
//! Isolation logic lives in exactly one place — this crate. The CLI is a thin
//! client over it.

pub mod error;
