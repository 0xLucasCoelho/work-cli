//! Isolation verification. Collection (docker inspect via engine) is separate
//! from analysis (pure), so analysis is unit-testable.

use std::collections::BTreeSet;

use anyhow::Result;

use crate::config;
use crate::engine::{ContainerState, Engine};
use crate::naming;

#[derive(Debug, Clone)]
pub struct CheckResult {
    pub label: String,
    pub ok: bool,
    pub detail: String,
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
    /// `{{.HostConfig.CapDrop}}` — expect to contain "ALL".
    pub cap_drop: String,
    /// `{{json .HostConfig.SecurityOpt}}` — expect "no-new-privileges".
    pub security_opt: String,
    /// `{{.Image}}` (the sha the container actually runs). `None` if unreadable.
    pub running_image_id: Option<String>,
    /// The configured tag's image id RE-RESOLVED at check time
    /// (`docker image inspect --format {{.Id}} <cfg.image>`). Compared against
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
        return CheckResult {
            label: ws.to_string(),
            ok: false,
            detail: format!(
                "networks must be exactly {{{expected_net}}}, found {:?}",
                networks.iter().collect::<Vec<_>>()
            ),
        };
    }

    let mounts_ok = mounts.len() == 1
        && mounts.iter().all(|(src, dst)| {
            // A volume mount's source is the volume name (no leading '/').
            // A host bind mount would start with '/' (or a drive) -> rejected.
            src == &expected_vol && dst == &expected_target && !src.starts_with('/')
        });
    if !mounts_ok {
        return CheckResult {
            label: ws.to_string(),
            ok: false,
            detail: format!(
                "mounts must be exactly {{{expected_vol} -> {expected_target}}}, found {:?}",
                mounts
            ),
        };
    }

    CheckResult {
        label: ws.to_string(),
        ok: true,
        detail: format!("on dedicated network {expected_net}; only {expected_vol} mounted"),
    }
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
                Some((vol, dst)) => CheckResult {
                    label: format!("{}:cross-volume", p.ws),
                    ok: false,
                    detail: format!("mounts foreign workspace volume {vol} at {dst}"),
                },
                None => CheckResult {
                    label: format!("{}:cross-volume", p.ws),
                    ok: true,
                    detail: "no foreign workspace volume mounted".into(),
                },
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

/// Per-workspace hardening: restart policy, non-root user, image matches
/// config, and no published host ports (isolation). PURE.
pub fn analyze_hardening(p: &HardeningProbe) -> Vec<CheckResult> {
    let mut out = Vec::new();

    let restart_ok = p.restart_policy == "unless-stopped";
    out.push(CheckResult {
        label: format!("{}:restart", p.ws),
        ok: restart_ok,
        detail: if restart_ok {
            "restart=unless-stopped".into()
        } else {
            format!(
                "restart policy must be 'unless-stopped', found '{}'",
                p.restart_policy
            )
        },
    });

    let non_root = !matches!(p.user.as_str(), "root" | "0");
    out.push(CheckResult {
        label: format!("{}:user", p.ws),
        ok: non_root,
        detail: if non_root {
            if p.user.is_empty() {
                "non-root (image default user)".into()
            } else {
                format!("runs as '{}'", p.user)
            }
        } else {
            "container must not run as root".into()
        },
    });

    let img_ok = p.image == p.configured_image;
    out.push(CheckResult {
        label: format!("{}:image", p.ws),
        ok: img_ok,
        detail: if img_ok {
            format!("image={}", p.image)
        } else {
            format!(
                "container image '{}' != configured '{}'",
                p.image, p.configured_image
            )
        },
    });

    let nports = published_port_count(&p.ports_json);
    out.push(CheckResult {
        label: format!("{}:ports", p.ws),
        ok: nports == 0,
        detail: if nports == 0 {
            "no host ports published".into()
        } else {
            format!("workspace container publishes {nports} host port(s) — isolation risk")
        },
    });

    let caps_ok = p.cap_drop.contains("ALL");
    out.push(CheckResult {
        label: format!("{}:cap-drop", p.ws),
        ok: caps_ok,
        detail: if caps_ok {
            "cap-drop=ALL".into()
        } else {
            format!("expected cap-drop ALL, found '{}'", p.cap_drop)
        },
    });

    let nnp_ok = p.security_opt.contains("no-new-privileges");
    out.push(CheckResult {
        label: format!("{}:no-new-privileges", p.ws),
        ok: nnp_ok,
        detail: if nnp_ok {
            "no-new-privileges set".into()
        } else {
            format!("expected no-new-privileges, found '{}'", p.security_opt)
        },
    });

    let label_ok = p.managed_label;
    out.push(CheckResult {
        label: format!("{}:managed", p.ws),
        ok: label_ok,
        detail: if label_ok {
            "work-managed label present".into()
        } else {
            "missing work-managed label — recreate it: `work harden <ws>`".into()
        },
    });

    // Image drift: compare the image the container actually runs against the
    // configured tag's CURRENT resolution (re-resolved here, at check time).
    // Comparing against a digest recorded at create time would be tautological —
    // a container pins an image id, not a tag — so a locally-rebuilt
    // work-base:latest (tag now -> id B, container still runs id A) would never
    // be flagged. Re-resolving the tag here catches exactly that.
    if let (Some(running), Some(resolved)) = (&p.running_image_id, &p.resolved_image_id) {
        let drift_ok = running == resolved;
        out.push(CheckResult {
            label: format!("{}:image-drift", p.ws),
            ok: drift_ok,
            detail: if drift_ok {
                "running image matches the configured tag".into()
            } else {
                format!(
                    "container runs {running}, but configured image '{}' now resolves to \
                     {resolved} — the tag was rebuilt/repointed; run `work harden {}` to recreate",
                    p.configured_image, p.ws
                )
            },
        });
    }
    out
}

