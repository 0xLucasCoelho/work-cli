//! Author-default dotfile templates, embedded at build from `personal/dotfiles`.
//! `--use-author-default` extracts these into a workspace so it comes up with the
//! author's full shell/editor/agent config — no external path required.

use anyhow::{Context, Result};
use include_dir::{include_dir, Dir};
use tempfile::TempDir;

/// Bundled author-default dotfile tree. Source of truth: `personal/dotfiles`.
pub static TEMPLATES: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/../../personal/dotfiles");

/// Extract the embedded templates to a fresh tempdir. Keep the returned TempDir
/// alive for as long as the extracted path is used (e.g. until after `seed_dir`).
pub fn extract_to_tempdir() -> Result<TempDir> {
    let dir = tempfile::tempdir().context("staging author-default templates")?;
    TEMPLATES
        .extract(dir.path())
        .context("extracting author-default templates")?;
    Ok(dir)
}
