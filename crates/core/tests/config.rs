use work_core::config::{GlobalConfig, WorkspaceConfig};
use work_core::workspace::{
    classify_daemon_recovery, DaemonRecoveryResources, DaemonRecoveryState,
};

#[test]
fn workspace_config_round_trips_toml() {
    let cfg = WorkspaceConfig {
        name: "acme".into(),
        legacy_fields: std::collections::BTreeMap::new(),
        image: "work-base:latest".into(),
        git_name: Some("Jane Doe".into()),
        git_email: Some("jane@acme.io".into()),
        shell: None,
        profile: None,
        bundles: Vec::new(),
        import_shell_config: None,
        import_herdr_config: None,
        import_starship_config: None,
        import_dotfiles: None,
        pids_limit: 4096,
        browser_confirmation: work_core::config::BrowserConfirmation::Prompt,
        browser_profile: work_core::config::BrowserProfile::Guest,
        daemon_id: None,
        image_digest: None,
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
fn workspace_preferences_round_trip_and_reject_unsafe_pid_values() {
    use work_core::config::{validate_pids_limit, BrowserConfirmation, BrowserProfile};

    let cfg: WorkspaceConfig = toml::from_str(
        "name = 'acme'\nimage = 'work-base:latest'\npids_limit = 8192\nbrowser_confirmation = 'trusted'\nbrowser_profile = 'default'\ncreated_at = 'now'\n",
    )
    .unwrap();
    assert_eq!(cfg.pids_limit, 8192);
    assert_eq!(cfg.browser_confirmation, BrowserConfirmation::Trusted);
    assert_eq!(cfg.browser_profile, BrowserProfile::Default);
    assert!(validate_pids_limit(0).is_err());
    assert!(validate_pids_limit(63).is_err());
    assert!(validate_pids_limit(64).is_ok());
}

#[test]
fn global_config_defaults_when_empty() {
    let g: GlobalConfig = toml::from_str("").unwrap();
    assert_eq!(g.default_image.as_deref(), Some("work-base:latest"));
    assert_eq!(g.effective_default_image(), "work-base:latest");
    assert_eq!(g.effective_default_profile(), "developer");
}

#[test]
fn developer_profile_resolves_only_the_advertised_tools_and_shells() {
    use work_core::config::{resolve_profile, DEVELOPER_IMAGE};

    let developer = resolve_profile(Some("developer"), Some("fish")).unwrap();
    assert_eq!(developer.name, "developer");
    assert_eq!(developer.shell, "fish");
    assert_eq!(developer.image, DEVELOPER_IMAGE);
    assert_eq!(developer.bundles, vec!["developer-tools"]);

    let err = resolve_profile(Some("minimal"), Some("fish")).unwrap_err();
    assert!(err.to_string().contains("does not support shell 'fish'"));
}

#[test]
fn detect_shell_supports_the_three_declared_shells() {
    use work_core::config::{detect_shell, rc_name};
    let sh = detect_shell();
    assert!(
        sh == "zsh" || sh == "bash" || sh == "fish",
        "resolved shell must be zsh, bash, or fish, got {sh}"
    );
    assert_eq!(rc_name("zsh"), ".zshrc");
    assert_eq!(rc_name("bash"), ".bashrc");
    assert_eq!(rc_name("fish"), ".config/fish/config.fish");
}

#[test]
fn global_config_supports_import_defaults() {
    let g: GlobalConfig = toml::from_str(
        "import_shell_config = '/Users/x/.zshrc'\nimport_herdr_config = '/Users/x/.config/herdr/config.toml'\nimport_starship_config = '/Users/x/.config/starship.toml'\nimport_dotfiles = '/Users/x/dotfiles'\n",
    )
    .unwrap();
    assert_eq!(
        g.import_shell_config.as_deref(),
        Some(std::path::Path::new("/Users/x/.zshrc"))
    );
    assert_eq!(
        g.import_herdr_config.as_deref(),
        Some(std::path::Path::new("/Users/x/.config/herdr/config.toml"))
    );
    assert_eq!(
        g.import_starship_config.as_deref(),
        Some(std::path::Path::new("/Users/x/.config/starship.toml"))
    );
    assert_eq!(
        g.import_dotfiles.as_deref(),
        Some(std::path::Path::new("/Users/x/dotfiles"))
    );

    // import defaults are absent in an empty config.
    let empty: GlobalConfig = toml::from_str("").unwrap();
    assert!(empty.import_shell_config.is_none());
    assert!(empty.import_herdr_config.is_none());
    assert!(empty.import_starship_config.is_none());
    assert!(empty.import_dotfiles.is_none());
}

#[test]
fn daemon_recovery_only_accepts_a_complete_managed_isolated_workspace() {
    assert_eq!(
        classify_daemon_recovery(DaemonRecoveryResources::default()),
        DaemonRecoveryState::Empty
    );

    assert_eq!(
        classify_daemon_recovery(DaemonRecoveryResources {
            container_exists: true,
            volume_exists: true,
            network_exists: true,
            container_managed: true,
            volume_managed: true,
            network_managed: true,
            container_isolated: true,
        }),
        DaemonRecoveryState::CompleteManagedIsolated
    );

    assert_eq!(
        classify_daemon_recovery(DaemonRecoveryResources {
            container_exists: true,
            volume_exists: true,
            network_exists: true,
            container_managed: true,
            volume_managed: false,
            network_managed: true,
            container_isolated: true,
        }),
        DaemonRecoveryState::Conflict
    );
}