/// Run the full doctor: engine sanity + per-workspace isolation + hardening.
pub fn run(engine: &dyn Engine) -> Result<Vec<CheckResult>> {
    let mut results = Vec::new();

    let running = engine.is_running()?;
    results.push(CheckResult {
        label: "engine".into(),
        ok: running,
        detail: format!(
            "{} ({}) {}",
            engine.kind().as_str(),
            engine.binary(),
            if running { "running" } else { "NOT running" }
        ),
    });

    let names = config::list_workspace_names()?;
    let mut probes = Vec::new();
    for name in &names {
        let ctr = naming::container(name);
        match engine.container_state(&ctr)? {
            ContainerState::Missing => {
                results.push(CheckResult {
                    label: name.clone(),
                    ok: false,
                    detail: "container missing (run `work start <ws>`)".into(),
                });
            }
            state => {
                let networks = engine.container_networks(&ctr)?;
                let mounts = engine.container_mounts(&ctr)?;
                let mut r = analyze_isolation(name, &networks, &mounts);
                r.detail = format!(
                    "[{}] {}",
                    match state {
                        ContainerState::Running => "running",
                        _ => "stopped",
                    },
                    r.detail
                );
                probes.push(IsolationProbe {
                    ws: name.clone(),
                    networks,
                    mounts,
                });
                results.push(r);

                // Hardening (only meaningful when the container exists). An
                // unreadable config is a FINDING, not a silent skip — otherwise a
                // bad merge would disable every hardening check invisibly.
                let cfg = match config::load_workspace(name) {
                    Ok(c) => c,
                    Err(e) => {
                        results.push(CheckResult {
                            label: format!("{name}:config"),
                            ok: false,
                            detail: format!("config unreadable, hardening checks skipped: {e}"),
                        });
                        continue;
                    }
                };
                let restart = engine
                    .inspect_format(&ctr, "{{.HostConfig.RestartPolicy.Name}}")
                    .unwrap_or_default();
                let user = engine
                    .inspect_format(&ctr, "{{.Config.User}}")
                    .unwrap_or_default();
                let image = engine
                    .inspect_format(&ctr, "{{.Config.Image}}")
                    .unwrap_or_default();
                // A port-inspect failure is a finding, not "0 ports = fine".
                let ports = match engine.inspect_format(&ctr, "{{json .NetworkSettings.Ports}}") {
                    Ok(p) => p,
                    Err(e) => {
                        results.push(CheckResult {
                            label: format!("{name}:ports"),
                            ok: false,
                            detail: format!("could not inspect ports: {e}"),
                        });
                        continue;
                    }
                };
                let cap_drop = engine
                    .inspect_format(&ctr, "{{.HostConfig.CapDrop}}")
                    .unwrap_or_default();
                let security_opt = engine
                    .inspect_format(&ctr, "{{json .HostConfig.SecurityOpt}}")
                    .unwrap_or_default();
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
                    ws: name.clone(),
                    restart_policy: restart,
                    user,
                    image,
                    configured_image: cfg.image,
                    ports_json: ports,
                    cap_drop,
                    security_opt,
                    running_image_id,
                    resolved_image_id,
                    managed_label,
                }));
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
            results.push(CheckResult {
                label: format!("forwarder:{ctr}"),
                ok: managed,
                detail: if managed {
                    "managed forwarder running (stop its `work fwd`/`work browse` to clear)".into()
                } else {
                    format!(
                        "unmanaged forwarder — likely an orphan; remove with `docker rm -f {ctr}`"
                    )
                },
            });
        }
    }
    results.extend(analyze_cross_volume(&probes));
    Ok(results)
}

/// True iff every CheckResult is ok.
pub fn all_ok(results: &[CheckResult]) -> bool {
    results.iter().all(|r| r.ok)
}
