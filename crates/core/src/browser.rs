//! Browser bridge primitives: the in-container `xdg-open` shim, the volume
//! FIFO path, host-browser selection, and the http(s) URL filter. Pure helpers
//! are unit-tested; `install_shim`/`ensure_fifo` touch the container and are
//! validated by the `work browse` smoke test.

use std::collections::HashSet;
use std::io::{self, Write};
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use url::{Host, Url};

use crate::config::{BrowserConfirmation, BrowserProfile};
use crate::engine::Engine;

/// Where the FIFO lives (in the workspace volume, so it persists across
/// container recreations and is reachable from both the shim and the host).
pub const FIFO_PATH: &str = "/home/dev/.work/browser.fifo";
/// Where the shim is installed — a system path, on PATH for every shell.
pub const SHIM_DEST: &str = "/usr/local/bin/xdg-open";

/// The in-container shim. Forwards an http(s) URL to the FIFO (non-blocking,
/// 5s timeout so a tool is never stuck without a bridge) and always echoes it.
/// Non-URL args are a silent no-op so calls on files/dirs don't break tools.
pub const SHIM: &str = r#"#!/bin/sh
# `work` browser shim: forward an http(s) URL to the host browser via the
# `work browse` bridge (a FIFO in the volume). Installed by `work new` /
# `work browse`. With no bridge running it still echoes the URL.
url=
for a in "$@"; do
  case "$a" in
    http://*|https://*) url="$a"; break ;;
  esac
done
[ -z "$url" ] && url="${1:-}"
case "$url" in
  http://*|https://*) ;;
  *) exit 0 ;;
esac
fifo="$HOME/.work/browser.fifo"
if [ -p "$fifo" ]; then
  timeout 5 sh -c 'printf "%s\n" "$1" > "$2"' sh "$url" "$fifo" 2>/dev/null
fi
printf '\n🌐  %s\n\n' "$url"
"#;

/// Host browser binary for a given OS string (`std::env::consts::OS`). PURE.
pub fn host_opener_for(os: &str) -> &'static str {
    match os {
        "macos" => "open",
        _ => "xdg-open",
    }
}

/// Effective host browser opener: `$WORK_HOST_BROWSER` wins if set, else the
/// OS default. WSL prefers `wslview` when it is installed so a browser URL
/// opened from the Linux process reaches the Windows host. The override is
/// used verbatim — the caller owns it.
pub fn host_opener() -> String {
    if let Some(b) = std::env::var_os("WORK_HOST_BROWSER") {
        return b.to_string_lossy().into_owned();
    }
    if is_wsl() && command_available("wslview") {
        return "wslview".into();
    }
    host_opener_for(std::env::consts::OS).to_string()
}

/// True when this Linux process is running inside Windows Subsystem for Linux.
/// WSL exports both variables for normal interactive distributions; accepting
/// either keeps this helper useful in CI and minimal WSL environments.
pub fn is_wsl() -> bool {
    is_wsl_environment(
        std::env::consts::OS,
        std::env::var_os("WSL_INTEROP").as_deref(),
        std::env::var_os("WSL_DISTRO_NAME").as_deref(),
    )
}

/// Pure WSL environment predicate used by the host-opener selection.
pub fn is_wsl_environment(
    target_os: &str,
    interop: Option<&std::ffi::OsStr>,
    distro: Option<&std::ffi::OsStr>,
) -> bool {
    target_os == "linux"
        && [interop, distro]
            .into_iter()
            .flatten()
            .any(|value| !value.is_empty())
}

