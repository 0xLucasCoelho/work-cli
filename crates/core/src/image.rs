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

use std::path::Path;

/// `work image build`: build the default image, or a custom `--tag` from a
/// `--dockerfile` path. Building a non-default tag requires `--dockerfile`.
pub fn build(engine: &dyn Engine, tag: Option<&str>, dockerfile: Option<&Path>) -> Result<()> {
    let tag = tag.unwrap_or(DEFAULT_IMAGE);
    match dockerfile {
        Some(path) => {
            let content = fs::read_to_string(path)
                .with_context(|| format!("reading Dockerfile {}", path.display()))?;
            build_image(engine, tag, &content)?;
        }
        None => {
            if tag == DEFAULT_IMAGE {
                build_default(engine)?;
            } else {
                anyhow::bail!("building a custom tag '{tag}' requires --dockerfile <path>");
            }
        }
    }
    Ok(())
}

/// Starter Dockerfile for a personal `work` image. Written by `work image init`.
/// Extends `work-base:latest` (so isolation invariants are inherited), installs
/// a few popular tools as prebuilt binaries, and documents the glibc/musl gotcha.
pub const PERSONAL_TEMPLATE: &str = r#"# Personal `work` image — customize this, then:
#   work image build --tag my-work:latest --dockerfile ./Dockerfile.work
#   work new <ws> --image my-work:latest
#   # or make it the default: set `default_image = "my-work:latest"` in
#   # ~/.config/work/config.toml
#
# This EXTENDS work-base:latest, so every isolation invariant `work doctor`
# checks is inherited (non-root `dev` user, /home/dev home, tmux/zsh/bash).
# Keep `USER dev` at the end, or `work doctor` will flag the container as root.
#
# Bring your ~/.zshrc per-workspace via `work new <ws> --import-shell-config`;
# the tools added below will be present, so your rc stops throwing
# "command not found".
FROM work-base:latest
USER root
# --- System packages (apt). Uncomment/add what you need. ---
RUN apt-get update && apt-get install -y --no-install-recommends \
      fzf ripgrep direnv less file unzip ca-certificates \
 && rm -rf /var/lib/apt/lists/*
# --- Rust-based CLIs as prebuilt binaries via cargo-binstall ---
# (No Rust toolchain, no compiling — fetches each project's release binary.)
# CARGO_HOME=/usr/local -> binaries land in /usr/local/bin (on PATH for all).
# Add/remove crates from the list. Popular: starship zoxide fd-find bat eza
# git-delta mise.
ENV CARGO_HOME=/usr/local
RUN curl -L --proto '=https' --tlsv1.2 -sSf \
      https://raw.githubusercontent.com/cargo-bins/cargo-binstall/main/install-from-binstall-release.sh \
    | bash \
 && cargo-binstall -y --disable-strategies compile \
      starship zoxide \
 && rm -rf /usr/local/registry /usr/local/.crates*
# GOTCHA — glibc version: some projects' default Linux binaries need a newer
# glibc than Debian bookworm ships (2.36). If a binstalled binary fails with
# `GLIBC_2.xx not found`, fetch that tool's *musl* (static) build instead, e.g.
# for atuin:
#   RUN ARCH=$(uname -m) \
#    && curl -fsSL "https://github.com/atuinsh/atuin/releases/latest/download/atuin-${ARCH}-unknown-linux-musl.tar.gz" \
#       | tar xz -C /tmp \
#    && mv "/tmp/atuin-${ARCH}-unknown-linux-musl/atuin" /usr/local/bin/atuin \
#    && rm -rf /tmp/atuin-*
# --- keep tool dirs on PATH for every zsh shell ---
RUN printf '%s\n' 'export PATH="/usr/local/bin:$HOME/.local/bin:$PATH"' >> /etc/zsh/zshenv
USER dev
WORKDIR /home/dev
"#;

/// `work image init`: write a personal-image starter Dockerfile (extends
/// work-base) for the user to customize. Refuses to overwrite an existing file.
pub fn init_template(path: &Path) -> Result<()> {
    if path.exists() {
        anyhow::bail!(
            "{} already exists; move it aside before scaffolding a new one",
            path.display()
        );
    }
    fs::write(path, PERSONAL_TEMPLATE).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::PERSONAL_TEMPLATE;

    #[test]
    fn personal_template_extends_work_base_and_keeps_dev_user() {
        // Must inherit work-base (isolation invariants) and end as the dev user.
        assert!(PERSONAL_TEMPLATE.contains("FROM work-base:latest"));
        assert!(PERSONAL_TEMPLATE.contains("USER dev"));
        // Must teach the build command and the glibc/musl gotcha.
        assert!(PERSONAL_TEMPLATE.contains("work image build"));
        assert!(PERSONAL_TEMPLATE.contains("musl"));
    }
}
