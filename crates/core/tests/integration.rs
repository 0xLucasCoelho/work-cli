//! Integration test against a LIVE container engine.
//!
//! Ignored by default (it creates real containers/volumes/networks). Run with:
//!     cargo test -p work-core --test integration -- --ignored
//!
//! Requires a running engine (OrbStack / Docker / Podman / Colima).

use std::{ffi::OsString, process::Command};

use work_core::{
    config, doctor,
    engine::{self, ContainerState},
    naming,
    workspace::Workspace,
};

/// Unique-ish workspace name so parallel runs don't collide.
fn it_name() -> String {
    format!("it-{}", std::process::id())
}

/// Keep this test from inspecting or modifying the user's real workspace
/// catalog, and guarantee engine/config cleanup if an assertion panics.
struct TestScope {
    name: String,
    previous_config_home: Option<OsString>,
    config_home: tempfile::TempDir,
}

impl TestScope {
    fn new(name: String) -> Self {
        let config_home = tempfile::tempdir().expect("create isolated config directory");
        let previous_config_home = std::env::var_os("XDG_CONFIG_HOME");
        std::env::set_var("XDG_CONFIG_HOME", config_home.path());
        cleanup(&name);
        Self {
            name,
            previous_config_home,
            config_home,
        }
    }
}

impl Drop for TestScope {
    fn drop(&mut self) {
        cleanup(&self.name);
        match self.previous_config_home.take() {
            Some(path) => std::env::set_var("XDG_CONFIG_HOME", path),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
        // Keep the temporary directory alive until after cleanup has restored
        // the environment; its Drop then removes the isolated config tree.
        let _ = &self.config_home;
    }
}

/// Remove a workspace's docker resources + config (work v1 has no `rm` command).
fn cleanup(name: &str) {
    let bin = engine::detect()
        .map(|e| e.binary().to_string())
        .unwrap_or_else(|_| "docker".into());
    for args in [
        vec!["rm", "-f", &naming::container(name)],
        vec!["volume", "rm", &naming::volume(name)],
        vec!["network", "rm", &naming::network(name)],
    ] {
        let _ = Command::new(&bin).args(&args).status();
    }
    let _ = std::fs::remove_file(config::workspace_config_path(name));
}

#[test]
#[ignore]
fn workspace_create_shell_ready_doctor_then_stop() {
    let name = it_name();
    let _scope = TestScope::new(name.clone());

    // Create the workspace (auto-builds the default base image if needed).
    let ws = Workspace::create(
        &name,
        None,
        Some("It Test".into()),
        Some("it@test.io".into()),
        None,
        None,
        None,
        None,
        false,
    )
    .expect("create workspace");
    assert_eq!(ws.cfg.image, config::DEFAULT_IMAGE);

    // Container is running and reachable as the non-root dev user.
    let ctr = naming::container(&name);
    let engine = engine::detect().unwrap();
    assert_eq!(
        engine.container_state(&ctr).unwrap(),
        ContainerState::Running
    );
    let who = engine.exec_capture(&ctr, &["whoami"]).unwrap();
    assert_eq!(who, "dev");
    let pwd = engine
        .exec_capture(&ctr, &["sh", "-c", "echo $PWD"])
        .unwrap();
    assert_eq!(pwd.trim(), "/home/dev");

    // Doctor must pass — isolation invariants hold.
    let results = doctor::run(&*engine).unwrap();
    assert!(doctor::all_ok(&results), "doctor failures: {results:?}");

    // Stop, then state is Stopped; start brings it back.
    ws.stop().unwrap();
    assert_eq!(
        engine.container_state(&ctr).unwrap(),
        ContainerState::Stopped
    );
    ws.start().unwrap();
    assert_eq!(
        engine.container_state(&ctr).unwrap(),
        ContainerState::Running
    );
}
