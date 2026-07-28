use work_core::engine::{pick_kind, EngineKind};

#[test]
fn prefers_orbstack_then_docker_then_podman_then_colima() {
    assert_eq!(
        pick_kind(true, true, true, true),
        Some(EngineKind::OrbStack)
    );
    assert_eq!(pick_kind(false, true, true, true), Some(EngineKind::Docker));
    assert_eq!(
        pick_kind(false, false, true, true),
        Some(EngineKind::Podman)
    );
    assert_eq!(
        pick_kind(false, false, false, true),
        Some(EngineKind::Colima)
    );
    assert_eq!(pick_kind(false, false, false, false), None);
}
