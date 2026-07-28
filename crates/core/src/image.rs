//! Default base image (`work-base:latest`) + `work image build`.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use tempfile::TempDir;

use crate::config::DEFAULT_IMAGE;
use crate::engine::{build_image_at, Engine};

/// Embedded default Dockerfile (matches the spec's base image).
pub const DEFAULT_DOCKERFILE: &str = include_str!("../../docker/work-base.Dockerfile");

/// Build the default image `work-base:latest`.
pub fn build_default(engine: &dyn Engine) -> Result<()> {
    build_image(engine, DEFAULT_IMAGE, DEFAULT_DOCKERFILE)
}

/// Build `tag` from `dockerfile_content` using a throwaway build context.
pub fn build_image(engine: &dyn Engine, tag: &str, dockerfile_content: &str) -> Result<()> {
    let dir: TempDir = tempfile::tempdir().context("creating build context")?;
    let dockerfile_path: PathBuf = dir.path().join("Dockerfile");
    fs::write(&dockerfile_path, dockerfile_content).context("writing Dockerfile")?;
    build_image_at(engine, tag, dir.path(), &dockerfile_path)?;
    // Keep the tempdir alive through the build, then it drops.
    drop(dir);
    Ok(())
}
