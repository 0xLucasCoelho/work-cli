use std::collections::BTreeSet;

use work_core::doctor::{analyze_cross_volume, analyze_isolation, IsolationProbe};

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
