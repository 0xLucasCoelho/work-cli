use work_core::engine::{
    is_wsl_kernel, parse_engine_override, pick_kind, select_kind, unsupported_platform_message,
    EngineKind,
};

#[test]
fn prefers_podman_then_docker_then_compatibility_fallbacks() {
    assert_eq!(pick_kind(true, true, true, true), Some(EngineKind::Podman));
    assert_eq!(pick_kind(false, true, true, true), Some(EngineKind::Podman));
    assert_eq!(
        pick_kind(true, true, false, true),
        Some(EngineKind::OrbStack)
    );
    assert_eq!(
        pick_kind(false, true, false, true),
        Some(EngineKind::Colima)
    );
    assert_eq!(
        pick_kind(false, true, false, false),
        Some(EngineKind::Docker)
    );
    assert_eq!(
        pick_kind(false, false, false, true),
        Some(EngineKind::Colima)
    );
    assert_eq!(pick_kind(false, false, false, false), None);
}

#[test]
fn explicit_override_wins_but_requires_available_engine() {
    assert_eq!(
        select_kind(Some(EngineKind::Docker), true, true, true, true).unwrap(),
        EngineKind::Docker
    );
    assert_eq!(
        select_kind(Some(EngineKind::OrbStack), true, false, true, true).unwrap(),
        EngineKind::OrbStack
    );

    let error = select_kind(Some(EngineKind::Colima), false, true, true, false)
        .unwrap_err()
        .to_string();
    assert!(error.contains("WORK_ENGINE=colima"));
    assert!(error.contains("not available"));
}

#[test]
fn parses_supported_work_engine_values_and_rejects_typos() {
    assert_eq!(parse_engine_override(None).unwrap(), None);
    assert_eq!(
        parse_engine_override(Some("  PoDmAn ")).unwrap(),
        Some(EngineKind::Podman)
    );
    assert_eq!(parse_engine_override(Some("")).unwrap(), None);

    let error = parse_engine_override(Some("containerd"))
        .unwrap_err()
        .to_string();
    assert!(error.contains("WORK_ENGINE"));
    assert!(error.contains("podman, docker, orbstack, colima"));
}

#[test]
fn identifies_wsl_kernel_markers_without_host_io() {
    assert!(is_wsl_kernel(
        "5.15.153.1-microsoft-standard-WSL2",
        "Linux version 5.15.153.1-microsoft-standard-WSL2"
    ));
    assert!(is_wsl_kernel(
        "5.15.90.1-generic",
        "Linux version 5.15.90.1-Microsoft-standard"
    ));
    assert!(!is_wsl_kernel(
        "6.8.0-31-generic",
        "Linux version 6.8.0-31-generic"
    ));
}

#[test]
fn native_windows_reports_wsl_only_support() {
    assert!(unsupported_platform_message("linux").is_none());
    assert!(unsupported_platform_message("macos").is_none());
    assert_eq!(
        unsupported_platform_message("windows"),
        Some("Windows support is WSL-only; run work from inside a WSL distribution.")
    );
}
