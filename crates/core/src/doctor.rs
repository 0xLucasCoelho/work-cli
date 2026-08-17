//! Isolation verification. Collection is IO; analysis is PURE.

use std::collections::BTreeSet;

use anyhow::Result;

use crate::config;
use crate::engine::{ContainerState, Engine};
use crate::isolation;
use crate::naming;

#[derive(Debug, Clone)]
pub struct CheckResult {
    pub label: String,
    pub ok: bool,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct IsolationProbe {
    pub ws: String,
    pub networks: BTreeSet<String>,
    pub mounts: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct HardeningProbe {
    pub ws: String,
    pub restart_policy: String,
    pub user: String,
    pub image: String,
    pub configured_image: String,
    pub ports_json: String,
    pub cap_drop: String,
    pub security_opt: String,
    pub managed_label: bool,
}

/// A workspace is isolated iff its only network is its own and its only mount
/// is its own named volume at `/home/dev` (not a host bind).
pub fn analyze_isolation(
    ws: &str,
    networks: &BTreeSet<String>,
    mounts: &[(String, String)],
) -> CheckResult {
    let expected_net = naming::network(ws);
    let expected_vol = naming::volume(ws);
    let expected_target = isolation::HOME.to_string();

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
            src == &expected_vol && dst == &expected_target && !isolation::is_host_bind(src)
        });
    if !mounts_ok {
        return CheckResult {
            label: ws.to_string(),
            ok: false,
            detail: format!(
                "mounts must be exactly {{{expected_vol} -> {expected_target}}}, found {mounts:?}"
            ),
        };
    }

    CheckResult {
        label: ws.to_string(),
        ok: true,
        detail: format!("on dedicated network {expected_net}; only {expected_vol} mounted"),
    }
}

/// Across listed workspaces: no container may mount another workspace's volume.
pub fn analyze_cross_volume(probes: &[IsolationProbe]) -> Vec<CheckResult> {
    let own_volumes: BTreeSet<String> = probes.iter().map(|p| naming::volume(&p.ws)).collect();
    probes
        .iter()
        .map(|p| {
            let expected_vol = naming::volume(&p.ws);
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

/// A non-workspace container's nets + mounts, for the daemon-join check.
pub type ForeignContainer = (String, BTreeSet<String>, Vec<(String, String)>);

/// Any daemon container (not just work-listed) mounting a work volume or joining
/// a work net that is not its own company box.
pub fn analyze_daemon_joins(work_names: &[String], foreign: &[ForeignContainer]) -> Vec<CheckResult> {
    let work_vols: BTreeSet<String> = work_names.iter().map(|n| naming::volume(n)).collect();
    let work_nets: BTreeSet<String> = work_names.iter().map(|n| naming::network(n)).collect();
    let work_ctrs: BTreeSet<String> = work_names.iter().map(|n| naming::container(n)).collect();

    let mut out = Vec::new();
    for (cname, nets, mounts) in foreign {
        if work_ctrs.contains(cname) {
            continue;
        }
        for (src, dst) in mounts {
            if work_vols.contains(src) {
                out.push(CheckResult {
                    label: format!("daemon:{cname}:volume"),
                    ok: false,
                    detail: format!("non-workspace container mounts {src} at {dst}"),
                });
            }
        }
        for n in nets {
            if work_nets.contains(n) {
                out.push(CheckResult {
                    label: format!("daemon:{cname}:net"),
                    ok: false,
                    detail: format!("non-workspace container joined {n}"),
                });
            }
        }
    }
    if out.is_empty() {
        out.push(CheckResult {
            label: "daemon:joins".into(),
            ok: true,
            detail: "no foreign container mounts a work volume or joins a work net".into(),
        });
    }
    out
}

/// Podman prefixes `localhost/` / `docker.io/` on images the user tagged without a registry.
fn images_match(running: &str, configured: &str) -> bool {
    fn strip(s: &str) -> &str {
        s.strip_prefix("localhost/")
            .or_else(|| s.strip_prefix("docker.io/library/"))
            .or_else(|| s.strip_prefix("docker.io/"))
            .unwrap_or(s)
    }
    running == configured || strip(running) == strip(configured)
}

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

pub fn analyze_hardening(p: &HardeningProbe) -> Vec<CheckResult> {
    let mut out = Vec::new();

    let restart_ok = p.restart_policy == "unless-stopped";
    out.push(CheckResult {
        label: format!("{}:restart", p.ws),
        ok: restart_ok,
        detail: if restart_ok {
            "restart=unless-stopped".into()
        } else {
            format!("restart policy must be 'unless-stopped', found '{}'", p.restart_policy)
        },
    });

    let non_root = !matches!(p.user.as_str(), "root" | "0");
    out.push(CheckResult {
        label: format!("{}:user", p.ws),
        ok: non_root,
        detail: if non_root {
            format!("runs as '{}'", if p.user.is_empty() { "image default" } else { &p.user })
        } else {
            "container must not run as root".into()
        },
    });

    let img_ok = images_match(&p.image, &p.configured_image);
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
            format!("workspace container publishes {nports} host port(s)")
        },
    });

    out.push(CheckResult {
        label: format!("{}:managed", p.ws),
        ok: p.managed_label,
        detail: if p.managed_label {
            "work-managed label present".into()
        } else {
            "missing work-managed label".into()
        },
    });

    let cap_ok = p.cap_drop.to_ascii_lowercase().contains("all");
    out.push(CheckResult {
        label: format!("{}:cap-drop", p.ws),
        ok: cap_ok,
        detail: if cap_ok {
            "cap-drop ALL".into()
        } else {
            format!("cap-drop should include ALL, found '{}'", p.cap_drop)
        },
    });

    let nnp = p
        .security_opt
        .to_ascii_lowercase()
        .contains("no-new-privileges");
    out.push(CheckResult {
        label: format!("{}:no-new-privileges", p.ws),
        ok: nnp,
        detail: if nnp {
            "no-new-privileges".into()
        } else {
            format!("missing no-new-privileges (found '{}')", p.security_opt)
        },
    });

    out
}

