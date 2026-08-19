//! Isolation verification. Collection (engine inspection via the selected CLI) is separate
//! from analysis (pure), so analysis is unit-testable.

use std::collections::BTreeSet;

use anyhow::Result;

use crate::config;
use crate::engine::{ContainerState, Engine};
use crate::naming;

/// How a failed check affects the ability to use a workspace.
///
/// `Warning` findings remain visible to `work doctor`, but are not a reason to
/// refuse a normal attach. `Blocking` is reserved for facts that cross, or
/// prevent us from verifying, an isolation boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckSeverity {
    Blocking,
    Warning,
}

#[derive(Debug, Clone)]
pub struct CheckResult {
    pub label: String,
    pub ok: bool,
    pub severity: CheckSeverity,
    pub detail: String,
}

impl CheckResult {
    fn blocking(label: impl Into<String>, ok: bool, detail: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            ok,
            severity: CheckSeverity::Blocking,
            detail: detail.into(),
        }
    }

    fn warning(label: impl Into<String>, ok: bool, detail: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            ok,
            severity: CheckSeverity::Warning,
            detail: detail.into(),
        }
    }

    pub fn blocks_attach(&self) -> bool {
        !self.ok && self.severity == CheckSeverity::Blocking
    }
}

/// Inputs to the pure cross-volume check.
#[derive(Debug, Clone)]
pub struct IsolationProbe {
    pub ws: String,
    pub networks: BTreeSet<String>,
    pub mounts: Vec<(String, String)>,
}

/// Inputs to the pure hardening check.
#[derive(Debug, Clone)]
pub struct HardeningProbe {
    pub ws: String,
    pub restart_policy: String,
    pub user: String,
    pub image: String,
    pub configured_image: String,
    pub ports_json: String,
    /// `{{.Image}}` (the sha the container actually runs). `None` if unreadable.
    pub running_image_id: Option<String>,
    /// The configured tag's image id RE-RESOLVED at check time
    /// (the selected engine's image inspect command). Compared against
    /// `running_image_id` to detect tag drift (a rebuilt work-base:latest).
    pub resolved_image_id: Option<String>,
    /// True iff the container carries the work managed label.
    pub managed_label: bool,
}

/// A single workspace is isolated iff:
/// - its only network is its own `work-net-<ws>`;
/// - its only mount is its own `work-<ws>-home` at `/home/dev` (a *volume* mount).
pub fn analyze_isolation(
    ws: &str,
    networks: &BTreeSet<String>,
    mounts: &[(String, String)],
) -> CheckResult {
    let expected_net = naming::network(ws);
    let expected_vol = naming::volume(ws);
    let expected_target = String::from("/home/dev");

    let nets_ok = networks.len() == 1 && networks.iter().all(|n| n == &expected_net);
    if !nets_ok {
        return CheckResult::blocking(
            ws,
            false,
            format!(
                "networks must be exactly {{{expected_net}}}, found {:?}",
                networks.iter().collect::<Vec<_>>()
            ),
        );
    }

    let mounts_ok = mounts.len() == 1
        && mounts.iter().all(|(src, dst)| {
            // A volume mount's source is the volume name (no leading '/').
            // A host bind mount would start with '/' (or a drive) -> rejected.
            src == &expected_vol && dst == &expected_target && !src.starts_with('/')
        });
    if !mounts_ok {
        return CheckResult::blocking(
            ws,
            false,
            format!(
                "mounts must be exactly {{{expected_vol} -> {expected_target}}}, found {:?}",
                mounts
            ),
        );
    }

    CheckResult::blocking(
        ws,
        true,
        format!("on dedicated network {expected_net}; only {expected_vol} mounted"),
    )
}

/// Across all workspaces: no container may mount another workspace's volume.
pub fn analyze_cross_volume(probes: &[IsolationProbe]) -> Vec<CheckResult> {
    // Every workspace's own volume name.
    let own_volumes: BTreeSet<String> = probes.iter().map(|p| naming::volume(&p.ws)).collect();

    probes
        .iter()
        .map(|p| {
            let expected_vol = naming::volume(&p.ws);
            // Any mount whose source is a known workspace volume that isn't ours.
            let breach = p
                .mounts
                .iter()
                .find(|(src, _)| own_volumes.contains(src) && src != &expected_vol);
            match breach {
                Some((vol, dst)) => CheckResult::blocking(
                    format!("{}:cross-volume", p.ws),
                    false,
                    format!("mounts foreign workspace volume {vol} at {dst}"),
                ),
                None => CheckResult::blocking(
                    format!("{}:cross-volume", p.ws),
                    true,
                    "no foreign workspace volume mounted",
                ),
            }
        })
        .collect()
}

