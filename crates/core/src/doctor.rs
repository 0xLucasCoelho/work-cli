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

/// Run the full doctor: engine sanity + per-workspace isolation.
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
                // Annotate with lifecycle state for readability.
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
            }
        }
    }

    results.extend(analyze_cross_volume(&probes));
    Ok(results)
}

/// True iff every CheckResult is ok.
pub fn all_ok(results: &[CheckResult]) -> bool {
    results.iter().all(|r| r.ok)
}