pub fn run(engine: &dyn Engine) -> Result<Vec<CheckResult>> {
    let mut results = Vec::new();
    let names = config::list_workspace_names()?;
    let mut probes = Vec::new();

    if !engine.is_running()? {
        results.push(CheckResult {
            label: "engine".into(),
            ok: false,
            detail: format!("engine '{}' is not running", engine.binary()),
        });
        return Ok(results);
    }
    results.push(CheckResult {
        label: "engine".into(),
        ok: true,
        detail: format!(
            "{} ({}){}",
            engine.kind().as_str(),
            engine.binary(),
            if engine.kind().is_rootless_default() {
                " · rootless-default"
            } else {
                " · rootful — any docker-group user can read every volume"
            }
        ),
    });

    for name in &names {
        let ctr = naming::container(name);
        let state = engine.container_state(&ctr).unwrap_or(ContainerState::Missing);
        if state == ContainerState::Missing {
            results.push(CheckResult {
                label: name.clone(),
                ok: false,
                detail: "container missing".into(),
            });
            continue;
        }
        let nets = engine.container_networks(&ctr).unwrap_or_default();
        let mounts = engine.container_mounts(&ctr).unwrap_or_default();
        results.push(analyze_isolation(name, &nets, &mounts));
        probes.push(IsolationProbe {
            ws: name.clone(),
            networks: nets,
            mounts,
        });

        if let Ok(cfg) = config::load_workspace(name) {
            let probe = HardeningProbe {
                ws: name.clone(),
                restart_policy: engine
                    .inspect_format(&ctr, "{{.HostConfig.RestartPolicy.Name}}")
                    .unwrap_or_default()
                    .trim()
                    .to_string(),
                user: engine
                    .inspect_format(&ctr, "{{.Config.User}}")
                    .unwrap_or_default()
                    .trim()
                    .to_string(),
                image: engine
                    .inspect_format(&ctr, "{{.Config.Image}}")
                    .unwrap_or_default()
                    .trim()
                    .to_string(),
                configured_image: cfg.image,
                ports_json: engine
                    .inspect_format(&ctr, "{{json .NetworkSettings.Ports}}")
                    .unwrap_or_else(|_| "{}".into()),
                cap_drop: engine
                    .inspect_format(&ctr, "{{json .HostConfig.CapDrop}}")
                    .unwrap_or_default(),
                security_opt: engine
                    .inspect_format(&ctr, "{{json .HostConfig.SecurityOpt}}")
                    .unwrap_or_default(),
                managed_label: engine
                    .object_has_label(&ctr, "container", naming::LABEL_KEY)
                    .unwrap_or(false),
            };
            results.extend(analyze_hardening(&probe));
        }
    }

    results.extend(analyze_cross_volume(&probes));

    let mut foreign = Vec::new();
    if let Ok(all) = engine.list_containers() {
        for cname in all {
            if names.iter().any(|n| naming::container(n) == cname) {
                continue;
            }
            let nets = engine.container_networks(&cname).unwrap_or_default();
            let mounts = engine.container_mounts(&cname).unwrap_or_default();
            foreign.push((cname, nets, mounts));
        }
    }
    results.extend(analyze_daemon_joins(&names, &foreign));
    Ok(results)
}

