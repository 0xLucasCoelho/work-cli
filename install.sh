#!/bin/sh
# Universal installer for `work` — downloads a pre-built binary from the latest
# GitHub release into a writable bin dir. Native macOS + Linux, arm64 + amd64.
# Windows users should run this installer from a Linux shell inside WSL.
set -e

REPO="0xlucascoelho/work-cli"
BIN_NAME="work"

os=$(uname -s | tr '[:upper:]' '[:lower:]')
arch=$(uname -m)

# WSL reports itself as Linux. Detect it so the installer can make the
# supported Windows path explicit while still selecting the Linux artifact.
is_wsl=0
case "$os" in
    linux)
        case "${WSL_INTEROP:-}${WSL_DISTRO_NAME:-}" in
            *[![:space:]]*) is_wsl=1 ;;
            *)
                case "$(uname -r | tr '[:upper:]' '[:lower:]')" in
                    *microsoft*|*wsl*) is_wsl=1 ;;
                esac
                ;;
        esac
        ;;
    mingw*|msys*|cygwin*|windows*)
        cat >&2 <<'EOF'
error: native Windows installation is not supported.
Install and run `work` inside a Linux distribution managed by WSL instead.
Open your WSL terminal and run this installer there so it downloads the Linux artifact.
EOF
        exit 1
        ;;
esac

command -v curl >/dev/null 2>&1 || { echo "error: curl is required" >&2; exit 1; }
command -v tar  >/dev/null 2>&1 || { echo "error: tar is required"  >&2; exit 1; }

# Resolve the latest tag (strip surrounding quotes / leading 'v' kept for URL).
latest_tag=$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" \
    | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n1)
[ -n "$latest_tag" ] || { echo "error: could not determine latest release" >&2; exit 1; }

# Map (os, arch) -> release archive target name. These are the targets built
# by .github/workflows/release.yml; Windows intentionally has no native target.
case "$os-$arch" in
    darwin-arm64|darwin-aarch64) target="work-aarch64-apple-darwin" ;;
    darwin-x86_64|darwin-amd64) target="work-x86_64-apple-darwin" ;;
    linux-aarch64|linux-arm64)   target="work-aarch64-unknown-linux-gnu" ;;
    linux-x86_64|linux-amd64)    target="work-x86_64-unknown-linux-gnu" ;;
    *) echo "error: no prebuilt binary for $os-$arch (use Linux in WSL on Windows)" >&2; exit 1 ;;
esac

if [ "$is_wsl" -eq 1 ]; then
    echo "Detected WSL; installing the Linux $arch artifact for work." >&2
fi

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
# Verify build provenance if the GitHub CLI is available; otherwise warn and
# continue (attestation is best-effort, not a hard install requirement).
if command -v gh >/dev/null 2>&1; then
    gh attestation verify "$tmpdir/pkg.tar.gz" --repo "${REPO}" \
        || { echo "error: attestation verification failed" >&2; exit 1; }
else
    echo "warning: gh not found — skipping provenance verification" >&2
fi
tar -xzf "$tmpdir/pkg.tar.gz" -C "$tmpdir"

cp "$tmpdir/$BIN_NAME" "$install_dir/$BIN_NAME"
chmod +x "$install_dir/$BIN_NAME"
echo "Installed $BIN_NAME to $install_dir"
echo "Run: $install_dir/$BIN_NAME --version"
