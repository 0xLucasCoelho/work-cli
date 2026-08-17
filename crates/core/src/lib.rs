//! work-core: isolation + orchestration. The CLI and (later) Tauri app are
//! thin clients over this crate.

pub mod config;
pub mod doctor;
pub mod engine;
pub mod error;
pub mod isolation;
pub mod naming;
pub mod safety;
pub mod workspace;