pub fn all_ok(results: &[CheckResult]) -> bool {
    results.iter().all(|r| r.ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolation_ok_for_canonical_tuple() {
        let mut nets = BTreeSet::new();
        nets.insert("work-net-acme".into());
        let mounts = vec![("work-acme-home".into(), "/home/dev".into())];
        assert!(analyze_isolation("acme", &nets, &mounts).ok);
    }

    #[test]
    fn isolation_rejects_host_bind() {
        let mut nets = BTreeSet::new();
        nets.insert("work-net-acme".into());
        let mounts = vec![("/home/you".into(), "/home/dev".into())];
        assert!(!analyze_isolation("acme", &nets, &mounts).ok);
    }

    #[test]
    fn isolation_rejects_extra_network() {
        let mut nets = BTreeSet::new();
        nets.insert("work-net-acme".into());
        nets.insert("bridge".into());
        let mounts = vec![("work-acme-home".into(), "/home/dev".into())];
        assert!(!analyze_isolation("acme", &nets, &mounts).ok);
    }

    #[test]
    fn cross_volume_detects_breach() {
        let a = IsolationProbe {
            ws: "acme".into(),
            networks: BTreeSet::new(),
            mounts: vec![("work-acme-home".into(), "/home/dev".into())],
        };
        let b = IsolationProbe {
            ws: "globex".into(),
            networks: BTreeSet::new(),
            mounts: vec![
                ("work-globex-home".into(), "/home/dev".into()),
                ("work-acme-home".into(), "/mnt/stolen".into()),
            ],
        };
        let r = analyze_cross_volume(&[a, b]);
        assert!(r[0].ok);
        assert!(!r[1].ok);
    }

    #[test]
    fn daemon_join_flags_foreign_container() {
        let names = vec!["acme".into()];
        let mut nets = BTreeSet::new();
        nets.insert("work-net-acme".into());
        let foreign = vec![(
            "evil".into(),
            nets,
            vec![("work-acme-home".into(), "/mnt".into())],
        )];
        let r = analyze_daemon_joins(&names, &foreign);
        assert!(r.iter().any(|c| !c.ok && c.label.contains("volume")));
        assert!(r.iter().any(|c| !c.ok && c.label.contains("net")));
    }

    #[test]
    fn hardening_requires_cap_drop_and_nnp() {
        let p = HardeningProbe {
            ws: "acme".into(),
            restart_policy: "unless-stopped".into(),
            user: "dev".into(),
            image: "work-base:latest".into(),
            configured_image: "work-base:latest".into(),
            ports_json: "{}".into(),
            cap_drop: "[\"ALL\"]".into(),
            security_opt: "[\"no-new-privileges:true\"]".into(),
            managed_label: true,
        };
        assert!(analyze_hardening(&p).iter().all(|c| c.ok));

        let mut bad = p.clone();
        bad.cap_drop = "[]".into();
        assert!(analyze_hardening(&bad).iter().any(|c| !c.ok && c.label.ends_with("cap-drop")));

        let mut podman = p.clone();
        podman.image = "localhost/work-base:latest".into();
        assert!(
            analyze_hardening(&podman)
                .iter()
                .find(|c| c.label.ends_with(":image"))
                .unwrap()
                .ok
        );
    }
}