/// Count published host-port bindings from `{{json .NetworkSettings.Ports}}`.
/// `{}` or null -> 0; `{"8080/tcp":[{"HostIp":"...","HostPort":"8080"}]}` -> 1.
fn published_port_count(ports_json: &str) -> usize {
    let v: serde_json::Value = serde_json::from_str(ports_json).unwrap_or(serde_json::Value::Null);
    let Some(map) = v.as_object() else {
        return 0;
    };
    map.values()
        .filter_map(|b| b.as_array())
        .map(|a| a.len())
        .sum()
}

/// Podman qualifies locally built short image names with `localhost/` when
/// reporting `.Config.Image`; Docker-compatible engines generally preserve the
/// short name. Treat only that implicit local-registry prefix as equivalent.
fn image_refs_match(actual: &str, configured: &str) -> bool {
    actual == configured
        || actual
            .strip_prefix("localhost/")
            .is_some_and(|short| short == configured)
        || configured
            .strip_prefix("localhost/")
            .is_some_and(|short| short == actual)
}

/// Per-workspace hardening. Only root execution and published host ports are
/// isolation boundaries; policy/configuration drift is intentionally warning
/// severity so it does not make an otherwise safe workspace unusable. PURE.
pub fn analyze_hardening(p: &HardeningProbe) -> Vec<CheckResult> {
    let mut out = Vec::new();

    let restart_ok = p.restart_policy == "unless-stopped";
    out.push(CheckResult::warning(
        format!("{}:restart", p.ws),
        restart_ok,
        if restart_ok {
            "restart=unless-stopped".into()
        } else {
            format!(
                "restart policy must be 'unless-stopped', found '{}'",
                p.restart_policy
            )
        },
    ));

    let non_root = !matches!(p.user.as_str(), "root" | "0");
    out.push(CheckResult::blocking(
        format!("{}:user", p.ws),
        non_root,
        if non_root {
            if p.user.is_empty() {
                "non-root (image default user)".into()
            } else {
                format!("runs as '{}'", p.user)
            }
        } else {
            "container must not run as root".into()
        },
    ));

    let img_ok = image_refs_match(&p.image, &p.configured_image);
    out.push(CheckResult::warning(
        format!("{}:image", p.ws),
        img_ok,
        if img_ok {
            format!("image={}", p.image)
        } else {
            format!(
                "container image '{}' != configured '{}'",
                p.image, p.configured_image
            )
        },
    ));

    let nports = published_port_count(&p.ports_json);
    out.push(CheckResult::blocking(
        format!("{}:ports", p.ws),
        nports == 0,
        if nports == 0 {
            "no host ports published".into()
        } else {
            format!("workspace container publishes {nports} host port(s) — isolation risk")
        },
    ));

    let label_ok = p.managed_label;
    out.push(CheckResult::warning(
        format!("{}:managed", p.ws),
        label_ok,
        if label_ok {
            "work-managed label present"
        } else {
            "missing work-managed label — recreate it: `work harden <ws>`"
        },
    ));

    // Image drift: compare the image the container actually runs against the
    // configured tag's CURRENT resolution (re-resolved here, at check time).
    // Comparing against a digest recorded at create time would be tautological —
    // a container pins an image id, not a tag — so a locally-rebuilt
    // work-base:latest (tag now -> id B, container still runs id A) would never
    // be flagged. Re-resolving the tag here catches exactly that.
    if let (Some(running), Some(resolved)) = (&p.running_image_id, &p.resolved_image_id) {
        let drift_ok = running == resolved;
        out.push(CheckResult::warning(
            format!("{}:image-drift", p.ws),
            drift_ok,
            if drift_ok {
                "running image matches the configured tag".into()
            } else {
                format!(
                    "container runs {running}, but configured image '{}' now resolves to \
                     {resolved} — the tag was rebuilt/repointed; run `work harden {}` to recreate",
                    p.configured_image, p.ws
                )
            },
        ));
    }
    out
}

