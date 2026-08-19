use std::collections::BTreeSet;

use work_core::doctor::{
    all_ok, analyze_cross_volume, analyze_isolation, CheckSeverity, IsolationProbe,
};

#[test]
fn clean_workspace_passes() {
    let mut nets = BTreeSet::new();
    nets.insert("work-net-acme".to_string());
    let mounts = vec![("work-acme-home".to_string(), "/home/dev".to_string())];
    let r = analyze_isolation("acme", &nets, &mounts);
    assert!(r.ok, "{}", r.detail);
}

#[test]
fn extra_network_fails() {
    let mut nets = BTreeSet::new();
    nets.insert("work-net-acme".to_string());
    nets.insert("bridge".to_string());
    let mounts = vec![("work-acme-home".to_string(), "/home/dev".to_string())];
    let r = analyze_isolation("acme", &nets, &mounts);
    assert!(!r.ok);
}

#[test]
fn wrong_volume_mounted_fails() {
    let mut nets = BTreeSet::new();
    nets.insert("work-net-acme".to_string());
    // another workspace's volume mounted here -> breach
    let mounts = vec![("work-other-home".to_string(), "/home/dev".to_string())];
    let r = analyze_isolation("acme", &nets, &mounts);
    assert!(!r.ok);
}

#[test]
fn host_bind_mount_fails() {
    let mut nets = BTreeSet::new();
    nets.insert("work-net-acme".to_string());
    // bind mount (Type != volume shows up as a host path) at /home/dev
    let mounts = vec![("/Users/x/code".to_string(), "/home/dev".to_string())];
    let r = analyze_isolation("acme", &nets, &mounts);
    assert!(!r.ok);
}

#[test]
fn cross_volume_detects_any_overlap() {
    // acme is clean; other mounts acme's volume -> breach
    let probe_a = IsolationProbe {
        ws: "acme".into(),
        networks: BTreeSet::from(["work-net-acme".into()]),
        mounts: vec![("work-acme-home".into(), "/home/dev".into())],
    };
    let probe_b = IsolationProbe {
        ws: "other".into(),
        networks: BTreeSet::from(["work-net-other".into()]),
        mounts: vec![("work-acme-home".into(), "/home/dev".into())],
    };
    let results = analyze_cross_volume(&[probe_a, probe_b]);
    assert!(results.iter().any(|r| !r.ok));
}

use work_core::doctor::{analyze_hardening, HardeningProbe};

fn hp(
    ws: &str,
    restart: &str,
    user: &str,
    image: &str,
    cfg_image: &str,
    ports: &str,
) -> HardeningProbe {
    HardeningProbe {
        ws: ws.into(),
        restart_policy: restart.into(),
        user: user.into(),
        image: image.into(),
        configured_image: cfg_image.into(),
        ports_json: ports.into(),
        // Hardening defaults that PASS — individual tests mutate the field
        // under test.
        running_image_id: None,
        resolved_image_id: None,
        managed_label: true,
    }
}

#[test]
fn hardening_passes_for_clean_workspace() {
    let p = hp(
        "acme",
        "unless-stopped",
        "dev",
        "work-base:latest",
        "work-base:latest",
        "{}",
    );
    let rs = analyze_hardening(&p);
    assert!(rs.iter().all(|r| r.ok), "{:?}", rs);
}

#[test]
fn hardening_flags_wrong_restart_policy() {
    let p = hp(
        "acme",
        "always",
        "dev",
        "work-base:latest",
        "work-base:latest",
        "{}",
    );
    let results = analyze_hardening(&p);
    let restart = results.iter().find(|r| r.label == "acme:restart").unwrap();
    assert!(!restart.ok);
    assert_eq!(restart.severity, CheckSeverity::Warning);
    assert!(all_ok(&results));
}

#[test]
fn hardening_flags_root_user() {
    let p = hp(
        "acme",
        "unless-stopped",
        "root",
        "work-base:latest",
        "work-base:latest",
        "{}",
    );
    let results = analyze_hardening(&p);
    let user = results.iter().find(|r| r.label == "acme:user").unwrap();
    assert!(!user.ok);
    assert_eq!(user.severity, CheckSeverity::Blocking);
    assert!(!all_ok(&results));
}

#[test]
fn hardening_flags_image_mismatch() {
    let p = hp(
        "acme",
        "unless-stopped",
        "dev",
        "node:20",
        "work-base:latest",
        "{}",
    );
    let results = analyze_hardening(&p);
    let image = results.iter().find(|r| r.label == "acme:image").unwrap();
    assert!(!image.ok);
    assert_eq!(image.severity, CheckSeverity::Warning);
    assert!(all_ok(&results));
}

#[test]
fn hardening_accepts_podman_localhost_image_prefix() {
    let p = hp(
        "acme",
        "unless-stopped",
        "dev",
        "localhost/work-base:latest",
        "work-base:latest",
        "{}",
    );
    assert!(
        analyze_hardening(&p)
            .iter()
            .find(|r| r.label == "acme:image")
            .unwrap()
            .ok
    );
}

#[test]
fn hardening_flags_published_ports() {
    let ports = r#"{"8080/tcp":[{"HostIp":"127.0.0.1","HostPort":"8080"}]}"#;
    let p = hp(
        "acme",
        "unless-stopped",
        "dev",
        "work-base:latest",
        "work-base:latest",
        ports,
    );
    let results = analyze_hardening(&p);
    let ports = results.iter().find(|r| r.label == "acme:ports").unwrap();
    assert!(!ports.ok);
    assert_eq!(ports.severity, CheckSeverity::Blocking);
    assert!(!all_ok(&results));
}

#[test]
fn hardening_flags_unmanaged_container() {
    let mut p = hp(
        "acme",
        "unless-stopped",
        "dev",
        "work-base:latest",
        "work-base:latest",
        "{}",
    );
    p.managed_label = false;
    let results = analyze_hardening(&p);
    let managed = results.iter().find(|r| r.label == "acme:managed").unwrap();
    assert!(!managed.ok);
    assert_eq!(managed.severity, CheckSeverity::Warning);
    assert!(all_ok(&results));
}

#[test]
fn hardening_flags_image_digest_drift() {
    let mut p = hp(
        "acme",
        "unless-stopped",
        "dev",
        "work-base:latest",
        "work-base:latest",
        "{}",
    );
    p.running_image_id = Some("sha256:aaaa".into());
    p.resolved_image_id = Some("sha256:bbbb".into());
    let results = analyze_hardening(&p);
    let drift = results
        .iter()
        .find(|r| r.label == "acme:image-drift")
        .unwrap();
    assert!(!drift.ok);
    assert_eq!(drift.severity, CheckSeverity::Warning);
    assert!(all_ok(&results));
}

#[test]
fn hardening_digest_ok_when_matching() {
    let mut p = hp(
        "acme",
        "unless-stopped",
        "dev",
        "work-base:latest",
        "work-base:latest",
        "{}",
    );
    p.running_image_id = Some("sha256:same".into());
    p.resolved_image_id = Some("sha256:same".into());
    assert!(
        analyze_hardening(&p)
            .iter()
            .find(|r| r.label == "acme:image-drift")
            .unwrap()
            .ok
    );
}