fn command_available(binary: &str) -> bool {
    Command::new(binary)
        .arg("--help")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// True iff `s` is an `http(s)` URL — the only thing the bridge forwards. PURE.
pub fn is_openable_url(s: &str) -> bool {
    let s = s.trim();
    s.starts_with("http://") || s.starts_with("https://")
}

/// If `raw_url` is a login URL carrying a loopback `redirect_uri` (RFC 8252),
/// return the callback port to bridge so the host browser's redirect reaches
/// the in-container listener. `None` for non-loopback / absent / no-port. PURE.
pub fn callback_port(raw_url: &str) -> Option<u16> {
    let url = Url::parse(raw_url).ok()?;
    let redirect = url
        .query_pairs()
        .find(|(k, _)| k == "redirect_uri")
        .map(|(_, v)| v.into_owned())?;
    let r = Url::parse(&redirect).ok()?;
    if r.scheme() != "http" {
        return None;
    }
    let loopback = matches!(r.host(), Some(Host::Ipv4(ip)) if ip.is_loopback())
        || matches!(r.host(), Some(Host::Ipv6(ip)) if ip.is_loopback())
        || matches!(r.host(), Some(Host::Domain(d)) if d == "localhost");
    if !loopback {
        return None;
    }
    r.port()
}

/// Minimum interval between two host-browser opens. Bounds a container that
/// spams the FIFO from forcing a flurry of browser launches.
const BROWSE_RATE_LIMIT: Duration = Duration::from_secs(2);

/// Gate for `work browse`: each http(s) URL a container writes to the FIFO is
/// opened in the host browser only after the user confirms the HOST (once per
/// `work browse` session) and never more than once per `BROWSE_RATE_LIMIT`. The
/// container can *request* a navigation; it cannot drive an authenticated host
/// browser session (Jira/GitHub/GCP) silently.
pub struct BrowseGuard {
    confirmed: HashSet<String>,
    last_open: Option<Instant>,
    trusted: bool,
}

impl BrowseGuard {
    /// Trusted mode skips the per-host prompt. It is deliberately an explicit
    /// workspace preference; `prompt` remains the default.
    pub fn new(confirmation: BrowserConfirmation) -> Self {
        Self {
            confirmed: HashSet::new(),
            last_open: None,
            trusted: confirmation == BrowserConfirmation::Trusted,
        }
    }

    /// True iff `url` may be opened now (passes the rate limit + host confirm).
    pub fn should_open(&mut self, url: &str) -> bool {
        if self
            .last_open
            .is_some_and(|t| t.elapsed() < BROWSE_RATE_LIMIT)
        {
            eprintln!("· rate-limited (last open <2s ago): {url}");
            return false;
        }
        let host = Url::parse(url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .unwrap_or_default();
        if !self.trusted && !self.confirmed.contains(&host) {
            if host.is_empty() {
                eprintln!("· no host in {url} — not opening");
                return false;
            }
            eprint!("· open {host} in your host browser? [y/N] ");
            let _ = io::stderr().flush();
            let mut line = String::new();
            if io::stdin().read_line(&mut line).is_err() {
                return false;
            }
            if line.trim().eq_ignore_ascii_case("y") {
                self.confirmed.insert(host);
            } else {
                eprintln!("· skipped {url}");
                return false;
            }
        }
        self.last_open = Some(Instant::now());
        true
    }
}

/// Open `url` in the host browser. Unless the workspace selected `default`
/// (or the process-local `WORK_BROWSE_PROFILE=default` override is set), try a
/// throwaway Chrome guest profile on macOS so a forced
/// navigation can't ride an authenticated profile; fall back to the default
/// opener if Chrome isn't present. Returns an error string on failure.
pub fn open_url(opener: &str, url: &str, profile: BrowserProfile) -> Result<(), String> {
    if std::env::consts::OS == "macos"
        && profile == BrowserProfile::Guest
        && std::env::var("WORK_BROWSE_PROFILE").ok().as_deref() != Some("default")
    {
        let status = Command::new("open")
            .args([
                "-na",
                "Google Chrome",
                "--args",
                "--profile-directory=Guest Profile",
            ])
            .arg(url)
            .status();
        if matches!(status, Ok(s) if s.success()) {
            return Ok(());
        }
        // Chrome absent / failed -> fall through to the default opener.
    }
    match Command::new(opener).arg(url).status() {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => Err(format!("opener exited {s}")),
        Err(e) => Err(e.to_string()),
    }
}

/// Idempotently install the shim + symlinks + profile.d export as root. Runs
/// in one `sh -c` via `exec_root`.
pub fn install_shim(engine: &dyn Engine, ctr: &str) -> Result<()> {
    let script = format!(
        "set -e\n\
         cat > {dest} <<'WORK_SHIM_EOF'\n{shim}WORK_SHIM_EOF\n\
         chmod 0755 {dest}\n\
         ln -sf {dest} /usr/local/bin/sensible-browser\n\
         ln -sf {dest} /usr/local/bin/x-www-browser\n\
         mkdir -p /etc/profile.d\n\
         cat > /etc/profile.d/work-browser.sh <<'WORK_PROF_EOF'\nexport BROWSER={dest}\nWORK_PROF_EOF\n\
         chmod 0644 /etc/profile.d/work-browser.sh\n",
        dest = SHIM_DEST,
        shim = SHIM,
    );
    engine
        .exec_root(ctr, &["sh", "-c", &script])
        .with_context(|| format!("installing browser shim in {ctr}"))
}

/// Idempotently create the FIFO (owned by dev). Runs as dev.
pub fn ensure_fifo(engine: &dyn Engine, ctr: &str) -> Result<()> {
    engine.exec_capture(
        ctr,
        &[
            "sh",
            "-c",
            "mkdir -p \"$HOME/.work\"; [ -p \"$HOME/.work/browser.fifo\" ] || mkfifo \"$HOME/.work/browser.fifo\"",
        ],
    )?;
    // Surface a clear error if the node isn't a FIFO after the attempt.
    if engine
        .exec_capture(ctr, &["test", "-p", FIFO_PATH])
        .is_err()
    {
        bail!("could not create browser FIFO {FIFO_PATH} in {ctr}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_opener_per_os() {
        assert_eq!(host_opener_for("macos"), "open");
        assert_eq!(host_opener_for("linux"), "xdg-open");
        assert_eq!(host_opener_for("freebsd"), "xdg-open"); // unknown -> xdg-open
    }

    #[test]
    fn wsl_environment_is_linux_only() {
        let marker = std::ffi::OsStr::new("1");
        assert!(is_wsl_environment("linux", Some(marker), None));
        assert!(!is_wsl_environment("macos", Some(marker), None));
        assert!(!is_wsl_environment("windows", None, Some(marker)));
    }

    #[test]
    fn openable_url_filters_scheme() {
        assert!(is_openable_url("https://example.com/login"));
        assert!(is_openable_url("http://127.0.0.1:8080/cb?code=x"));
        assert!(!is_openable_url("ftp://example.com"));
        assert!(!is_openable_url("mailto:a@b.com"));
        assert!(!is_openable_url("file:///etc/hosts"));
        assert!(!is_openable_url(""));
        assert!(!is_openable_url("not a url"));
        // tolerates surrounding whitespace (docker exec capture can pad)
        assert!(is_openable_url("  https://example.com  "));
    }

    #[test]
    fn callback_port_from_login_url() {
        let url = "https://provider.example/oauth/authorize?client_id=x&redirect_uri=http%3A%2F%2Flocalhost%3A8080%2Fcallback&code_challenge=y";
        assert_eq!(callback_port(url), Some(8080));
        assert_eq!(
            callback_port("https://p/a?redirect_uri=http://127.0.0.1:9000/cb"),
            Some(9000)
        );
        assert_eq!(
            callback_port("https://p/a?redirect_uri=http://[::1]:7000/cb"),
            Some(7000)
        );
    }

    #[test]
    fn callback_port_none_when_not_loopback_or_absent() {
        assert_eq!(
            callback_port("https://p/a?redirect_uri=https://example.com/cb"),
            None
        );
        assert_eq!(
            callback_port("https://p/a?redirect_uri=http://example.com:8080/cb"),
            None
        );
        assert_eq!(callback_port("https://p/a"), None);
        assert_eq!(
            callback_port("https://p/a?redirect_uri=http://localhost/cb"),
            None
        );
        assert_eq!(callback_port("not a url"), None);
    }
}