/// Collect the checks that govern whether this workspace may be attached to.
///
/// This is deliberately shared by `work doctor` and the hot attach path: a
/// reported warning must not become an attach-only refusal, and an isolation
/// boundary must not be missed during a normal attach.
pub fn workspace_checks(
    engine: &dyn Engine,
    cfg: &config::WorkspaceConfig,
) -> Result<(Vec<CheckResult>, IsolationProbe)> {
    let ctr = naming::container(&cfg.name);
    let networks = engine.container_networks(&ctr)?;
    let mounts = engine.container_mounts(&ctr)?;
    let probe = IsolationProbe {
        ws: cfg.name.clone(),
        networks: networks.clone(),
        mounts: mounts.clone(),
    };
    let mut results = vec![analyze_isolation(&cfg.name, &networks, &mounts)];

    let restart = engine
        .inspect_format(&ctr, "{{.HostConfig.RestartPolicy.Name}}")
        .unwrap_or_default();
    let user = engine.inspect_format(&ctr, "{{.Config.User}}").map_err(|e| {
        anyhow::anyhow!(
            "could not inspect the workspace user for '{}'; refusing attach because the root-user boundary is unknown: {e}",
            cfg.name
        )
    })?;
    let image = engine
        .inspect_format(&ctr, "{{.Config.Image}}")
        .unwrap_or_default();
    let ports = engine.inspect_format(&ctr, "{{json .NetworkSettings.Ports}}").map_err(|e| {
        anyhow::anyhow!(
            "could not inspect published ports for '{}'; refusing attach because the host-port boundary is unknown: {e}",
            cfg.name
        )
    })?;
    let running_image_id = engine
        .inspect_format(&ctr, "{{.Image}}")
        .ok()
        .filter(|s| !s.is_empty());
    let resolved_image_id = engine.image_id(&cfg.image).ok().filter(|s| !s.is_empty());
    let label_fmt = format!("{{{{index .Config.Labels \"{}\"}}}}", naming::LABEL_KEY);
    let managed_label = engine
        .inspect_format(&ctr, &label_fmt)
        .unwrap_or_default()
        .trim()
        == "true";
    results.extend(analyze_hardening(&HardeningProbe {
        ws: cfg.name.clone(),
        restart_policy: restart,
        user,
        image,
        configured_image: cfg.image.clone(),
        ports_json: ports,
        running_image_id,
        resolved_image_id,
        managed_label,
    }));
    Ok((results, probe))
}

/// Run the full doctor: engine sanity + per-workspace isolation + hardening.
pub fn run(engine: &dyn Engine) -> Result<Vec<CheckResult>> {
    let mut results = Vec::new();

    let running = engine.is_running()?;
    results.push(CheckResult::blocking(
        "engine",
        running,
        format!(
            "{} ({}) {}",
            engine.kind().as_str(),
            engine.binary(),
            if running { "running" } else { "NOT running" }
        ),
    ));

    let names = config::list_workspace_names()?;
    let mut probes = Vec::new();
    for name in &names {
        let ctr = naming::container(name);
        match engine.container_state(&ctr)? {
            ContainerState::Missing => {
                results.push(CheckResult::blocking(
                    name,
                    false,
                    "container missing (run `work start <ws>`)",
                ));
            }
            state => {
                // An unreadable config is a blocking finding, not a silent skip:
                // otherwise we cannot determine the workspace's boundary facts.
                let cfg = match config::load_workspace(name) {
                    Ok(c) => c,
                    Err(e) => {
                        results.push(CheckResult::blocking(
                            format!("{name}:config"),
                            false,
                            format!("config unreadable, workspace checks skipped: {e}"),
                        ));
                        continue;
                    }
                };
                match workspace_checks(engine, &cfg) {
                    Ok((mut checks, probe)) => {
                        if let Some(isolation) = checks.first_mut() {
                            isolation.detail = format!(
                                "[{}] {}",
                                match state {
                                    ContainerState::Running => "running",
                                    _ => "stopped",
                                },
                                isolation.detail
                            );
                        }
                        probes.push(probe);
                        results.extend(checks);
                    }
                    Err(e) => results.push(CheckResult::blocking(
                        name,
                        false,
                        format!("could not inspect isolation boundary: {e}"),
                    )),
                }
            }
        }
    }

    // Forwarder containers (work fwd / work browse) share a workspace network
    // but aren't workspaces. Surface them so an orphaned bridge — e.g. a parent
    // `work browse` kill -9'd mid-loop, leaving a still-running `--rm` container
    // the daemon won't auto-remove — is visible instead of invisible.
    for ctr in engine.list_containers().unwrap_or_default() {
        if ctr.starts_with("work-fwd-") || ctr.starts_with("work-browse-") {
            let managed = engine
                .object_has_label(&ctr, "container", naming::LABEL_KEY)
                .unwrap_or(false);
            results.push(CheckResult::warning(
                format!("forwarder:{ctr}"),
                managed,
                if managed {
                    "managed forwarder running (stop its `work fwd`/`work browse` to clear)".into()
                } else {
                    format!(
                        "unmanaged forwarder — likely an orphan; remove with `{} rm -f {ctr}`",
                        engine.binary()
                    )
                },
            ));
        }
    }
    results.extend(analyze_cross_volume(&probes));
    Ok(results)
}

/// True iff no failed check represents an isolation boundary violation.
pub fn all_ok(results: &[CheckResult]) -> bool {
    results.iter().all(|r| !r.blocks_attach())
}
