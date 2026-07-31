//! work-core: isolation + orchestration engine for the `work` CLI.
//!
//! Isolation logic lives in exactly one place — this crate. The CLI is a thin
//! client over it.

pub mod banner;
pub mod browser;
pub mod config;
pub mod doctor;
pub mod engine;
pub mod error;
pub mod image;
pub mod naming;
pub mod safety;
pub mod templates;
pub mod update;
pub mod workspace;
