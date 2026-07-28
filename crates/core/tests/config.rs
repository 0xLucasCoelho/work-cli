use work_core::config::{GlobalConfig, WorkspaceConfig};

#[test]
fn workspace_config_round_trips_toml() {
    let cfg = WorkspaceConfig {
        name: "acme".into(),
        image: "work-base:latest".into(),
        git_name: Some("Jane Doe".into()),
        git_email: Some("jane@acme.io".into()),
        shell: None,
        created_at: "2026-07-28T12:00:00Z".into(),
    };
    let s = toml::to_string(&cfg).unwrap();
    let back: WorkspaceConfig = toml::from_str(&s).unwrap();
    assert_eq!(back.name, "acme");
    assert_eq!(back.git_email.as_deref(), Some("jane@acme.io"));
    // shell is None -> must not be serialized (skip_serializing_if)
    assert!(!s.contains("shell"));
}

#[test]
fn global_config_defaults_when_empty() {
    let g: GlobalConfig = toml::from_str("").unwrap();
    assert_eq!(g.default_image.as_deref(), Some("work-base:latest"));
    assert_eq!(g.effective_default_image(), "work-base:latest");
}
