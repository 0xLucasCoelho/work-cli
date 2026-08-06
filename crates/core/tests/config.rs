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

#[test]
fn detect_shell_clamps_to_zsh_or_bash() {
    use work_core::config::{detect_shell, rc_name};
    let sh = detect_shell();
    assert!(
        sh == "zsh" || sh == "bash",
        "resolved shell must be zsh or bash, got {sh}"
    );
    assert_eq!(rc_name("zsh"), ".zshrc");
    assert_eq!(rc_name("bash"), ".bashrc");
    // non-zsh shells map to .bashrc
    assert_eq!(rc_name("fish"), ".bashrc");
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
