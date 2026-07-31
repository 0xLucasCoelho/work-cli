#!/bin/sh
# Universal installer for `work` — downloads a pre-built binary from the latest
# GitHub release into a writable bin dir. macOS + Linux, arm64 + x86_64.
set -e

REPO="coelhucas-dev/work-cli"
BIN_NAME="work"

command -v curl >/dev/null 2>&1 || { echo "error: curl is required" >&2; exit 1; }
command -v tar  >/dev/null 2>&1 || { echo "error: tar is required"  >&2; exit 1; }

# Resolve the latest tag (strip surrounding quotes / leading 'v' kept for URL).
latest_tag=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
    | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n1)
[ -n "$latest_tag" ] || { echo "error: could not determine latest release" >&2; exit 1; }

# Map (os, arch) -> release archive target name.
os=$(uname -s | tr '[:upper:]' '[:lower:]')
arch=$(uname -m)
case "$os-$arch" in
    darwin-arm64)   target="work-aarch64-apple-darwin" ;;
    darwin-x86_64)  target="work-x86_64-apple-darwin" ;;
    linux-x86_64)   target="work-x86_64-unknown-linux-gnu" ;;
    *) echo "error: no prebuilt binary for $os-$arch" >&2; exit 1 ;;
esac

# Pick a writable install dir.
install_dir="/usr/local/bin"
if [ ! -w "$install_dir" ]; then
    install_dir="${CARGO_HOME:-$HOME/.cargo}/bin"
fi
if [ ! -w "$install_dir" ]; then
    install_dir="$HOME/.local/bin"
fi
mkdir -p "$install_dir"

tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT
url="https://github.com/${REPO}/releases/download/${latest_tag}/${target}.tar.gz"
echo "Downloading $url"
curl -fsSL "$url" -o "$tmpdir/pkg.tar.gz"
tar -xzf "$tmpdir/pkg.tar.gz" -C "$tmpdir"

cp "$tmpdir/$BIN_NAME" "$install_dir/$BIN_NAME"
chmod +x "$install_dir/$BIN_NAME"
echo "Installed $BIN_NAME to $install_dir"
echo "Run: $install_dir/$BIN_NAME --version"
